//! Clause parsing (`head.` / `head :- body.`) and `:- ...` directive
//! handling (`dynamic/1`). Ported from patch-prolog's `parser.rs`.

use super::{Parser, ProgramDirectives};
use crate::tokenizer::TokenKind;
use plg_shared::{Clause, StringInterner, Term};

impl Parser<'_> {
    pub(super) fn parse_clause(&mut self) -> Result<Clause, String> {
        let head = self.parse_term()?;
        match self.current_kind() {
            Some(TokenKind::Dot) => {
                self.advance();
                Ok(Clause { head, body: vec![] })
            }
            Some(TokenKind::Neck) => {
                self.advance();
                let body = self.parse_goal_list()?;
                self.expect(&TokenKind::Dot)?;
                Ok(Clause { head, body })
            }
            Some(tok) => {
                let tok = tok.clone();
                let pos = self.current().unwrap();
                Err(format!(
                    "expected `.` or `:-`, got {} at line {} col {}",
                    tok, pos.line, pos.col
                ))
            }
            None => Err("Unexpected end of input in clause".to_string()),
        }
    }

    /// Interpret a directive body and update `directives` accordingly.
    pub(super) fn process_directive(
        &self,
        body: Term,
        directives: &mut ProgramDirectives,
    ) -> Result<(), String> {
        match body {
            Term::Compound { functor, args } if self.interner.resolve(functor) == "dynamic" => {
                for arg in args {
                    self.collect_dynamic_specs(arg, directives)?;
                }
                Ok(())
            }
            _ => Err(format!(
                "Unknown directive: {}",
                format_directive_term(&body, self.interner)
            )),
        }
    }

    /// Walk a dynamic spec — either `F/A` or a comma-chain of such — into
    /// the directives set. ISO writes both `:- dynamic(f/1, g/2).` (handled
    /// by the caller iterating compound args) and `:- dynamic((f/1, g/2)).`
    /// (a single comma-chain arg, handled here).
    fn collect_dynamic_specs(
        &self,
        spec: Term,
        directives: &mut ProgramDirectives,
    ) -> Result<(), String> {
        match spec {
            Term::Compound { functor, args }
                if self.interner.resolve(functor) == "," && args.len() == 2 =>
            {
                let mut it = args.into_iter();
                self.collect_dynamic_specs(it.next().unwrap(), directives)?;
                self.collect_dynamic_specs(it.next().unwrap(), directives)
            }
            Term::Compound { functor, args }
                if self.interner.resolve(functor) == "/" && args.len() == 2 =>
            {
                let f = match &args[0] {
                    Term::Atom(id) => *id,
                    other => {
                        return Err(format!(
                            "dynamic spec functor must be an atom, got {}",
                            format_directive_term(other, self.interner)
                        ));
                    }
                };
                let arity = match &args[1] {
                    Term::Integer(n) if *n >= 0 => *n as usize,
                    other => {
                        return Err(format!(
                            "dynamic spec arity must be a non-negative integer, got {}",
                            format_directive_term(other, self.interner)
                        ));
                    }
                };
                directives.dynamic.push((f, arity));
                Ok(())
            }
            other => Err(format!(
                "Invalid dynamic spec (expected F/A): {}",
                format_directive_term(&other, self.interner)
            )),
        }
    }
}

/// Tiny term formatter used only for parser-error messages about malformed
/// directives. Not a general pretty-printer.
fn format_directive_term(term: &Term, interner: &StringInterner) -> String {
    match term {
        Term::Atom(id) => interner.resolve(*id).to_string(),
        Term::Var(id) => format!("_G{id}"),
        Term::Integer(n) => n.to_string(),
        Term::Float(f) => f.to_string(),
        Term::Compound { functor, args } => {
            let parts: Vec<String> = args
                .iter()
                .map(|a| format_directive_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*functor), parts.join(", "))
        }
        Term::List { .. } => format!("{term:?}"),
    }
}
