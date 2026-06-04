//! The solve driver: dispatches a parsed goal into compiled predicates
//! and runs the backtracking loop.
//!
//! Forward execution inside compiled code is all `musttail` chains; when
//! a chain fails it returns 0 all the way back here, and this loop pops
//! the next choice point, rewinds, and invokes its retry function. The
//! C stack therefore never grows with Prolog recursion or backtracking
//! depth — this loop is the trampoline.

use crate::cell::*;
#[cfg(test)]
use crate::machine::ContFn;
use crate::machine::{MAX_ARGS, Machine};
use crate::render;

pub enum Outcome {
    /// Search finished. `stopped_early` is true when the solution limit
    /// cut enumeration short (=> exhausted:false in the output).
    Done,
    Error,
}

/// Solve `goal`, capturing solutions via the print continuation.
pub fn solve(m: &mut Machine, goal: Word) -> Outcome {
    m.k_fn = capture_k;
    m.k_env = 0;
    let mut r = call_goal(m, goal);
    loop {
        if m.error.is_some() {
            return Outcome::Error;
        }
        if r == 1 {
            return Outcome::Done; // stopped (limit reached)
        }
        match m.cps.pop() {
            None => return Outcome::Done, // exhausted
            Some(cp) => {
                m.rewind_to(cp.trail_mark, cp.heap_mark);
                r = unsafe { (cp.retry)(m as *mut Machine, cp.env) };
            }
        }
    }
}

/// Success continuation for top-level queries: capture the bindings,
/// then ask for more solutions (return 0 = force backtracking) until
/// the limit is reached (return 1 = stop).
unsafe extern "C" fn capture_k(m: *mut Machine, _env: u64) -> i32 {
    let m = unsafe { &mut *m };
    m.solutions.push(render::capture_solution(m));
    match m.solution_limit {
        Some(limit) if m.solutions.len() >= limit => 1,
        _ => 0,
    }
}

/// Dispatch a goal term: control constructs and deterministic builtins
/// go through `control::try_builtin`; everything else is a registry
/// lookup into compiled code. Also used by control continuations (and
/// later by call/1 and findall/3).
pub fn call_goal(m: &mut Machine, goal: Word) -> i32 {
    let goal = m.deref(goal);
    match tag_of(goal) {
        TAG_ATOM => {
            let name = m.atoms.resolve(atom_id(goal)).to_string();
            if let Some(r) = crate::control::try_atom_builtin(m, &name) {
                return r;
            }
            dispatch(m, atom_id(goal), 0, 0)
        }
        TAG_STR => {
            let idx = payload(goal) as usize;
            let (f, n) = unpack_functor(m.heap[idx]);
            let name = m.atoms.resolve(f).to_string();
            if let Some(r) = crate::control::try_builtin(m, &name, idx + 1, n) {
                return r;
            }
            dispatch(m, f, n, idx + 1)
        }
        TAG_REF => {
            m.error = Some(crate::machine::RtError {
                message: "error(instantiation_error, Goal is an unbound variable)".to_string(),
                uncatchable: false,
            });
            0
        }
        _ => {
            let culprit = render::term_to_string(m, goal);
            m.error = Some(crate::machine::RtError {
                message: format!("error(type_error(callable, {culprit}), Goal is not callable)"),
                uncatchable: false,
            });
            0
        }
    }
}

fn dispatch(m: &mut Machine, functor: u32, arity: u32, args_idx: usize) -> i32 {
    let Some(f) = m.registry_lookup(functor, arity) else {
        let name = m.atoms.resolve(functor).to_string();
        // v1 message shape: error(existence_error(procedure, /(name, N)), Undefined procedure: name/N)
        m.error = Some(crate::machine::RtError {
            message: format!(
                "error(existence_error(procedure, /({name}, {arity})), Undefined procedure: {name}/{arity})"
            ),
            uncatchable: false,
        });
        return 0;
    };
    debug_assert!(arity as usize <= MAX_ARGS);
    for i in 0..arity as usize {
        m.areg[i] = m.heap[args_idx + i];
    }
    unsafe { f(m as *mut Machine, 0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::RegistryEntry;
    use plg_shared::StringInterner;

    /// A hand-written "compiled predicate" standing in for codegen
    /// output: `p(a). p(b).` — entry pushes a CP for clause 2, then
    /// tries clause 1, calling the continuation on success.
    unsafe extern "C" fn p_entry(m: *mut Machine, _env: u64) -> i32 {
        let mr = unsafe { &mut *m };
        if !mr.step() {
            return 0;
        }
        // frame: [a0, k_fn, k_env]
        let f = mr.frame_alloc(3);
        mr.heap[f] = mr.areg[0];
        mr.heap[f + 1] = mr.k_fn as usize as u64;
        mr.heap[f + 2] = mr.k_env;
        mr.push_cp(p_clause2, f as u64);
        unsafe { p_clause1(m, f as u64) }
    }

    unsafe extern "C" fn p_clause1(m: *mut Machine, env: u64) -> i32 {
        let mr = unsafe { &mut *m };
        let f = env as usize;
        let atom_a = mr.atoms.lookup("a").unwrap();
        if !crate::unify::unify(mr, mr.heap[f], make_atom(atom_a)) {
            return 0;
        }
        let k: ContFn = unsafe { std::mem::transmute(mr.heap[f + 1] as usize) };
        unsafe { k(m, mr.heap[f + 2]) }
    }

    unsafe extern "C" fn p_clause2(m: *mut Machine, env: u64) -> i32 {
        let mr = unsafe { &mut *m };
        let f = env as usize;
        let atom_b = mr.atoms.lookup("b").unwrap();
        if !crate::unify::unify(mr, mr.heap[f], make_atom(atom_b)) {
            return 0;
        }
        let k: ContFn = unsafe { std::mem::transmute(mr.heap[f + 1] as usize) };
        unsafe { k(m, mr.heap[f + 2]) }
    }

    fn machine_with_p() -> Box<Machine> {
        let mut atoms = StringInterner::new();
        let p = atoms.intern("p");
        atoms.intern("a");
        atoms.intern("b");
        let registry = vec![RegistryEntry {
            functor: p,
            arity: 1,
            f: p_entry,
        }];
        Machine::new(atoms, registry)
    }

    #[test]
    fn enumerates_both_solutions_via_backtracking() {
        let mut m = machine_with_p();
        let goal = crate::query::parse_query(&mut m, "p(X)").unwrap();
        assert!(matches!(solve(&mut m, goal), Outcome::Done));
        assert!(m.error.is_none());
        assert_eq!(m.solutions.len(), 2);
        assert_eq!(m.solutions[0].bindings[0].2, "a");
        assert_eq!(m.solutions[1].bindings[0].2, "b");
    }

    #[test]
    fn ground_query_checks_membership() {
        let mut m = machine_with_p();
        let goal = crate::query::parse_query(&mut m, "p(b)").unwrap();
        solve(&mut m, goal);
        assert_eq!(m.solutions.len(), 1);

        let mut m2 = machine_with_p();
        let goal2 = crate::query::parse_query(&mut m2, "p(zzz)").unwrap();
        solve(&mut m2, goal2);
        assert_eq!(m2.solutions.len(), 0);
    }

    #[test]
    fn limit_stops_enumeration() {
        let mut m = machine_with_p();
        m.solution_limit = Some(1);
        let goal = crate::query::parse_query(&mut m, "p(X)").unwrap();
        solve(&mut m, goal);
        assert_eq!(m.solutions.len(), 1);
    }

    #[test]
    fn conjunction_runs_both_goals() {
        let mut m = machine_with_p();
        let goal = crate::query::parse_query(&mut m, "p(X), p(Y)").unwrap();
        solve(&mut m, goal);
        // 2 x 2 cartesian solutions
        assert_eq!(m.solutions.len(), 4);
    }

    #[test]
    fn unknown_predicate_raises_existence_error() {
        let mut m = machine_with_p();
        let goal = crate::query::parse_query(&mut m, "nosuch(X)").unwrap();
        assert!(matches!(solve(&mut m, goal), Outcome::Error));
        let msg = &m.error.as_ref().unwrap().message;
        assert_eq!(
            msg,
            "error(existence_error(procedure, /(nosuch, 1)), Undefined procedure: nosuch/1)"
        );
    }
}
