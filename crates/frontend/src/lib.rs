//! plg-frontend: ISO Prolog tokenizer, parser, and source-level static
//! analysis, ported from patch-prolog.
//!
//! Consumed by the compiler and the LSP. Compiled Prolog binaries carry a
//! minimal goal-only parser inside the runtime instead.

pub mod error;
pub mod lint;
pub mod parse_error;
pub mod parser;
pub mod source_map;
pub mod tokenizer;

pub use error::{PrologError, ThrownError, format_term};
pub use parse_error::ParseError;
pub use parser::{CallSite, CgClause, Parser, ProgramDirectives};
pub use source_map::SourceMap;
pub use tokenizer::{Token, TokenKind, Tokenizer};
