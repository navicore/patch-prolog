//! The embedded standard library source.
//!
//! Pure-Prolog list predicates that the
//! compiler parses BEFORE user files and compiles into every binary;
//! user clauses for the same name/arity append to the stdlib predicate.
//! It lives in `plg-shared` because the stdlib *source*
//! is part of the language definition — the compiler embeds it and the
//! LSP offers its predicates in completion, but neither should own it.
//!
//! IMPORTANT: only the compiler (compile-time) and the LSP reference
//! this. The runtime must NOT — `STDLIB_PL` would otherwise be a string
//! literal in every compiled program (it is dead-stripped today because
//! nothing in the runtime path touches it).
pub const STDLIB_PL: &str = include_str!("../stdlib.pl");
