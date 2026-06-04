//! The M3 builtin layer: arithmetic evaluation (`arith`), standard term
//! order (`order`), and the C-ABI predicate surface (`pred`) that compiled
//! code calls into for `is/2`, arithmetic/term comparison, `\=/2`,
//! `compare/3`, cut, and codegen helpers.
//!
//! Semantics are ported byte-for-byte from patch-prolog v1 so error message
//! text and ordering decisions stay identical; see the per-module docs.

pub mod arith;
pub mod order;
pub mod pred;
