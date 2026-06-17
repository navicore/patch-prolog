//! Operator-precedence parser for ISO Prolog programs and queries.
//!
//! Ported from patch-prolog's `parser.rs`, split into focused submodules:
//! - [`operators`]: the operator-name table DATA (token → atom name).
//! - [`term`]: term / primary parsing and the precedence-climbing levels.
//! - [`clause`]: clause parsing and `:- ...` directive handling.
//! - [`query`]: program / query entry points and goal-list parsing.
//!
//! Changes from v1: `fnv::FnvHashMap` → `std::collections::HashMap`, serde
//! derives dropped, and `Term`/`Clause`/`StringInterner`/`VarId`/`AtomId`
//! sourced from `plg_shared`.

mod clause;
pub mod operators;
mod query;
mod term;

use crate::parse_error::ParseError;
use crate::tokenizer::{Token, TokenKind};
use plg_shared::{AtomId, Span, StringInterner, VarId};
use std::collections::HashMap;

/// Directives extracted from a program (`:- dynamic(f/1).` etc).
///
/// Currently only `dynamic/1` is recognized. Future directives (e.g.
/// `multifile`, `discontiguous`) extend this struct.
#[derive(Debug, Default, Clone)]
pub struct ProgramDirectives {
    /// `(functor, arity)` pairs declared `:- dynamic(F/A).`.
    /// A goal referencing a predicate in this set fails silently when no
    /// clauses match, instead of throwing `existence_error`.
    pub dynamic: Vec<(AtomId, usize)>,
}

/// A source occurrence of an atom-functor term (`name` or `name(...)`),
/// captured during parsing. Over-approximates "call sites": it records
/// every such term, goal or data, but never matches text in comments
/// (those aren't parsed). The LSP filters these by the lint's undefined
/// `(name, arity)` set to squiggle real calls instead of text matches.
#[derive(Debug, Clone)]
pub struct CallSite {
    pub functor: AtomId,
    pub arity: usize,
    pub span: Span,
}

/// Parser for Edinburgh Prolog syntax.
/// Parses tokens into Terms and Clauses, with variable scoping per clause.
pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    interner: &'a mut StringInterner,
    var_map: HashMap<String, VarId>,
    next_var: VarId,
    /// Atom-functor term occurrences, accumulated across the whole program
    /// (not reset per clause — the LSP wants every buffer occurrence).
    call_sites: Vec<CallSite>,
}

impl<'a> Parser<'a> {
    /// Build a parser over already-tokenized input.
    fn from_tokens(tokens: Vec<Token>, interner: &'a mut StringInterner) -> Self {
        Parser {
            tokens,
            pos: 0,
            interner,
            var_map: HashMap::new(),
            next_var: 0,
            call_sites: Vec::new(),
        }
    }

    /// Record an atom-functor term occurrence (see [`CallSite`]).
    fn record_call_site(&mut self, functor: AtomId, arity: usize, span: Span) {
        self.call_sites.push(CallSite {
            functor,
            arity,
            span,
        });
    }

    fn reset_vars(&mut self) {
        self.var_map.clear();
        self.next_var = 0;
    }

    fn current(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn current_kind(&self) -> Option<&TokenKind> {
        self.current().map(|t| &t.kind)
    }

    fn at_eof(&self) -> bool {
        matches!(self.current_kind(), None | Some(TokenKind::Eof))
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        self.pos += 1;
        tok
    }

    /// Span of the current token, or a point at end-of-input if exhausted.
    /// All parser errors point "here" — the position the parser stalled at.
    fn here_span(&self) -> Span {
        match self.current() {
            Some(t) => Span::new(0, t.lo, t.hi),
            None => self.eof_span(),
        }
    }

    /// A point span at end of input (the `Eof` token's offset).
    fn eof_span(&self) -> Span {
        let off = self.tokens.last().map(|t| t.hi).unwrap_or(0);
        Span::point(0, off)
    }

    /// Build a `ParseError` pointing at the current token.
    fn error_here(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(message, self.here_span())
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<(), ParseError> {
        match self.current() {
            Some(tok) if &tok.kind == kind => {
                self.advance();
                Ok(())
            }
            Some(tok) => {
                let msg = format!("expected {}, got {}", kind, tok.kind);
                Err(self.error_here(msg))
            }
            None => Err(self.error_here(format!("expected {kind}, got end of input"))),
        }
    }

    /// Get the variable name map (for extracting query variable names in results).
    pub fn var_names(&self) -> &HashMap<String, VarId> {
        &self.var_map
    }
}
