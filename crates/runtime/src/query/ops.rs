//! Operator-precedence climbing for query goals.
//!
//! Levels mirror the ISO subset implemented by plg-frontend's parser
//! (the reference): 1100 `;` xfy · 1050 `->` xfy · 1000 `,` xfy ·
//! 900 `\+` fy · 700 comparisons/`is`/unify xfx · 500 additive yfx ·
//! 400 multiplicative yfx · 200 `**` xfx, `^` xfy.

use super::QueryParser;
use crate::cell::Word;
use crate::machine::Machine;

/// Symbolic operators, longest-match-first within each level.
const OPS_700: &[&str] = &[
    "=..", "@=<", "@>=", "=:=", "=\\=", "\\==", "==", "@<", "@>", "=<", ">=", "\\=", "is", "=",
    "<", ">",
];
const OPS_500: &[&str] = &["+", "-", "/\\", "\\/", "xor"];
const OPS_400: &[&str] = &["*", "//", "/", "mod", "rem", "div", "<<", ">>"];

impl QueryParser {
    /// Parse a term at the given maximum priority.
    pub(crate) fn parse_level(&mut self, m: &mut Machine, max: u32) -> Result<Word, String> {
        if max >= 1100 {
            return self.parse_xfy(m, ";", 1100, 1050);
        }
        if max >= 1050 {
            return self.parse_xfy(m, "->", 1050, 1000);
        }
        if max >= 1000 {
            return self.parse_xfy(m, ",", 1000, 999);
        }
        if max >= 900 {
            return self.parse_naf(m);
        }
        self.parse_700(m)
    }

    /// Right-associative binary level: `a OP b OP c` ⇒ `OP(a, OP(b, c))`.
    fn parse_xfy(
        &mut self,
        m: &mut Machine,
        op: &str,
        level: u32,
        below: u32,
    ) -> Result<Word, String> {
        let left = self.parse_level(m, below)?;
        self.skip_ws();
        if self.match_symbol(op) {
            let right = self.parse_level(m, level)?; // right-assoc
            Ok(self.make_binop(m, op, left, right))
        } else {
            Ok(left)
        }
    }

    /// 900 fy: `\+ Goal`.
    fn parse_naf(&mut self, m: &mut Machine) -> Result<Word, String> {
        self.skip_ws();
        if self.match_symbol("\\+") {
            let g = self.parse_naf(m)?; // fy: operand at same level
            let id = m.atoms.intern("\\+");
            let idx = m.heap.len();
            m.heap.push(crate::cell::pack_functor(id, 1));
            m.heap.push(g);
            return Ok(crate::cell::make(crate::cell::TAG_STR, idx as u64));
        }
        self.parse_700(m)
    }

    /// 700 xfx (non-associative): comparisons, unification, `is`.
    fn parse_700(&mut self, m: &mut Machine) -> Result<Word, String> {
        let left = self.parse_500(m)?;
        self.skip_ws();
        if let Some(op) = self.match_any(OPS_700) {
            let right = self.parse_500(m)?;
            return Ok(self.make_binop(m, &op, left, right));
        }
        Ok(left)
    }

    /// 500 yfx (left-associative): additive and bitwise.
    fn parse_500(&mut self, m: &mut Machine) -> Result<Word, String> {
        let mut left = self.parse_400(m)?;
        loop {
            self.skip_ws();
            // `-` here must not swallow a negative-literal primary that
            // actually belongs to an infix context: `1 - 2` is fine, the
            // primary parser only folds `-N` at term start.
            match self.match_any(OPS_500) {
                Some(op) => {
                    let right = self.parse_400(m)?;
                    left = self.make_binop(m, &op, left, right);
                }
                None => return Ok(left),
            }
        }
    }

    /// 400 yfx: multiplicative and shifts.
    fn parse_400(&mut self, m: &mut Machine) -> Result<Word, String> {
        let mut left = self.parse_200(m)?;
        loop {
            self.skip_ws();
            match self.match_any(OPS_400) {
                Some(op) => {
                    let right = self.parse_200(m)?;
                    left = self.make_binop(m, &op, left, right);
                }
                None => return Ok(left),
            }
        }
    }

    /// 200: `**` xfx (no chaining), `^` xfy (right-assoc).
    fn parse_200(&mut self, m: &mut Machine) -> Result<Word, String> {
        let left = self.parse_primary(m)?;
        self.skip_ws();
        if self.match_symbol("**") {
            let right = self.parse_primary(m)?;
            return Ok(self.make_binop(m, "**", left, right));
        }
        if self.match_symbol("^") {
            let right = self.parse_200(m)?;
            return Ok(self.make_binop(m, "^", left, right));
        }
        Ok(left)
    }

    /// Longest-match a symbolic or word operator from `ops`.
    fn match_any(&mut self, ops: &[&str]) -> Option<String> {
        for op in ops {
            if self.match_symbol(op) {
                return Some((*op).to_string());
            }
        }
        None
    }

    /// Match an operator at the cursor. Word operators (`is`, `mod`, …)
    /// require a non-identifier boundary; symbolic operators use
    /// maximal munch (don't match `=` inside `=:=`, or `<` inside `<<`).
    fn match_symbol(&mut self, op: &str) -> bool {
        let chars: Vec<char> = op.chars().collect();
        if self.chars.len() - self.pos < chars.len() {
            return false;
        }
        if self.chars[self.pos..self.pos + chars.len()] != chars[..] {
            return false;
        }
        let next = self.chars.get(self.pos + chars.len()).copied();
        let word = chars[0].is_ascii_alphabetic();
        if word {
            // `is`/`mod`/`xor`… must end at an identifier boundary.
            if next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                return false;
            }
        } else {
            // Maximal munch for symbolic ops: reject if the next char
            // extends to a longer operator (e.g. `=` before `..`,
            // `<` before `<`, `*` before `*`, `/` before `/` or `\`).
            if let Some(n) = next
                && is_symbol_char(n)
                && symbol_would_extend(op, n)
            {
                return false;
            }
        }
        self.pos += chars.len();
        true
    }
}

fn is_symbol_char(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '\\' | '<' | '>' | '=' | ':' | '@' | '^' | '.' | '?' | '&' | '~'
    )
}

/// Would `op` followed by `next` form a longer known operator?
fn symbol_would_extend(op: &str, next: char) -> bool {
    const ALL: &[&str] = &[
        "=..", "@=<", "@>=", "=:=", "=\\=", "\\==", "==", "@<", "@>", "=<", ">=", "\\=", "=", "<",
        ">", "+", "-", "*", "//", "/", "/\\", "\\/", "<<", ">>", "**", "^", "->", ";", "\\+",
    ];
    let candidate: String = format!("{op}{next}");
    ALL.iter().any(|o| o.starts_with(&candidate))
}
