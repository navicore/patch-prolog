use crate::term::{AtomId, Clause, FirstArgKey, Term};
use fnv::FnvHashMap;
use serde::{Deserialize, Serialize};

/// Entry for a single predicate (functor/arity).
/// Supports two-tier indexing: functor/arity lookup, then first-argument hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredicateEntry {
    /// First-argument index: ground first arg -> clause indices
    pub arg_index: FnvHashMap<FirstArgKey, Vec<usize>>,
    /// Clause indices where the first arg is a variable (must always be included)
    pub var_clause_indices: Vec<usize>,
    /// All clause indices in source order (fallback)
    pub all_clause_indices: Vec<usize>,
}

/// Two-tier predicate index: (functor AtomId, arity) -> PredicateEntry
pub type PredicateIndex = FnvHashMap<(AtomId, usize), PredicateEntry>;

/// Build the predicate index from a list of clauses.
pub fn build_index(clauses: &[Clause]) -> PredicateIndex {
    let mut index: PredicateIndex = FnvHashMap::default();

    for (clause_idx, clause) in clauses.iter().enumerate() {
        let (functor, arity) = match clause.head.functor_arity() {
            Some(fa) => fa,
            None => continue, // Skip malformed clauses
        };

        let entry = index
            .entry((functor, arity))
            .or_insert_with(|| PredicateEntry {
                arg_index: FnvHashMap::default(),
                var_clause_indices: Vec::new(),
                all_clause_indices: Vec::new(),
            });

        entry.all_clause_indices.push(clause_idx);

        match clause.head.first_arg_key() {
            Some(key) => {
                entry
                    .arg_index
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .push(clause_idx);
            }
            None => {
                // Variable or no first arg — goes to var_clauses
                if let Term::Compound { args, .. } = &clause.head {
                    if !args.is_empty() && args[0].is_var() {
                        entry.var_clause_indices.push(clause_idx);
                    }
                }
                // Atom head (arity 0) has no first arg — no special indexing needed
            }
        }
    }

    index
}

/// Look up candidate clause indices for a goal.
/// Returns indices into the clauses Vec.
pub fn lookup_clauses(
    index: &PredicateIndex,
    goal: &Term,
    clauses: &[Clause],
) -> Vec<usize> {
    let _ = clauses; // used for type reference only
    let (functor, arity) = match goal.functor_arity() {
        Some(fa) => fa,
        None => return vec![],
    };

    let entry = match index.get(&(functor, arity)) {
        Some(e) => e,
        None => return vec![],
    };

    // If any clause has a variable first arg, we must use all_clauses to preserve ordering
    if !entry.var_clause_indices.is_empty() {
        return entry.all_clause_indices.clone();
    }

    // Try to use first-arg indexing if the goal's first arg is ground
    let first_arg_key = match goal {
        Term::Compound { args, .. } if !args.is_empty() => {
            match &args[0] {
                Term::Atom(id) => Some(FirstArgKey::Atom(*id)),
                Term::Integer(n) => Some(FirstArgKey::Integer(*n)),
                Term::Compound { functor, args } => {
                    Some(FirstArgKey::Functor(*functor, args.len()))
                }
                _ => None, // Variable first arg in query -> use all_clauses
            }
        }
        _ => None,
    };

    match first_arg_key {
        Some(key) => {
            match entry.arg_index.get(&key) {
                Some(indices) => indices.clone(),
                None => entry.all_clause_indices.clone(), // fallback
            }
        }
        None => entry.all_clause_indices.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Clause, Term};

    fn make_fact(functor: AtomId, args: Vec<Term>) -> Clause {
        Clause {
            head: Term::Compound { functor, args },
            body: vec![],
        }
    }

    #[test]
    fn test_build_index_basic() {
        let clauses = vec![
            make_fact(0, vec![Term::Atom(1)]),
            make_fact(0, vec![Term::Atom(2)]),
            make_fact(0, vec![Term::Atom(3)]),
        ];
        let index = build_index(&clauses);
        let entry = index.get(&(0, 1)).unwrap();
        assert_eq!(entry.all_clause_indices, vec![0, 1, 2]);
        assert!(entry.var_clause_indices.is_empty());
        assert_eq!(entry.arg_index.len(), 3);
    }

    #[test]
    fn test_lookup_with_ground_arg() {
        let clauses = vec![
            make_fact(0, vec![Term::Atom(1)]),
            make_fact(0, vec![Term::Atom(2)]),
            make_fact(0, vec![Term::Atom(3)]),
        ];
        let index = build_index(&clauses);

        // Lookup with specific first arg
        let goal = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(2)],
        };
        let result = lookup_clauses(&index, &goal, &clauses);
        assert_eq!(result, vec![1]); // only the matching clause
    }

    #[test]
    fn test_lookup_with_variable_arg() {
        let clauses = vec![
            make_fact(0, vec![Term::Atom(1)]),
            make_fact(0, vec![Term::Atom(2)]),
            make_fact(0, vec![Term::Atom(3)]),
        ];
        let index = build_index(&clauses);

        // Lookup with variable first arg -> all clauses
        let goal = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0)],
        };
        let result = lookup_clauses(&index, &goal, &clauses);
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn test_lookup_unknown_predicate() {
        let clauses = vec![make_fact(0, vec![Term::Atom(1)])];
        let index = build_index(&clauses);

        let goal = Term::Compound {
            functor: 99,
            args: vec![Term::Atom(1)],
        };
        let result = lookup_clauses(&index, &goal, &clauses);
        assert!(result.is_empty());
    }

    #[test]
    fn test_mixed_ground_and_var_clauses() {
        // When some clauses have variable first arg, all lookups use all_clauses
        let clauses = vec![
            make_fact(0, vec![Term::Atom(1)]),
            make_fact(0, vec![Term::Atom(2)]),
            Clause {
                head: Term::Compound {
                    functor: 0,
                    args: vec![Term::Var(0)],
                },
                body: vec![],
            },
        ];
        let index = build_index(&clauses);

        let goal = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(1)],
        };
        let result = lookup_clauses(&index, &goal, &clauses);
        // Must return all clauses because there's a variable-headed clause
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn test_multiple_predicates() {
        let clauses = vec![
            make_fact(0, vec![Term::Atom(10)]), // color(red)
            make_fact(0, vec![Term::Atom(11)]), // color(blue)
            make_fact(1, vec![Term::Atom(20)]), // shape(circle)
        ];
        let index = build_index(&clauses);

        // color lookup
        let goal_color = Term::Compound {
            functor: 0,
            args: vec![Term::Var(0)],
        };
        assert_eq!(lookup_clauses(&index, &goal_color, &clauses), vec![0, 1]);

        // shape lookup
        let goal_shape = Term::Compound {
            functor: 1,
            args: vec![Term::Var(0)],
        };
        assert_eq!(lookup_clauses(&index, &goal_shape, &clauses), vec![2]);
    }

    #[test]
    fn test_no_match_returns_all_clauses() {
        let clauses = vec![
            make_fact(0, vec![Term::Atom(1)]),
            make_fact(0, vec![Term::Atom(2)]),
        ];
        let index = build_index(&clauses);

        // Query with an atom that doesn't match any first arg
        let goal = Term::Compound {
            functor: 0,
            args: vec![Term::Atom(99)],
        };
        let result = lookup_clauses(&index, &goal, &clauses);
        // Falls back to all_clauses since no match in arg_index
        assert_eq!(result, vec![0, 1]);
    }
}
