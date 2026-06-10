//! Static lint: calls to predicates that are defined nowhere.
//!
//! patch-prolog is a whole-program compiler with no `assert`/`retract`, so
//! the complete predicate set is known at compile time. A direct body goal
//! that calls a predicate with no clauses, no `:- dynamic` declaration, and
//! which is not a builtin or stdlib predicate can never succeed — at
//! runtime it raises `existence_error(procedure, F/A)`.
//!
//! ISO requires that error to stay a *catchable runtime* condition (so
//! `catch/3` of an undefined call works), which is why callers treat this
//! as a **warning** by default and only promote it to an error on request
//! (`plgc … --deny-undefined`).
//!
//! Lives in the frontend so BOTH the compiler and the LSP can run it
//! without pulling in codegen or the runtime. Only *direct,
//! statically-resolvable* calls are checked; runtime-built goals (a
//! variable goal, `call/N` with N>1) are left to the runtime.

use crate::ProgramDirectives;
use plg_shared::{AtomId, BUILTINS, Clause, StringInterner, Term};
use std::collections::BTreeSet;

/// An undefined-predicate reference, rendered for display.
pub struct Undefined {
    pub callee: (String, usize),
    pub caller: (String, usize),
    /// Closest defined/builtin name of the same arity, if one is near.
    pub suggestion: Option<String>,
}

/// Collect every `(caller, callee)` where a clause body directly calls a
/// predicate that is defined nowhere. Deduplicated and deterministically
/// ordered. `clauses`/`directives` must be the FULL compilation unit
/// (stdlib included) so stdlib calls are not flagged.
pub fn undefined_calls(
    clauses: &[Clause],
    directives: &ProgramDirectives,
    interner: &StringInterner,
) -> Vec<Undefined> {
    let mut defined: BTreeSet<(AtomId, usize)> = BTreeSet::new();
    for c in clauses {
        if let Some(k) = c.head.functor_arity() {
            defined.insert(k);
        }
    }
    for &(f, a) in &directives.dynamic {
        defined.insert((f, a));
    }

    // dedup key: (caller_name, caller_arity, callee_name, callee_arity)
    let mut hits: BTreeSet<(String, usize, String, usize)> = BTreeSet::new();
    for c in clauses {
        let Some((cf, ca)) = c.head.functor_arity() else {
            continue;
        };
        let caller = (interner.resolve(cf).to_string(), ca);
        for goal in &c.body {
            collect(goal, &defined, interner, &mut |fid, arity| {
                hits.insert((
                    caller.0.clone(),
                    caller.1,
                    interner.resolve(fid).to_string(),
                    arity,
                ));
            });
        }
    }

    let defined_names: Vec<(String, usize)> = defined
        .iter()
        .map(|&(f, a)| (interner.resolve(f).to_string(), a))
        .collect();
    hits.into_iter()
        .map(|(cn, ca, en, ea)| Undefined {
            suggestion: suggest(&en, ea, &defined_names),
            callee: (en, ea),
            caller: (cn, ca),
        })
        .collect()
}

/// Format a lint as `<predicate> ... [— did you mean <x>?]`.
pub fn message(u: &Undefined) -> String {
    let mut m = format!(
        "undefined predicate {}/{} called from {}/{}",
        u.callee.0, u.callee.1, u.caller.0, u.caller.1
    );
    if let Some(s) = &u.suggestion {
        m.push_str(&format!(" — did you mean {s}?"));
    }
    m
}

/// Walk a goal term, invoking `emit(fid, arity)` for each direct call to a
/// predicate not in `defined` and not a recognized builtin. Recurses into
/// the goal positions of control constructs and meta-predicates.
fn collect(
    goal: &Term,
    defined: &BTreeSet<(AtomId, usize)>,
    interner: &StringInterner,
    emit: &mut impl FnMut(AtomId, usize),
) {
    let Some((fid, arity)) = goal.functor_arity() else {
        return; // variable goal / non-callable: runtime's problem
    };
    let name = interner.resolve(fid);
    let args: &[Term] = match goal {
        Term::Compound { args, .. } => args,
        _ => &[],
    };
    match (name, arity) {
        (",", 2) | (";", 2) | ("->", 2) => {
            collect(&args[0], defined, interner, emit);
            collect(&args[1], defined, interner, emit);
        }
        ("\\+", 1) | ("once", 1) => collect(&args[0], defined, interner, emit),
        // `call(G)` checks G; `call(G, Extra…)` can't be resolved to a
        // fixed arity statically — leave it to the runtime.
        ("call", 1) => collect(&args[0], defined, interner, emit),
        ("call", _) => {}
        ("findall", 3) => collect(&args[1], defined, interner, emit),
        ("catch", 3) => {
            collect(&args[0], defined, interner, emit);
            collect(&args[2], defined, interner, emit);
        }
        // Any other recognized builtin: its args are data, not goals.
        _ if is_builtin(name, arity) => {}
        // A real user-predicate call.
        _ if !defined.contains(&(fid, arity)) => emit(fid, arity),
        _ => {}
    }
}

fn is_builtin(name: &str, arity: usize) -> bool {
    BUILTINS
        .iter()
        .any(|b| b.name == name && b.arity as usize == arity)
}

/// Nearest same-arity defined or builtin name within edit distance 2.
fn suggest(name: &str, arity: usize, defined: &[(String, usize)]) -> Option<String> {
    let candidates = defined
        .iter()
        .filter(|(_, a)| *a == arity)
        .map(|(n, a)| (n.as_str(), *a))
        .chain(
            BUILTINS
                .iter()
                .filter(|b| b.arity as usize == arity)
                .map(|b| (b.name, b.arity as usize)),
        );
    let mut best: Option<(usize, String)> = None;
    for (cand, a) in candidates {
        if cand == name {
            continue;
        }
        let d = edit_distance(name, cand);
        if d <= 2 && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, format!("{cand}/{a}")));
        }
    }
    best.map(|(_, s)| s)
}

/// Levenshtein distance (small inputs; the two-row DP is plenty).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Parser;

    /// Undefined callees (name/arity) for a self-contained program.
    fn callees(src: &str) -> Vec<(String, usize)> {
        let mut interner = StringInterner::new();
        let (clauses, directives) =
            Parser::parse_program_with_directives(src, &mut interner).unwrap();
        let mut v: Vec<_> = undefined_calls(&clauses, &directives, &interner)
            .into_iter()
            .map(|u| u.callee)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn flags_a_direct_undefined_call() {
        assert_eq!(callees("a :- b.\n"), vec![("b".into(), 0)]);
    }

    #[test]
    fn defined_predicate_is_not_flagged() {
        assert!(callees("a :- b.\nb.\n").is_empty());
    }

    #[test]
    fn builtins_and_inline_ops_not_flagged() {
        assert!(callees("a(X) :- X is 1 + 2, X > 0, write(X).\n").is_empty());
    }

    #[test]
    fn dynamic_declaration_suppresses() {
        assert!(callees(":- dynamic(b/1).\na(X) :- b(X).\n").is_empty());
    }

    #[test]
    fn recurses_into_disjunction_and_naf() {
        assert_eq!(
            callees("a :- (b ; \\+ c).\n"),
            vec![("b".into(), 0), ("c".into(), 0)]
        );
    }

    #[test]
    fn recurses_into_findall_goal_and_catch() {
        assert_eq!(
            callees("a :- findall(X, gen(X), _).\n"),
            vec![("gen".into(), 1)]
        );
        assert_eq!(
            callees("a :- catch(risky, _, recover).\n"),
            vec![("recover".into(), 0), ("risky".into(), 0)]
        );
    }

    #[test]
    fn call_of_one_arg_is_checked_higher_arities_deferred() {
        assert_eq!(callees("a :- call(b).\n"), vec![("b".into(), 0)]);
        // call(G, X) -> effective arity unknowable; not flagged.
        assert!(callees("a :- call(b, x).\n").is_empty());
    }

    #[test]
    fn variable_goal_is_left_to_runtime() {
        assert!(callees("a(G) :- G.\n").is_empty());
    }

    #[test]
    fn suggests_a_near_defined_name() {
        let mut interner = StringInterner::new();
        let (clauses, directives) = Parser::parse_program_with_directives(
            "parent(tom).\nq :- xarent(tom).\n",
            &mut interner,
        )
        .unwrap();
        let u = &undefined_calls(&clauses, &directives, &interner)[0];
        assert_eq!(u.suggestion.as_deref(), Some("parent/1"));
    }
}
