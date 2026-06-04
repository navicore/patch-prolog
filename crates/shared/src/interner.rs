//! String interner mapping atom names to dense `AtomId`s.
//!
//! Ported from patch-prolog's `term.rs` interner (same API), with two
//! deliberate changes:
//! - no serde/fnv dependencies (this crate ships in every compiled
//!   binary and must stay dependency-free)
//! - pre-seeds the well-known atoms so fixed ids hold (see `atom.rs`)

use crate::atom::{AtomId, WELL_KNOWN_ATOMS};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct StringInterner {
    to_id: HashMap<String, AtomId>,
    to_str: Vec<String>,
}

impl StringInterner {
    pub fn new() -> Self {
        let mut interner = StringInterner {
            to_id: HashMap::new(),
            to_str: Vec::new(),
        };
        for name in WELL_KNOWN_ATOMS {
            interner.intern(name);
        }
        interner
    }

    /// Intern a string, returning its AtomId. If already interned, returns existing id.
    pub fn intern(&mut self, s: &str) -> AtomId {
        if let Some(&id) = self.to_id.get(s) {
            return id;
        }
        let id = self.to_str.len() as AtomId;
        self.to_str.push(s.to_string());
        self.to_id.insert(s.to_string(), id);
        id
    }

    /// Resolve an AtomId back to its string. Panics if id is invalid.
    pub fn resolve(&self, id: AtomId) -> &str {
        &self.to_str[id as usize]
    }

    /// Try to resolve an AtomId, returning None if invalid.
    pub fn try_resolve(&self, id: AtomId) -> Option<&str> {
        self.to_str.get(id as usize).map(|s| s.as_str())
    }

    /// Look up a string without interning it.
    pub fn lookup(&self, s: &str) -> Option<AtomId> {
        self.to_id.get(s).copied()
    }

    pub fn len(&self) -> usize {
        self.to_str.len()
    }

    pub fn is_empty(&self) -> bool {
        self.to_str.is_empty()
    }

    /// Iterate names in id order (used by codegen to emit the atom table).
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.to_str.iter().map(|s| s.as_str())
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::{ATOM_DOT, ATOM_NIL, ATOM_TRUE};

    #[test]
    fn well_known_atoms_have_fixed_ids() {
        let mut i = StringInterner::new();
        assert_eq!(i.intern("[]"), ATOM_NIL);
        assert_eq!(i.intern("."), ATOM_DOT);
        assert_eq!(i.intern("true"), ATOM_TRUE);
        assert_eq!(i.resolve(ATOM_NIL), "[]");
    }

    #[test]
    fn test_string_interner_basic() {
        let mut interner = StringInterner::new();
        let a = interner.intern("hello");
        let b = interner.intern("world");
        let c = interner.intern("hello"); // duplicate

        assert_eq!(a, c);
        assert_ne!(a, b);
        assert_eq!(interner.resolve(a), "hello");
        assert_eq!(interner.resolve(b), "world");
    }

    #[test]
    fn test_string_interner_lookup() {
        let mut interner = StringInterner::new();
        let id = interner.intern("foo");

        assert_eq!(interner.lookup("foo"), Some(id));
        assert_eq!(interner.lookup("bar"), None);
    }
}
