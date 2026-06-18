//! The builtin layer: arithmetic evaluation (`arith`), standard term
//! order (`order`), and the C-ABI predicate surface (`pred`) that compiled
//! code calls into for `is/2`, arithmetic/term comparison, `\=/2`,
//! `compare/3`, cut, and codegen helpers.
//!
//! The M4 deterministic builtins live in their own family modules:
//! `typecheck` (var/atom/number/... type tests), `termops` (functor/arg/
//! `=..`/copy_term), `atomops` (atom_length/concat/chars, number_chars/
//! codes), `sortops` (msort/sort), and `miscops` (succ/plus/
//! unify_with_occurs_check/write/writeln/nl).
//!
//! Semantics are ported byte-for-byte from patch-prolog v1 so error message
//! text and ordering decisions stay identical; see the per-module docs.

pub mod arith;
pub mod atomops;
pub mod facts;
pub mod miscops;
pub mod order;
pub mod pred;
pub mod sortops;
pub mod termops;
pub mod typecheck;
