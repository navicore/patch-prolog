//! Prolog term representation (compiler-side AST).
//!
//! Ported from patch-prolog's `term.rs`, minus the serde derives — the
//! compiled artifact is native code + a static atom table, never a
//! serialized term database.

use crate::atom::AtomId;

pub type VarId = u32;

/// Key for first-argument indexing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FirstArgKey {
    Atom(AtomId),
    Integer(i64),
    Functor(AtomId, usize), // functor atom id + arity
}

/// Prolog term representation.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Atom(AtomId),
    Var(VarId),
    Integer(i64),
    Float(f64),
    Compound { functor: AtomId, args: Vec<Term> },
    List { head: Box<Term>, tail: Box<Term> },
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
            Term::Compound { functor, args } => Some(FirstArgKey::Functor(*functor, args.len())),
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
#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    pub head: Term,
    pub body: Vec<Term>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
