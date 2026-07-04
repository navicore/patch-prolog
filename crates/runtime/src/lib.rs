//! plg-runtime: the support library linked into every compiled Prolog
//! binary (`libplg_runtime.a`).
//!
//! It provides the machine substrate compiled predicates call into —
//! term heap, trail, choice-point stack, generic unification, the
//! goal-only `--query` parser, solution output, and the process entry
//! point. It contains NO clause interpreter: clause control flow lives
//! in the LLVM IR that `plgc` generates (see
//! docs/design/COMPILATION_MODEL.md and docs/design/LESSONS_FROM_V1.md).

pub mod abi;
pub mod builtins;
pub mod cell;
pub mod control;
pub mod copyterm;
pub mod core;
pub mod entry;
pub mod errors;
pub mod machine;
pub mod query;
pub mod reactor;
pub mod render;
pub mod solve;
pub mod unify;
pub mod wire;
