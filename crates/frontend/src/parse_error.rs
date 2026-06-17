//! Structured parse error: a message plus the source `Span` it points at.
//!
//! Replaces the old `"... at line N col M"` string trailer. Consumers render
//! `file:line:col` from `span` via [`crate::source_map::SourceMap`] — the
//! position is no longer baked into the message text.

use crate::tokenizer::Token;
use plg_shared::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl ParseError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    /// Error pointing at `tok`'s lexeme (single-buffer: file `0`).
    pub fn at(message: impl Into<String>, tok: &Token) -> Self {
        Self::new(message, Span::new(0, tok.lo, tok.hi))
    }
}

impl std::fmt::Display for ParseError {
    /// Message only — position is rendered separately from the span, so this
    /// stays free of the old trailer.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
