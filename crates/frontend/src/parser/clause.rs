//! Clause parsing (`head.` / `head :- body.`) and `:- ...` directive
//! handling (`dynamic/1`). Ported from patch-prolog's `parser.rs`.

use super::{Parser, ProgramDirectives};
use crate::parse_error::ParseError;
use crate::tokenizer::TokenKind;
use plg_shared::{Clause, StringInterner, Term};

impl Parser<'_> {
    pub(super) fn parse_clause(&mut self) -> Result<Clause, ParseError> {
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
                let msg = format!("expected `.` or `:-`, got {tok}");
                Err(self.error_here(msg))
            }
            None => Err(self.error_here("Unexpected end of input in clause")),
        }
    }

    /// Interpret a directive body and update `directives` accordingly.
    pub(super) fn process_directive(
        &self,
        body: Term,
        directives: &mut ProgramDirectives,
    ) -> Result<(), ParseError> {
        match body {
            Term::Compound { functor, args } if self.interner.resolve(functor) == "dynamic" => {
                for arg in args {
                    self.collect_dynamic_specs(arg, directives)?;
                }
                Ok(())
            }
            Term::Compound { functor, args }
                if self.interner.resolve(functor) == "io_format" && args.len() == 1 =>
            {
                self.collect_io_format_specs(&args[0], directives)
            }
            _ => Err(self.error_here(format!(
                "Unknown directive: {}",
                format_directive_term(&body, self.interner)
            ))),
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
    ) -> Result<(), ParseError> {
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
                        return Err(self.error_here(format!(
                            "dynamic spec functor must be an atom, got {}",
                            format_directive_term(other, self.interner)
                        )));
                    }
                };
                let arity = match &args[1] {
                    Term::Integer(n) if *n >= 0 => *n as usize,
                    other => {
                        return Err(self.error_here(format!(
                            "dynamic spec arity must be a non-negative integer, got {}",
                            format_directive_term(other, self.interner)
                        )));
                    }
                };
                directives.dynamic.push((f, arity));
                Ok(())
            }
            other => Err(self.error_here(format!(
                "Invalid dynamic spec (expected F/A): {}",
                format_directive_term(&other, self.interner)
            ))),
        }
    }

    /// Walk an `io_format` spec — a single atom, or a comma-chain / proper list
    /// of atoms — into the directives set. Accepts `:- io_format(bson).`,
    /// `:- io_format((text, bson)).`, and `:- io_format([text, bson]).`. The
    /// atom text is validated against the known encoder names at codegen
    /// time; here we just collect strings (dedup is codegen's job, since it
    /// owns the cross-file merge). An improper list (`[text | bson]`) is
    /// rejected rather than silently parsed.
    fn collect_io_format_specs(
        &self,
        spec: &Term,
        directives: &mut ProgramDirectives,
    ) -> Result<(), ParseError> {
        match spec {
            Term::Compound { functor, args }
                if self.interner.resolve(*functor) == "," && args.len() == 2 =>
            {
                self.collect_io_format_specs(&args[0], directives)?;
                self.collect_io_format_specs(&args[1], directives)
            }
            Term::List { head, tail } => {
                self.collect_io_format_specs(head, directives)?;
                match tail.as_ref() {
                    Term::List { .. } => self.collect_io_format_specs(tail, directives),
                    Term::Atom(a) if self.interner.resolve(*a) == "[]" => Ok(()),
                    other => Err(self.error_here(format!(
                        "io_format list must be a proper list of atoms; unexpected tail: {}",
                        format_directive_term(other, self.interner)
                    ))),
                }
            }
            Term::Atom(nil) if self.interner.resolve(*nil) == "[]" => Ok(()),
            Term::Atom(id) => {
                directives
                    .io_format
                    .push(self.interner.resolve(*id).to_string());
                Ok(())
            }
            other => Err(self.error_here(format!(
                "io_format spec must be an atom (or list of atoms), got {}",
                format_directive_term(other, self.interner)
            ))),
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
