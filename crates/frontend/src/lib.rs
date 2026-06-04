//! plg-frontend: ISO Prolog tokenizer and parser, ported from patch-prolog.
//!
//! This crate is consumed only by the compiler. Compiled Prolog binaries
//! carry a minimal goal-only parser inside the runtime instead.

pub mod error;
pub mod parser;
pub mod tokenizer;

pub use error::{PrologError, ThrownError, format_term};
pub use parser::{Parser, ProgramDirectives};
pub use tokenizer::{Token, TokenKind, Tokenizer};
