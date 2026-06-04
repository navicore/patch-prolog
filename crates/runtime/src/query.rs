//! Minimal goal-only parser for runtime `--query` strings.
//!
//! Deliberately NOT the full plg-frontend parser (binary size): a query
//! is one goal term. Supports atoms (plain and quoted), variables,
//! integers, compounds, lists, and the standard operator set via the
//! precedence climber in `query::ops` — the levels mirror
//! plg-frontend/src/parser (the reference implementation); the
//! differential test corpus guards against drift.
//!
//! Terms are built directly on the machine heap; query variables are
//! recorded in `m.query_vars` (first-occurrence order, `_` excluded —
//! the renderer sorts by name, matching v1).

mod ops;

use crate::cell::{self, Word};
use crate::machine::Machine;
use std::collections::HashMap;

pub fn parse_query(m: &mut Machine, src: &str) -> Result<Word, String> {
    let mut p = QueryParser {
        chars: src.chars().collect(),
        pos: 0,
        vars: HashMap::new(),
    };
    let goal = p.parse_level(m, 1200)?;
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

pub(crate) struct QueryParser {
    pub(crate) chars: Vec<char>,
    pub(crate) pos: usize,
    vars: HashMap<String, Word>,
}

impl QueryParser {
    pub(crate) fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    pub(crate) fn skip_ws(&mut self) {
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

    pub(crate) fn make_binop(&self, m: &mut Machine, name: &str, a: Word, b: Word) -> Word {
        let id = m.atoms.intern(name);
        let idx = m.heap.len();
        m.heap.push(cell::pack_functor(id, 2));
        m.heap.push(a);
        m.heap.push(b);
        cell::make(cell::TAG_STR, idx as u64)
    }

    /// A primary term: constants, variables, compounds, lists, parens.
    pub(crate) fn parse_primary(&mut self, m: &mut Machine) -> Result<Word, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of query".to_string()),
            Some('(') => {
                self.pos += 1;
                let t = self.parse_level(m, 1200)?;
                self.expect(')')?;
                Ok(t)
            }
            Some('[') => self.parse_list(m),
            Some('\'') => {
                let name = self.read_quoted()?;
                self.parse_atom_or_compound(m, name)
            }
            Some(c) if c.is_ascii_digit() => self.parse_number(m, false),
            // Standalone operator atoms (`(-)`, `[+, *]`, `call(=, X, Y)`)
            // and canonical compound forms (`=(X, 7)`): a symbol run
            // followed by `(` is a compound, by a term-end is an atom.
            Some(c) if is_symbol_atom_char(c) && self.symbol_run_is_standalone() => {
                let name = self.read_symbol_run();
                self.parse_atom_or_compound(m, name)
            }
            Some('-')
                if self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|c| c.is_ascii_digit()) =>
            {
                self.pos += 1;
                self.parse_number(m, true)
            }
            Some('-') => {
                // Prefix minus over a non-literal: -(Term).
                self.pos += 1;
                let t = self.parse_primary(m)?;
                self.make_prefix(m, "-", t)
            }
            Some('+') => {
                // Prefix plus: folds into a positive numeric literal
                // (v1 issue #19), otherwise builds '+'/1.
                self.pos += 1;
                self.skip_ws();
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.parse_number(m, false)
                } else {
                    let t = self.parse_primary(m)?;
                    self.make_prefix(m, "+", t)
                }
            }
            Some('\\') => {
                // Prefix `\` (and `\+` reached here only as an atom —
                // the level-900 parser handles real negation).
                let name = self.read_symbol_run();
                if self.peek() == Some('(') {
                    self.parse_atom_or_compound(m, name)
                } else {
                    let t = self.parse_primary(m)?;
                    self.make_prefix(m, &name, t)
                }
            }
            Some('!') => {
                self.pos += 1;
                Ok(cell::make_atom(m.atoms.intern("!")))
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
            // Arguments parse at priority 999 (no bare `,`).
            let mut args = vec![self.parse_level(m, 999)?];
            loop {
                self.skip_ws();
                match self.peek() {
                    Some(',') => {
                        self.pos += 1;
                        args.push(self.parse_level(m, 999)?);
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
        let mut elements = vec![self.parse_level(m, 999)?];
        let mut tail = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                    elements.push(self.parse_level(m, 999)?);
                }
                Some('|') => {
                    self.pos += 1;
                    tail = Some(self.parse_level(m, 999)?);
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

    /// Integer or float literal. A `.` continues a float only when a
    /// digit follows (otherwise it's the goal terminator) — same
    /// disambiguation as the frontend tokenizer.
    fn parse_number(&mut self, m: &mut Machine, neg: bool) -> Result<Word, String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let is_float = self.peek() == Some('.')
            && self
                .chars
                .get(self.pos + 1)
                .is_some_and(|c| c.is_ascii_digit());
        if is_float {
            self.pos += 1; // '.'
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            let text: String = self.chars[start..self.pos].iter().collect();
            let f: f64 = text
                .parse()
                .map_err(|_| format!("invalid float `{text}`"))?;
            let f = if neg { -f } else { f };
            let idx = m.heap.len();
            m.heap.push(f.to_bits());
            return Ok(cell::make(cell::TAG_FLT, idx as u64));
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        let n: i64 = digits
            .parse()
            .map_err(|_| format!("invalid integer `{digits}`"))?;
        let n = if neg { -n } else { n };
        if !(cell::INT_MIN..=cell::INT_MAX).contains(&n) {
            // Box beyond-immediate integers (full i64 range, v1 parity).
            let idx = m.heap.len();
            m.heap.push(n as u64);
            return Ok(cell::make(cell::TAG_BIG, idx as u64));
        }
        Ok(cell::make_int(n))
    }

    pub(crate) fn read_ident(&mut self) -> String {
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

    fn read_symbol_run(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_symbol_atom_char) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// Does the symbol run starting at the cursor stand alone as an
    /// atom (followed by a term-end) or start a canonical compound
    /// (immediately followed by `(`)?
    fn symbol_run_is_standalone(&self) -> bool {
        let mut p = self.pos;
        while self.chars.get(p).copied().is_some_and(is_symbol_atom_char) {
            p += 1;
        }
        if self.chars.get(p) == Some(&'(') {
            return true; // canonical compound: =(X, 7)
        }
        let mut q = p;
        while self.chars.get(q).is_some_and(|c| c.is_whitespace()) {
            q += 1;
        }
        match self.chars.get(q) {
            None => {
                // End of input — but a trailing '.' inside the run is
                // the goal terminator, not part of the atom; only the
                // run itself ending the input counts.
                true
            }
            Some(')' | ',' | ']' | '|') => true,
            _ => false,
        }
    }

    fn make_prefix(&self, m: &mut Machine, name: &str, t: Word) -> Result<Word, String> {
        let id = m.atoms.intern(name);
        let idx = m.heap.len();
        m.heap.push(cell::pack_functor(id, 1));
        m.heap.push(t);
        Ok(cell::make(cell::TAG_STR, idx as u64))
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

/// Characters that form symbolic atoms (Prolog "symbol char" class).
fn is_symbol_atom_char(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '\\' | '<' | '>' | '=' | ':' | '@' | '^' | '.' | '?' | '&' | '~'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn functor_name(m: &Machine, w: Word) -> String {
        let idx = payload(w) as usize;
        let (f, _) = unpack_functor(m.heap[idx]);
        m.atoms.resolve(f).to_string()
    }

    #[test]
    fn parses_compound_with_vars() {
        let mut m = machine();
        let w = parse_query(&mut m, "parent(tom, X)").unwrap();
        assert_eq!(tag_of(w), TAG_STR);
        assert_eq!(functor_name(&m, w), "parent");
        assert_eq!(m.query_vars.len(), 1);
        assert_eq!(m.query_vars[0].0, "X");
    }

    #[test]
    fn conjunction_shares_variables() {
        let mut m = machine();
        let w = parse_query(&mut m, "p(X), q(X, Y)").unwrap();
        assert_eq!(functor_name(&m, w), ",");
        assert_eq!(m.query_vars.len(), 2, "X shared, Y new");
    }

    #[test]
    fn operator_goals_parse() {
        let mut m = machine();
        let w = parse_query(&mut m, "X is 2 + 3 * 4").unwrap();
        assert_eq!(functor_name(&m, w), "is");
        let w = parse_query(&mut m, "1 < 2").unwrap();
        assert_eq!(functor_name(&m, w), "<");
        let w = parse_query(&mut m, "(a ; b)").unwrap();
        assert_eq!(functor_name(&m, w), ";");
        let w = parse_query(&mut m, "(a -> b ; c)").unwrap();
        assert_eq!(functor_name(&m, w), ";");
        let w = parse_query(&mut m, "\\+ p(X)").unwrap();
        assert_eq!(functor_name(&m, w), "\\+");
    }

    #[test]
    fn precedence_multiplication_binds_tighter() {
        let mut m = machine();
        // is(X, +(2, *(3, 4)))
        let w = parse_query(&mut m, "X is 2 + 3 * 4").unwrap();
        let idx = payload(w) as usize;
        let rhs = m.deref(m.heap[idx + 2]);
        assert_eq!(functor_name(&m, rhs), "+");
        let plus_idx = payload(rhs) as usize;
        let right = m.deref(m.heap[plus_idx + 2]);
        assert_eq!(functor_name(&m, right), "*");
    }

    #[test]
    fn args_parse_operators_but_not_bare_comma() {
        let mut m = machine();
        let w = parse_query(&mut m, "p(1 + 2, X)").unwrap();
        let idx = payload(w) as usize;
        let (f, n) = unpack_functor(m.heap[idx]);
        assert_eq!(m.atoms.resolve(f), "p");
        assert_eq!(n, 2, "1 + 2 is one arg, X the other");
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
