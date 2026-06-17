//! plg-shared: types used by both the compiler (codegen) and the runtime
//! (query parsing / output) — atom interning, term representation, the
//! operator table, and first-argument indexing keys.
//!
//! This crate is linked into every compiled Prolog binary via
//! `libplg_runtime.a`. It must stay dependency-free and lean.

pub mod atom;
pub mod builtins;
pub mod interner;
pub mod span;
pub mod stdlib;
pub mod term;

pub use atom::AtomId;
pub use builtins::{BUILTINS, BuiltinKind, BuiltinSpec};
pub use interner::StringInterner;
pub use span::{FileId, Span, Spanned};
pub use stdlib::STDLIB_PL;
pub use term::{Clause, FirstArgKey, Term, VarId};
