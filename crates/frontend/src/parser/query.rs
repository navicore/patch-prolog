//! Program / query entry points and goal-list (conjunction / disjunction)
//! parsing. Ported from patch-prolog's `parser.rs`.

use super::{Parser, ProgramDirectives};
use crate::parse_error::ParseError;
use crate::tokenizer::{TokenKind, Tokenizer};
use plg_shared::{Clause, StringInterner, Term, VarId};
use std::collections::HashMap;

impl<'a> Parser<'a> {
    /// Parse a complete program (multiple clauses) from source text.
    /// Directives (`:- ...`) are recognized and skipped — use
    /// `parse_program_with_directives` to capture them.
    pub fn parse_program(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<Vec<Clause>, ParseError> {
        let (clauses, _) = Self::parse_program_with_directives(input, interner)?;
        Ok(clauses)
    }

    /// Parse a complete program, returning both clauses and any directives
    /// (currently `:- dynamic(F/A).`). The compile pipeline uses this so the
    /// directive information reaches the database.
    pub fn parse_program_with_directives(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<(Vec<Clause>, ProgramDirectives), ParseError> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser::from_tokens(tokens, interner);
        parser.parse_program_body()
    }

    /// Like `parse_program_with_directives`, but also returns the atom-functor
    /// call-site occurrences (see [`super::CallSite`]) for the LSP to map
    /// undefined-predicate warnings onto precise source ranges.
    pub fn parse_program_with_spans(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<(Vec<Clause>, ProgramDirectives, Vec<super::CallSite>), ParseError> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser::from_tokens(tokens, interner);
        let (clauses, directives) = parser.parse_program_body()?;
        Ok((clauses, directives, parser.call_sites))
    }

    /// Shared program-parsing loop. Clauses are collected; `:- ...` directives
    /// are interpreted into `directives`.
    fn parse_program_body(&mut self) -> Result<(Vec<Clause>, ProgramDirectives), ParseError> {
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
                clauses.push(self.parse_clause()?);
            }
        }
        Ok((clauses, directives))
    }

    /// Parse a single query (goal list) from source text, e.g. "parent(tom, X)".
    /// Does NOT require a trailing dot.
    pub fn parse_query(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<Vec<Term>, ParseError> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser::from_tokens(tokens, interner);
        // Skip optional ?- prefix
        if parser.current_kind() == Some(&TokenKind::QueryOp) {
            parser.advance();
        }
        let goals = parser.parse_goal_list()?;
        // Allow optional trailing dot
        if parser.current_kind() == Some(&TokenKind::Dot) {
            parser.advance();
        }
        // Issue #30: the whole input must be consumed. Without this, a query
        // like `member(X,[1,2,3]) zzz` would silently drop the trailing tokens
        // and report success. The "after query" phrasing distinguishes this
        // from mid-expression parse errors.
        if !parser.at_eof() {
            let pos = parser.current().unwrap();
            let msg = format!("unexpected {} after query", pos.kind);
            return Err(parser.error_here(msg));
        }
        Ok(goals)
    }

    /// Parse a query and also return the variable name mapping.
    pub fn parse_query_with_vars(
        input: &str,
        interner: &mut StringInterner,
    ) -> Result<(Vec<Term>, HashMap<String, VarId>), ParseError> {
        let tokens = Tokenizer::tokenize(input)?;
        let mut parser = Parser::from_tokens(tokens, interner);
        if parser.current_kind() == Some(&TokenKind::QueryOp) {
            parser.advance();
        }
        let goals = parser.parse_goal_list()?;
        if parser.current_kind() == Some(&TokenKind::Dot) {
            parser.advance();
        }
        // Issue #30 — see `parse_query` for rationale.
        if !parser.at_eof() {
            let pos = parser.current().unwrap();
            let msg = format!("unexpected {} after query", pos.kind);
            return Err(parser.error_here(msg));
        }
        let vars = parser.var_map;
        Ok((goals, vars))
    }

    pub(super) fn parse_goal_list(&mut self) -> Result<Vec<Term>, ParseError> {
        // Parse the entire body as a conjunction/disjunction tree.
        // The solver flattens ','(a, b) via BuiltinResult::Conjunction.
        let body = self.parse_goal_disjunction()?;
        Ok(vec![body])
    }

    /// Parse disjunction (;) — ISO precedence 1100, looser than comma (1000).
    fn parse_goal_disjunction(&mut self) -> Result<Term, ParseError> {
        let left = self.parse_goal_conjunction()?;
        if self.current_kind() == Some(&TokenKind::Semicolon) {
            self.advance();
            let right = self.parse_goal_disjunction()?;
            let functor = self.interner.intern(";");
            Ok(Term::Compound {
                functor,
                args: vec![left, right],
            })
        } else {
            Ok(left)
        }
    }

    /// Parse conjunction (,) — ISO precedence 1000, tighter than semicolon.
    fn parse_goal_conjunction(&mut self) -> Result<Term, ParseError> {
        let first = self.parse_term()?;
        if self.current_kind() == Some(&TokenKind::Comma) {
            let mut goals = vec![first];
            while self.current_kind() == Some(&TokenKind::Comma) {
                self.advance();
                goals.push(self.parse_term()?);
            }
            // Build right-associative conjunction: a, b, c → ','(a, ','(b, c))
            let comma = self.interner.intern(",");
            let mut result = goals.pop().unwrap();
            while let Some(g) = goals.pop() {
                result = Term::Compound {
                    functor: comma,
                    args: vec![g, result],
                };
            }
            Ok(result)
        } else {
            Ok(first)
        }
    }
}

#[cfg(test)]
mod tests;
