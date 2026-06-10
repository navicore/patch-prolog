//! plg-frontend: ISO Prolog tokenizer, parser, and source-level static
//! analysis, ported from patch-prolog.
//!
//! Consumed by the compiler and the LSP. Compiled Prolog binaries carry a
//! minimal goal-only parser inside the runtime instead.

pub mod error;
pub mod lint;
pub mod parser;
pub mod tokenizer;

pub use error::{PrologError, ThrownError, format_term};
pub use parser::{Parser, ProgramDirectives};
pub use tokenizer::{Token, TokenKind, Tokenizer};
