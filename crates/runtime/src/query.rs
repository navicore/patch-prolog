//! Minimal goal-only parser for runtime `--query` strings.
//!
//! Deliberately NOT the full plg-frontend parser (binary size): a query
//! is one goal term, optionally a `,`-conjunction. Supports atoms
//! (plain and quoted), variables, integers, compounds, and lists.
//! Operator goals (`X = Y`, arithmetic) arrive with the builtins that
//! implement them (M3+) — the operator TABLE will then be shared from
//! plg-shared so the two parsers cannot diverge.
//!
//! Terms are built directly on the machine heap; query variables are
//! recorded in `m.query_vars` (first-occurrence order, `_` excluded —
//! the renderer sorts by name, matching v1).

use crate::cell::{self, Word};
use crate::machine::Machine;
use std::collections::HashMap;

pub fn parse_query(m: &mut Machine, src: &str) -> Result<Word, String> {
    let mut p = QueryParser {
        chars: src.chars().collect(),
        pos: 0,
        vars: HashMap::new(),
    };
    let goal = p.parse_conjunction(m)?;
    p.skip_ws();
    // Tolerate a trailing '.' like the v1 query parser.
    if p.peek() == Some('.') {
        p.pos += 1;
        p.skip_ws();
    }
    if p.pos < p.chars.len() {
        return Err(format!("unexpected input at column {}", p.pos + 1));
    }
    Ok(goal)
}

struct QueryParser {
    chars: Vec<char>,
    pos: usize,
    vars: HashMap<String, Word>,
}

impl QueryParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else if c == '%' {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected `{c}` at column {}", self.pos + 1))
        }
    }

    /// goal [, goal]* — right-associated `','(A, B)` compounds, the
    /// same shape the frontend produces for conjunctions.
    fn parse_conjunction(&mut self, m: &mut Machine) -> Result<Word, String> {
        let first = self.parse_term(m)?;
        self.skip_ws();
        if self.peek() == Some(',') {
            self.pos += 1;
            let rest = self.parse_conjunction(m)?;
            let comma = m.atoms.intern(",");
            let idx = m.heap.len();
            m.heap.push(cell::pack_functor(comma, 2));
            m.heap.push(first);
            m.heap.push(rest);
            Ok(cell::make(cell::TAG_STR, idx as u64))
        } else {
            Ok(first)
        }
    }

    fn parse_term(&mut self, m: &mut Machine) -> Result<Word, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of query".to_string()),
            Some('(') => {
                self.pos += 1;
                let t = self.parse_conjunction(m)?;
                self.expect(')')?;
                Ok(t)
            }
            Some('[') => self.parse_list(m),
            Some('\'') => {
                let name = self.read_quoted()?;
                self.parse_atom_or_compound(m, name)
            }
            Some(c) if c.is_ascii_digit() => self.parse_integer(m, false),
            Some('-')
                if self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|c| c.is_ascii_digit()) =>
            {
                self.pos += 1;
                self.parse_integer(m, true)
            }
            Some(c) if c.is_uppercase() || c == '_' => {
                let name = self.read_ident();
                Ok(self.var_word(m, &name))
            }
            Some(c) if c.is_lowercase() => {
                let name = self.read_ident();
                self.parse_atom_or_compound(m, name)
            }
            Some(c) => Err(format!("unexpected `{c}` at column {}", self.pos + 1)),
        }
    }

    fn parse_atom_or_compound(&mut self, m: &mut Machine, name: String) -> Result<Word, String> {
        let id = m.atoms.intern(&name);
        // No whitespace allowed between functor and `(` (ISO).
        if self.peek() == Some('(') {
            self.pos += 1;
            let mut args = vec![self.parse_term(m)?];
            loop {
                self.skip_ws();
                match self.peek() {
                    Some(',') => {
                        self.pos += 1;
                        args.push(self.parse_term(m)?);
                    }
                    Some(')') => {
                        self.pos += 1;
                        break;
                    }
                    _ => return Err(format!("expected `,` or `)` at column {}", self.pos + 1)),
                }
            }
            let idx = m.heap.len();
            m.heap.push(cell::pack_functor(id, args.len() as u32));
            m.heap.extend_from_slice(&args);
            Ok(cell::make(cell::TAG_STR, idx as u64))
        } else {
            Ok(cell::make_atom(id))
        }
    }

    fn parse_list(&mut self, m: &mut Machine) -> Result<Word, String> {
        self.expect('[')?;
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(cell::make_atom(plg_shared::atom::ATOM_NIL));
        }
        let mut elements = vec![self.parse_term(m)?];
        let mut tail = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                    elements.push(self.parse_term(m)?);
                }
                Some('|') => {
                    self.pos += 1;
                    tail = Some(self.parse_term(m)?);
                    self.expect(']')?;
                    break;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(format!(
                        "expected `,`, `|` or `]` at column {}",
                        self.pos + 1
                    ));
                }
            }
        }
        let mut w = tail.unwrap_or(cell::make_atom(plg_shared::atom::ATOM_NIL));
        for e in elements.into_iter().rev() {
            let idx = m.heap.len();
            m.heap.push(e);
            m.heap.push(w);
            w = cell::make(cell::TAG_LST, idx as u64);
        }
        Ok(w)
    }

    fn parse_integer(&mut self, m: &mut Machine, neg: bool) -> Result<Word, String> {
        let _ = m;
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        let n: i64 = digits
            .parse()
            .map_err(|_| format!("invalid integer `{digits}`"))?;
        let n = if neg { -n } else { n };
        if !(cell::INT_MIN..=cell::INT_MAX).contains(&n) {
            return Err(format!("integer `{n}` out of supported range"));
        }
        Ok(cell::make_int(n))
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn read_quoted(&mut self) -> Result<String, String> {
        self.pos += 1; // opening '
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated quoted atom".to_string()),
                Some('\'') => {
                    self.pos += 1;
                    if self.peek() == Some('\'') {
                        out.push('\''); // '' escape
                        self.pos += 1;
                    } else {
                        return Ok(out);
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// `_` is always fresh and never recorded; named variables are
    /// shared within the query and recorded for solution output.
    fn var_word(&mut self, m: &mut Machine, name: &str) -> Word {
        if name == "_" {
            return m.new_var();
        }
        if let Some(&w) = self.vars.get(name) {
            return w;
        }
        let w = m.new_var();
        self.vars.insert(name.to_string(), w);
        m.query_vars
            .push((name.to_string(), cell::payload(w) as usize));
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    #[test]
    fn parses_compound_with_vars() {
        let mut m = machine();
        let w = parse_query(&mut m, "parent(tom, X)").unwrap();
        assert_eq!(tag_of(w), TAG_STR);
        let idx = payload(w) as usize;
        let (f, n) = unpack_functor(m.heap[idx]);
        assert_eq!(m.atoms.resolve(f), "parent");
        assert_eq!(n, 2);
        assert_eq!(m.query_vars.len(), 1);
        assert_eq!(m.query_vars[0].0, "X");
    }

    #[test]
    fn conjunction_shares_variables() {
        let mut m = machine();
        let w = parse_query(&mut m, "p(X), q(X, Y)").unwrap();
        let idx = payload(w) as usize;
        let (f, n) = unpack_functor(m.heap[idx]);
        assert_eq!(m.atoms.resolve(f), ",");
        assert_eq!(n, 2);
        assert_eq!(m.query_vars.len(), 2, "X shared, Y new");
    }

    #[test]
    fn lists_and_quoted_atoms() {
        let mut m = machine();
        let w = parse_query(&mut m, "p([1, 2 | T], 'hello world')").unwrap();
        assert_eq!(tag_of(w), TAG_STR);
        assert!(m.atoms.lookup("hello world").is_some());
        assert_eq!(m.query_vars[0].0, "T");
    }

    #[test]
    fn underscore_never_recorded() {
        let mut m = machine();
        parse_query(&mut m, "p(_, _)").unwrap();
        assert!(m.query_vars.is_empty());
    }

    #[test]
    fn negative_integers_and_trailing_dot() {
        let mut m = machine();
        let w = parse_query(&mut m, "p(-42).").unwrap();
        let idx = payload(w) as usize;
        assert_eq!(int_value(m.deref(m.heap[idx + 1])), -42);
    }

    #[test]
    fn rejects_trailing_garbage() {
        let mut m = machine();
        assert!(parse_query(&mut m, "p(a) q").is_err());
        assert!(parse_query(&mut m, "p(").is_err());
    }
}
