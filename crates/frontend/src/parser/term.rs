//! Term and primary parsing: the precedence-climbing levels (700 → 200),
//! prefix operators, parenthesized control constructs, and list syntax.
//! Ported from patch-prolog's `parser.rs`. Operator-name lookups defer to
//! [`super::operators`].

use super::Parser;
use super::operators;
use crate::parse_error::ParseError;
use crate::tokenizer::TokenKind;
use plg_shared::{Span, Term};

impl Parser<'_> {
    /// Parse a term at the top level (precedence 700 — non-associative comparison/is level).
    pub(super) fn parse_term(&mut self) -> Result<Term, ParseError> {
        self.parse_expr_700()
    }

    /// Precedence 700: non-associative operators (is, =, \=, <, >, =<, >=, =:=, =\=)
    fn parse_expr_700(&mut self) -> Result<Term, ParseError> {
        let left = self.parse_expr_500()?;
        if let Some(op) = self.match_op_700() {
            let right = self.parse_expr_500()?;
            Ok(self.build_binop(&op, left, right))
        } else {
            Ok(left)
        }
    }

    fn match_op_700(&mut self) -> Option<String> {
        let kind = self.current_kind()?;
        // Word operators (`@<`, `=..`, ...) arrive as `Atom` tokens.
        if let TokenKind::Atom(s) = kind {
            if let Some(op) = operators::word_op_700(s) {
                self.advance();
                return Some(op.to_string());
            }
            return None;
        }
        let op = operators::op_700(kind)?;
        self.advance();
        Some(op.to_string())
    }

    /// Precedence 500: left-associative (+, -, /\, \/, xor — all yfx).
    fn parse_expr_500(&mut self) -> Result<Term, ParseError> {
        let mut left = self.parse_expr_400()?;
        while let Some(op) = self.current_kind().and_then(operators::op_500) {
            let op = op.to_string();
            self.advance();
            let right = self.parse_expr_400()?;
            left = self.build_binop(&op, left, right);
        }
        Ok(left)
    }

    /// Precedence 400: left-associative (*, /, //, mod, rem, div, <<, >> — all yfx).
    fn parse_expr_400(&mut self) -> Result<Term, ParseError> {
        let mut left = self.parse_expr_200()?;
        while let Some(op) = self.current_kind().and_then(operators::op_400) {
            let op = op.to_string();
            self.advance();
            let right = self.parse_expr_200()?;
            left = self.build_binop(&op, left, right);
        }
        Ok(left)
    }

    /// Precedence 200: `**` (xfx, non-associative), `^` (xfy, right-assoc),
    /// `:` (xfy, right-assoc). Issue #29.
    fn parse_expr_200(&mut self) -> Result<Term, ParseError> {
        let left = self.parse_primary()?;
        match self.current_kind() {
            // xfx — no chaining: RHS is just a primary, not parse_expr_200.
            Some(TokenKind::Pow) => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(self.build_binop("**", left, right))
            }
            // xfy — right-associative: recurse into parse_expr_200.
            Some(TokenKind::Caret) => {
                self.advance();
                let right = self.parse_expr_200()?;
                Ok(self.build_binop("^", left, right))
            }
            Some(TokenKind::Colon) => {
                self.advance();
                let right = self.parse_expr_200()?;
                Ok(self.build_binop(":", left, right))
            }
            _ => Ok(left),
        }
    }

    fn build_binop(&mut self, op: &str, left: Term, right: Term) -> Term {
        let functor = self.interner.intern(op);
        Term::Compound {
            functor,
            args: vec![left, right],
        }
    }

    /// Issue #19 — recognize operator tokens as bare atoms in term position.
    /// Returns the atom name when the *current* token is one of the listed
    /// operators AND the *next* token is a closing context (`)`, `]`, `,`,
    /// `|`, `.`, EOF). The closing-context check is what keeps `1 + 2` from
    /// being misread (there `+` is preceded by a primary and is at infix
    /// position, not at primary start).
    fn operator_as_atom_lookahead(&self) -> Option<&'static str> {
        let name = operators::op_as_atom(self.current_kind()?)?;
        match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
            Some(TokenKind::RParen)
            | Some(TokenKind::RBracket)
            | Some(TokenKind::Comma)
            | Some(TokenKind::Pipe)
            | Some(TokenKind::Dot)
            | Some(TokenKind::Eof)
            | None => Some(name),
            _ => None,
        }
    }

    fn parse_primary(&mut self) -> Result<Term, ParseError> {
        // Issue #19: an operator token at the start of a primary, immediately
        // followed by a "closing context" token, reads as the atom for that
        // operator. Handles `p(+)`, `[<, >]`, `X = (mod)`, `=..` round-trips,
        // etc., without breaking `1 + 2` (where `+` appears at infix
        // position, not primary).
        if let Some(name) = self.operator_as_atom_lookahead() {
            self.advance();
            let id = self.interner.intern(name);
            return Ok(Term::Atom(id));
        }
        match self.current_kind().cloned() {
            Some(TokenKind::Integer(n)) => {
                self.advance();
                Ok(Term::Integer(n))
            }
            Some(TokenKind::Float(f)) => {
                self.advance();
                Ok(Term::Float(f))
            }
            Some(TokenKind::Variable(ref name)) => {
                let name = name.clone();
                self.advance();
                Ok(self.intern_variable(name))
            }
            Some(TokenKind::Atom(ref name)) => {
                let name = name.clone();
                // Capture the atom token's start/end for the call-site span
                // before advancing past it.
                let (lo, atom_hi) = self.current().map(|t| (t.lo, t.hi)).unwrap_or((0, 0));
                self.advance();
                // Check if followed by '(' — compound term
                // The call-site span underlines just the functor name (not
                // its args), so squiggles land tightly on the predicate name.
                let span = Span::new(0, lo, atom_hi);
                if self.current_kind() == Some(&TokenKind::LParen) {
                    self.advance(); // skip (
                    let args = self.parse_arg_list()?;
                    self.expect(&TokenKind::RParen)?;
                    let functor = self.interner.intern(&name);
                    let arity = args.len();
                    self.record_call_site(functor, arity, span);
                    Ok(Term::Compound { functor, args })
                } else {
                    let id = self.interner.intern(&name);
                    self.record_call_site(id, 0, span);
                    Ok(Term::Atom(id))
                }
            }
            Some(TokenKind::LParen) => {
                self.advance();
                let term = self.parse_paren_body()?;
                self.expect(&TokenKind::RParen)?;
                Ok(term)
            }
            Some(TokenKind::Minus) => {
                self.advance();
                let operand = self.parse_primary()?;
                // Optimize: if operand is a literal number, negate it directly
                match operand {
                    Term::Integer(n) => Ok(Term::Integer(-n)),
                    Term::Float(f) => Ok(Term::Float(-f)),
                    _ => {
                        let functor = self.interner.intern("-");
                        Ok(Term::Compound {
                            functor,
                            args: vec![operand],
                        })
                    }
                }
            }
            // Issue #28: ISO `+` (fy 200) — unary plus, folded for literal numbers.
            Some(TokenKind::Plus) => {
                self.advance();
                let operand = self.parse_primary()?;
                match operand {
                    Term::Integer(_) | Term::Float(_) => Ok(operand),
                    _ => {
                        let functor = self.interner.intern("+");
                        Ok(Term::Compound {
                            functor,
                            args: vec![operand],
                        })
                    }
                }
            }
            // Issue #28: ISO `\` (fy 200) — bitwise complement. No literal
            // folding; the arithmetic evaluator handles `\N` at `is`-time.
            Some(TokenKind::Backslash) => {
                self.advance();
                let operand = self.parse_primary()?;
                let functor = self.interner.intern("\\");
                Ok(Term::Compound {
                    functor,
                    args: vec![operand],
                })
            }
            Some(TokenKind::LBracket) => {
                self.advance(); // skip [
                self.parse_list_body()
            }
            Some(TokenKind::Cut) => {
                self.advance();
                let id = self.interner.intern("!");
                Ok(Term::Atom(id))
            }
            Some(TokenKind::Not) => {
                // \+ Goal — ISO precedence 900fy, parses argument at 700
                self.advance();
                let goal = self.parse_term()?;
                let functor = self.interner.intern("\\+");
                Ok(Term::Compound {
                    functor,
                    args: vec![goal],
                })
            }
            Some(ref tok) => {
                let msg = format!("unexpected {tok}");
                Err(self.error_here(msg))
            }
            None => Err(self.error_here("unexpected end of input")),
        }
    }

    /// Resolve a variable name to a `Term::Var`, allocating a fresh id for
    /// `_` (anonymous) and for each distinct named variable within the clause.
    fn intern_variable(&mut self, name: String) -> Term {
        if name == "_" {
            // Anonymous variable — always fresh
            let id = self.next_var;
            self.next_var += 1;
            Term::Var(id)
        } else if let Some(&id) = self.var_map.get(&name) {
            Term::Var(id)
        } else {
            let id = self.next_var;
            self.next_var += 1;
            self.var_map.insert(name, id);
            Term::Var(id)
        }
    }

    /// Parse the body of a parenthesized expression, handling ; and ->.
    /// Supports: (A ; B), (Cond -> Then), (Cond -> Then ; Else)
    fn parse_paren_body(&mut self) -> Result<Term, ParseError> {
        let first = self.parse_paren_comma_list()?;

        if self.current_kind() == Some(&TokenKind::Arrow) {
            // (Cond -> Then) or (Cond -> Then ; Else)
            self.advance();
            let then = self.parse_paren_comma_list()?;
            let arrow_functor = self.interner.intern("->");
            let if_then = Term::Compound {
                functor: arrow_functor,
                args: vec![first, then],
            };
            if self.current_kind() == Some(&TokenKind::Semicolon) {
                self.advance();
                let else_branch = self.parse_paren_body()?;
                let semi_functor = self.interner.intern(";");
                Ok(Term::Compound {
                    functor: semi_functor,
                    args: vec![if_then, else_branch],
                })
            } else {
                Ok(if_then)
            }
        } else if self.current_kind() == Some(&TokenKind::Semicolon) {
            // (A ; B)
            self.advance();
            let right = self.parse_paren_body()?;
            let functor = self.interner.intern(";");
            Ok(Term::Compound {
                functor,
                args: vec![first, right],
            })
        } else {
            Ok(first)
        }
    }

    /// Parse a comma-separated goal conjunction within parens, building ','(A,B) terms.
    fn parse_paren_comma_list(&mut self) -> Result<Term, ParseError> {
        let first = self.parse_term()?;
        if self.current_kind() == Some(&TokenKind::Comma) {
            // Check that the next comma isn't just the end of an arg list —
            // but inside parens for ; / ->, comma means conjunction
            self.advance();
            let rest = self.parse_paren_comma_list()?;
            let functor = self.interner.intern(",");
            Ok(Term::Compound {
                functor,
                args: vec![first, rest],
            })
        } else {
            Ok(first)
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Term>, ParseError> {
        let mut args = vec![self.parse_term()?];
        while self.current_kind() == Some(&TokenKind::Comma) {
            self.advance();
            args.push(self.parse_term()?);
        }
        Ok(args)
    }

    fn parse_list_body(&mut self) -> Result<Term, ParseError> {
        // We're right after '['. Parse list elements.
        if self.current_kind() == Some(&TokenKind::RBracket) {
            self.advance();
            let nil = self.interner.intern("[]");
            return Ok(Term::Atom(nil));
        }

        let first = self.parse_term()?;
        self.parse_list_tail(first)
    }

    fn parse_list_tail(&mut self, head: Term) -> Result<Term, ParseError> {
        match self.current_kind() {
            Some(TokenKind::Comma) => {
                self.advance();
                let next_head = self.parse_term()?;
                let tail = self.parse_list_tail(next_head)?;
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(tail),
                })
            }
            Some(TokenKind::Pipe) => {
                self.advance();
                let tail = self.parse_term()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(tail),
                })
            }
            Some(TokenKind::RBracket) => {
                self.advance();
                let nil = self.interner.intern("[]");
                Ok(Term::List {
                    head: Box::new(head),
                    tail: Box::new(Term::Atom(nil)),
                })
            }
            _ => Err(self.error_here("Expected ',', '|', or ']' in list")),
        }
    }
}
