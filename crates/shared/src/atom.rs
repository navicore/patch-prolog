//! Atom identifiers and well-known atoms.
//!
//! Atoms are interned to dense `u32` ids at compile time. The compiler
//! emits the full atom table into the binary as global data; the runtime
//! rebuilds the same name→id map at startup so a `--query` goal interns
//! into the identical id space. Well-known atoms get FIXED ids so codegen
//! and runtime agree without consulting the table.

/// Interned atom identifier.
pub type AtomId = u32;

/// Well-known atoms with fixed ids. The interner pre-seeds these in
/// order, so `intern("[]") == ATOM_NIL` always holds.
pub const ATOM_NIL: AtomId = 0; // []
pub const ATOM_DOT: AtomId = 1; // '.' (list functor)
pub const ATOM_TRUE: AtomId = 2;
pub const ATOM_FAIL: AtomId = 3;
pub const ATOM_FALSE: AtomId = 4;
pub const ATOM_ERROR: AtomId = 5; // error/2 wrapper

/// Names of the well-known atoms, in id order. Used by the interner to
/// pre-seed and by tests to verify the contract.
pub const WELL_KNOWN_ATOMS: &[&str] = &["[]", ".", "true", "fail", "false", "error"];
