use fnv::FnvHashMap;
use serde::{Deserialize, Serialize};

pub type AtomId = u32;
pub type VarId = u32;

/// Interned string table: AtomId <-> String mapping.
/// Atoms are interned at build time so unification compares integers, not strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringInterner {
    to_id: FnvHashMap<String, AtomId>,
    to_str: Vec<String>,
}

impl StringInterner {
    pub fn new() -> Self {
        StringInterner {
            to_id: FnvHashMap::default(),
            to_str: Vec::new(),
        }
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
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

/// Key for first-argument indexing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FirstArgKey {
    Atom(AtomId),
    Integer(i64),
    Functor(AtomId, usize), // functor atom id + arity
}

/// Prolog term representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Term {
    Atom(AtomId),
    Var(VarId),
    Integer(i64),
    Float(f64),
    Compound {
        functor: AtomId,
        args: Vec<Term>,
    },
    List {
        head: Box<Term>,
        tail: Box<Term>,
    },
}

impl Term {
    /// Extract functor AtomId and arity for a term used as a goal/head.
    /// - Atom: (atom_id, 0)
    /// - Compound: (functor, len(args))
    /// - Others: None
    pub fn functor_arity(&self) -> Option<(AtomId, usize)> {
        match self {
            Term::Atom(id) => Some((*id, 0)),
            Term::Compound { functor, args } => Some((*functor, args.len())),
            _ => None,
        }
    }

    /// Extract the first-argument indexing key from a term used as a clause head.
    pub fn first_arg_key(&self) -> Option<FirstArgKey> {
        let first = match self {
            Term::Compound { args, .. } if !args.is_empty() => &args[0],
            _ => return None,
        };
        match first {
            Term::Atom(id) => Some(FirstArgKey::Atom(*id)),
            Term::Integer(n) => Some(FirstArgKey::Integer(*n)),
            Term::Compound { functor, args } => {
                Some(FirstArgKey::Functor(*functor, args.len()))
            }
            _ => None, // Var, Float, List -> not indexable
        }
    }

    /// Check if this term is a variable.
    pub fn is_var(&self) -> bool {
        matches!(self, Term::Var(_))
    }
}

/// A Prolog clause: head :- body.
/// For facts, body is empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clause {
    pub head: Term,
    pub body: Vec<Term>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(interner.len(), 2);
    }

    #[test]
    fn test_string_interner_lookup() {
        let mut interner = StringInterner::new();
        interner.intern("foo");

        assert_eq!(interner.lookup("foo"), Some(0));
        assert_eq!(interner.lookup("bar"), None);
    }

    #[test]
    fn test_term_functor_arity() {
        let atom = Term::Atom(0);
        assert_eq!(atom.functor_arity(), Some((0, 0)));

        let compound = Term::Compound {
            functor: 1,
            args: vec![Term::Atom(2), Term::Var(0)],
        };
        assert_eq!(compound.functor_arity(), Some((1, 2)));

        let var = Term::Var(0);
        assert_eq!(var.functor_arity(), None);

        let int = Term::Integer(42);
        assert_eq!(int.functor_arity(), None);
    }

    #[test]
    fn test_first_arg_key() {
        // Compound with atom first arg
        let t = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1)],
        };
        assert_eq!(t.first_arg_key(), Some(FirstArgKey::Atom(1)));

        // Compound with integer first arg
        let t = Term::Compound {
            functor: 0,
            args: vec![Term::Integer(42)],
        };
        assert_eq!(t.first_arg_key(), Some(FirstArgKey::Integer(42)));

        // Compound with variable first arg -> None (not indexable)
        let t = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0)],
        };
        assert_eq!(t.first_arg_key(), None);

        // Atom (no args) -> None
        let t = Term::Atom(0);
        assert_eq!(t.first_arg_key(), None);
    }

    #[test]
    fn test_clause_construction() {
        let clause = Clause {
            head: Term::Compound {
                functor: 0,
                args: vec![Term::Atom(1), Term::Var(0)],
            },
            body: vec![Term::Compound {
                functor: 2,
                args: vec![Term::Var(0)],
            }],
        };
        assert_eq!(clause.body.len(), 1);
        assert_eq!(clause.head.functor_arity(), Some((0, 2)));
    }

    #[test]
    fn test_term_serialization() {
        let term = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1), Term::Integer(42)],
        };
        let bytes = bincode::serialize(&term).unwrap();
        let restored: Term = bincode::deserialize(&bytes).unwrap();
        assert_eq!(term, restored);
    }
}
