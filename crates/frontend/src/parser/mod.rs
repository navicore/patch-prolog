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

use crate::tokenizer::{Token, TokenKind};
use plg_shared::{AtomId, StringInterner, VarId};
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

/// Parser for Edinburgh Prolog syntax.
/// Parses tokens into Terms and Clauses, with variable scoping per clause.
pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    interner: &'a mut StringInterner,
    var_map: HashMap<String, VarId>,
    next_var: VarId,
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
        }
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

    fn expect(&mut self, kind: &TokenKind) -> Result<(), String> {
        match self.current() {
            Some(tok) if &tok.kind == kind => {
                self.advance();
                Ok(())
            }
            Some(tok) => Err(format!(
                "expected {}, got {} at line {} col {}",
                kind, tok.kind, tok.line, tok.col
            )),
            None => Err(format!("expected {kind}, got end of input")),
        }
    }

    /// Get the variable name map (for extracting query variable names in results).
    pub fn var_names(&self) -> &HashMap<String, VarId> {
        &self.var_map
    }
}
