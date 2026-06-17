//! Codegen-path program parsing: like `parse_program_with_directives`, but
//! each clause body is split into its top-level conjuncts, each carrying a
//! source `Span` (SPANS.md Layer 3). Codegen threads those spans to raising
//! call sites so runtime errors can name `file:line:col`.
//!
//! Granularity: top-level comma-separated goals get precise spans; a goal
//! nested inside a top-level `;`/`->`/`\+` (or a parenthesized conjunction)
//! inherits its enclosing conjunct's span — coarser, but present. A
//! top-level `;` makes the whole body a single spanned goal.

use super::{Parser, ProgramDirectives};
use crate::parse_error::ParseError;
use crate::tokenizer::{TokenKind, Tokenizer};
use plg_shared::{Span, Spanned, StringInterner, Term};

/// A clause as codegen consumes it: a plain head term plus body goals whose
/// source spans survive lowering. (Provenance for the head is not needed —
/// existence errors point at the call site, not the definition.)
#[derive(Clone)]
pub struct CgClause {
    pub head: Term,
    pub body: Vec<Spanned<Term>>,
}

impl<'a> Parser<'a> {
    /// Parse a complete program for codegen, stamping `file_id` on every body
    /// goal span (SPANS.md Layer 3). Mirrors `parse_program_with_directives`;
    /// only the body representation differs.
    pub fn parse_program_cg(
        input: &str,
        interner: &'a mut StringInterner,
        file_id: plg_shared::FileId,
    ) -> Result<(Vec<CgClause>, ProgramDirectives), ParseError> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser::from_tokens(tokens, interner);
        parser.file_id = file_id;
        parser.parse_program_cg_body()
    }
}

impl Parser<'_> {
    fn parse_program_cg_body(&mut self) -> Result<(Vec<CgClause>, ProgramDirectives), ParseError> {
        let mut clauses = Vec::new();
        let mut directives = ProgramDirectives::default();
        while !self.at_eof() {
            self.reset_vars();
            if self.current_kind() == Some(&TokenKind::Neck) {
                self.advance();
                let body = self.parse_term()?;
                self.expect(&TokenKind::Dot)?;
                self.process_directive(body, &mut directives)?;
            } else {
                clauses.push(self.parse_clause_cg()?);
            }
        }
        Ok((clauses, directives))
    }

    fn parse_clause_cg(&mut self) -> Result<CgClause, ParseError> {
        let head = self.parse_term()?;
        match self.current_kind() {
            Some(TokenKind::Dot) => {
                self.advance();
                Ok(CgClause { head, body: vec![] })
            }
            Some(TokenKind::Neck) => {
                self.advance();
                let body = self.parse_body_conjuncts()?;
                self.expect(&TokenKind::Dot)?;
                Ok(CgClause { head, body })
            }
            Some(tok) => {
                let msg = format!("expected `.` or `:-`, got {tok}");
                Err(self.error_here(msg))
            }
            None => Err(self.error_here("Unexpected end of input in clause")),
        }
    }

    /// Body as top-level conjuncts with spans. A top-level `;` (looser than
    /// `,`) collapses the whole body to one spanned goal.
    fn parse_body_conjuncts(&mut self) -> Result<Vec<Spanned<Term>>, ParseError> {
        let body_lo = self.here_lo();
        let mut conjuncts = Vec::new();
        self.push_conjunct(&mut conjuncts)?;
        while self.current_kind() == Some(&TokenKind::Comma) {
            self.advance();
            self.push_conjunct(&mut conjuncts)?;
        }
        if self.current_kind() == Some(&TokenKind::Semicolon) {
            // Top-level disjunction: rebuild what we parsed as the left arm,
            // parse the rest, and return the whole `;` as a single goal.
            let comma = self.interner.intern(",");
            let left = rebuild_conjunction(conjuncts, comma);
            self.advance();
            let right = self.parse_goal_disjunction()?;
            let semi = self.interner.intern(";");
            let hi = self.prev_hi();
            let whole = Term::Compound {
                functor: semi,
                args: vec![left, right],
            };
            return Ok(vec![Spanned::new(
                whole,
                Span::new(self.file_id, body_lo, hi),
            )]);
        }
        Ok(conjuncts)
    }

    fn push_conjunct(&mut self, out: &mut Vec<Spanned<Term>>) -> Result<(), ParseError> {
        let lo = self.here_lo();
        let t = self.parse_term()?;
        let hi = self.prev_hi();
        out.push(Spanned::new(t, Span::new(self.file_id, lo, hi)));
        Ok(())
    }

    /// Byte offset of the current token's start (or end-of-input).
    fn here_lo(&self) -> u32 {
        self.here_span().lo
    }

    /// Byte offset just past the most recently consumed token.
    fn prev_hi(&self) -> u32 {
        self.pos
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map(|t| t.hi)
            .unwrap_or(0)
    }
}

/// Right-associate a goal list back into a `,`-tree: `[a, b, c]` →
/// `','(a, ','(b, c))`. Inverse of the conjunct flatten.
fn rebuild_conjunction(mut goals: Vec<Spanned<Term>>, comma: plg_shared::AtomId) -> Term {
    let mut acc = goals.pop().expect("at least one conjunct").node;
    while let Some(g) = goals.pop() {
        acc = Term::Compound {
            functor: comma,
            args: vec![g.node, acc],
        };
    }
    acc
}
