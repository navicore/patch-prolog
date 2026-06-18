//! Tagged 64-bit term words.
//!
//! The encoding (tag values, the low-3-bits split, functor packing) is the
//! word/cell ABI shared with the compiler, so it lives in `plg-shared` as the
//! single source of truth — codegen emits the same layout into `.rodata` fact
//! tables and blobs. Re-exported here so the runtime keeps using `crate::cell`.

pub use plg_shared::cell::*;
