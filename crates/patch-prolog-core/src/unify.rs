use crate::term::{Term, VarId};
use fnv::FnvHashSet;

/// Vec-based substitution with trail for efficient backtracking.
/// Bindings are stored in a Vec indexed by VarId (O(1) lookup/bind).
/// The trail records which VarIds were bound, enabling undo on backtracking.
#[derive(Debug, Clone)]
pub struct Substitution {
    bindings: Vec<Option<Term>>,
    trail: Vec<VarId>,
}

impl Substitution {
    pub fn new() -> Self {
        Substitution {
            bindings: Vec::new(),
            trail: Vec::new(),
        }
    }

    /// Create a substitution pre-sized for the given number of variables.
    pub fn with_capacity(n: usize) -> Self {
        Substitution {
            bindings: vec![None; n],
            trail: Vec::new(),
        }
    }

    /// Get the current trail mark (for backtracking).
    pub fn trail_mark(&self) -> usize {
        self.trail.len()
    }

    /// Undo all bindings back to the given trail mark.
    pub fn undo_to(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let var = self.trail.pop().unwrap();
            self.bindings[var as usize] = None;
        }
    }

    /// Bind a variable to a term.
    fn bind(&mut self, var: VarId, term: Term) {
        let idx = var as usize;
        if idx >= self.bindings.len() {
            self.bindings.resize(idx + 1, None);
        }
        self.bindings[idx] = Some(term);
        self.trail.push(var);
    }

    /// Look up a variable's binding.
    fn lookup(&self, var: VarId) -> Option<&Term> {
        self.bindings.get(var as usize).and_then(|b| b.as_ref())
    }

    /// Dereference: follow variable chains to their ultimate value.
    pub fn walk(&self, term: &Term) -> Term {
        match term {
            Term::Var(id) => match self.lookup(*id) {
                Some(bound) => self.walk(bound),
                None => term.clone(),
            },
            _ => term.clone(),
        }
    }

    /// Deep walk: recursively substitute all variables in a term.
    /// Handles circular terms (from unification without occurs check) by
    /// stopping expansion when a variable cycle is detected.
    pub fn apply(&self, term: &Term) -> Term {
        let mut seen = FnvHashSet::default();
        self.apply_impl(term, &mut seen)
    }

    fn apply_impl(&self, term: &Term, seen: &mut FnvHashSet<VarId>) -> Term {
        match term {
            Term::Var(id) => {
                if seen.contains(id) {
                    // Cycle detected — return variable as-is to break infinite recursion
                    return term.clone();
                }
                match self.lookup(*id) {
                    Some(bound) => {
                        seen.insert(*id);
                        let result = self.apply_impl(bound, seen);
                        seen.remove(id);
                        result
                    }
                    None => term.clone(),
                }
            }
            Term::Compound { functor, args } => Term::Compound {
                functor: *functor,
                args: args.iter().map(|a| self.apply_impl(a, seen)).collect(),
            },
            Term::List { head, tail } => Term::List {
                head: Box::new(self.apply_impl(head, seen)),
                tail: Box::new(self.apply_impl(tail, seen)),
            },
            _ => term.clone(),
        }
    }

    /// Unify two terms. Returns true if unification succeeds.
    /// On failure, bindings made during this attempt remain (caller should undo via trail).
    pub fn unify(&mut self, t1: &Term, t2: &Term) -> bool {
        let t1 = self.walk(t1);
        let t2 = self.walk(t2);

        match (&t1, &t2) {
            // Both same variable
            (Term::Var(a), Term::Var(b)) if a == b => true,

            // Bind variable to the other term (ISO: no occurs check for =/2)
            (Term::Var(id), other) | (other, Term::Var(id)) => {
                self.bind(*id, other.clone());
                true
            }

            // Atom equality
            (Term::Atom(a), Term::Atom(b)) => a == b,

            // Integer equality
            (Term::Integer(a), Term::Integer(b)) => a == b,

            // Float equality (use to_bits for structural equality — handles NaN)
            (Term::Float(a), Term::Float(b)) => a.to_bits() == b.to_bits(),

            // Compound: same functor and arity, then unify args pairwise
            (
                Term::Compound {
                    functor: f1,
                    args: a1,
                },
                Term::Compound {
                    functor: f2,
                    args: a2,
                },
            ) => {
                if f1 != f2 || a1.len() != a2.len() {
                    return false;
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    if !self.unify(arg1, arg2) {
                        return false;
                    }
                }
                true
            }

            // List: unify head and tail
            (Term::List { head: h1, tail: t1 }, Term::List { head: h2, tail: t2 }) => {
                self.unify(h1, h2) && self.unify(t1, t2)
            }

            // Anything else fails
            _ => false,
        }
    }

    /// Unify with occurs check (ISO 8.2.2).
    /// Fails if binding a variable would create a circular term.
    pub fn unify_with_occurs_check(&mut self, t1: &Term, t2: &Term) -> bool {
        let t1 = self.walk(t1);
        let t2 = self.walk(t2);

        match (&t1, &t2) {
            (Term::Var(a), Term::Var(b)) if a == b => true,
            (Term::Var(id), other) | (other, Term::Var(id)) => {
                if self.occurs_in(*id, other) {
                    return false;
                }
                self.bind(*id, other.clone());
                true
            }
            (Term::Atom(a), Term::Atom(b)) => a == b,
            (Term::Integer(a), Term::Integer(b)) => a == b,
            (Term::Float(a), Term::Float(b)) => a.to_bits() == b.to_bits(),
            (
                Term::Compound {
                    functor: f1,
                    args: a1,
                },
                Term::Compound {
                    functor: f2,
                    args: a2,
                },
            ) => {
                if f1 != f2 || a1.len() != a2.len() {
                    return false;
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    if !self.unify_with_occurs_check(arg1, arg2) {
                        return false;
                    }
                }
                true
            }
            (Term::List { head: h1, tail: t1 }, Term::List { head: h2, tail: t2 }) => {
                self.unify_with_occurs_check(h1, h2) && self.unify_with_occurs_check(t1, t2)
            }
            _ => false,
        }
    }

    fn occurs_in(&self, var: VarId, term: &Term) -> bool {
        match term {
            Term::Var(id) => {
                if *id == var {
                    return true;
                }
                match self.lookup(*id) {
                    Some(bound) => self.occurs_in(var, bound),
                    None => false,
                }
            }
            Term::Compound { args, .. } => args.iter().any(|a| self.occurs_in(var, a)),
            Term::List { head, tail } => self.occurs_in(var, head) || self.occurs_in(var, tail),
            _ => false,
        }
    }
}

impl Default for Substitution {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::Term;

    #[test]
    fn test_unify_atoms() {
        let mut sub = Substitution::new();
        assert!(sub.unify(&Term::Atom(0), &Term::Atom(0)));
        assert!(!sub.unify(&Term::Atom(0), &Term::Atom(1)));
    }

    #[test]
    fn test_unify_integers() {
        let mut sub = Substitution::new();
        assert!(sub.unify(&Term::Integer(42), &Term::Integer(42)));
        assert!(!sub.unify(&Term::Integer(1), &Term::Integer(2)));
    }

    #[test]
    fn test_unify_var_to_atom() {
        let mut sub = Substitution::new();
        assert!(sub.unify(&Term::Var(0), &Term::Atom(1)));
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(1));
    }

    #[test]
    fn test_unify_var_to_var() {
        let mut sub = Substitution::new();
        assert!(sub.unify(&Term::Var(0), &Term::Var(1)));
        // After binding, both should resolve to the same thing
        assert!(sub.unify(&Term::Var(1), &Term::Atom(5)));
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(5));
    }

    #[test]
    fn test_unify_compound() {
        let mut sub = Substitution::new();
        let t1 = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0), Term::Atom(1)],
        };
        let t2 = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(2), Term::Atom(1)],
        };
        assert!(sub.unify(&t1, &t2));
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(2));
    }

    #[test]
    fn test_unify_compound_mismatch_functor() {
        let mut sub = Substitution::new();
        let t1 = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1)],
        };
        let t2 = Term::Compound {
            functor: 1,
            args: vec![Term::Atom(1)],
        };
        assert!(!sub.unify(&t1, &t2));
    }

    #[test]
    fn test_unify_compound_mismatch_arity() {
        let mut sub = Substitution::new();
        let t1 = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1)],
        };
        let t2 = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1), Term::Atom(2)],
        };
        assert!(!sub.unify(&t1, &t2));
    }

    #[test]
    fn test_no_occurs_check() {
        let mut sub = Substitution::new();
        // X = f(X) should succeed (ISO: =/2 does not occurs-check)
        let t1 = Term::Var(0);
        let t2 = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0)],
        };
        assert!(sub.unify(&t1, &t2));
    }

    #[test]
    fn test_trail_backtracking() {
        let mut sub = Substitution::new();

        let mark = sub.trail_mark();
        assert!(sub.unify(&Term::Var(0), &Term::Atom(1)));
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(1));

        sub.undo_to(mark);
        // Var should be free again
        assert_eq!(sub.walk(&Term::Var(0)), Term::Var(0));
    }

    #[test]
    fn test_apply() {
        let mut sub = Substitution::new();
        sub.unify(&Term::Var(0), &Term::Atom(5));
        sub.unify(&Term::Var(1), &Term::Integer(42));

        let term = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0), Term::Var(1), Term::Var(2)],
        };
        let applied = sub.apply(&term);
        match applied {
            Term::Compound { args, .. } => {
                assert_eq!(args[0], Term::Atom(5));
                assert_eq!(args[1], Term::Integer(42));
                assert_eq!(args[2], Term::Var(2)); // unbound
            }
            _ => panic!("Expected compound"),
        }
    }

    #[test]
    fn test_unify_list() {
        let mut sub = Substitution::new();
        let t1 = Term::List {
            head: Box::new(Term::Var(0)),
            tail: Box::new(Term::Atom(10)), // nil
        };
        let t2 = Term::List {
            head: Box::new(Term::Atom(5)),
            tail: Box::new(Term::Atom(10)),
        };
        assert!(sub.unify(&t1, &t2));
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(5));
    }

    #[test]
    fn test_unify_same_var() {
        let mut sub = Substitution::new();
        assert!(sub.unify(&Term::Var(0), &Term::Var(0)));
    }

    #[test]
    fn test_multiple_trail_marks() {
        let mut sub = Substitution::new();

        let mark1 = sub.trail_mark();
        sub.unify(&Term::Var(0), &Term::Atom(1));

        let mark2 = sub.trail_mark();
        sub.unify(&Term::Var(1), &Term::Atom(2));

        // Undo second binding only
        sub.undo_to(mark2);
        assert_eq!(sub.walk(&Term::Var(0)), Term::Atom(1));
        assert_eq!(sub.walk(&Term::Var(1)), Term::Var(1));

        // Undo first binding
        sub.undo_to(mark1);
        assert_eq!(sub.walk(&Term::Var(0)), Term::Var(0));
    }
}
