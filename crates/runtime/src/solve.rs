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
    m.qbarrier = 0;
    let r = call_goal(m, goal);
    drive(m, 0, r);
    if m.error.is_some() {
        Outcome::Error
    } else {
        Outcome::Done
    }
}

/// The backtracking driver: pops choice points down to `floor`,
/// rewinding and retrying, and unwinds errors to the nearest catch
/// frame. Shared by the top level and findall/3's bounded sub-search.
/// Returns 1 if a continuation stopped enumeration; otherwise 0 (CP
/// stack drained to floor, or an uncaught error left in `m.error` for
/// the next layer out).
pub fn drive(m: &mut Machine, floor: usize, mut r: i32) -> i32 {
    loop {
        if m.error.is_some() {
            if m.error.as_ref().is_some_and(|e| e.uncatchable) {
                return 0;
            }
            match unwind_to_catch(m, floor) {
                Some(r2) => {
                    r = r2;
                    continue;
                }
                None => return 0, // no catch here; error stays set
            }
        }
        if r == 1 {
            return 1; // stop (solution limit / committed)
        }
        if m.cps.len() <= floor {
            return 0; // exhausted this window
        }
        let cp = m.cps.pop().unwrap();
        m.rewind_to(cp.trail_mark, cp.heap_mark);
        r = unsafe { (cp.retry)(m as *mut Machine, cp.env) };
    }
}

/// Unwind the CP stack looking for a catch frame whose catcher unifies
/// with the ball (v1 dispatch_or_return semantics). On a match, clears
/// the error, restores the catch-time continuation, and runs the
/// recovery goal, returning its result. `None` if no frame matches
/// above `floor` (the error remains set).
fn unwind_to_catch(m: &mut Machine, floor: usize) -> Option<i32> {
    use crate::machine::CpKind;
    while m.cps.len() > floor {
        let cp = m.cps.pop().unwrap();
        m.rewind_to(cp.trail_mark, cp.heap_mark);
        if cp.kind != CpKind::Catch {
            continue;
        }
        // Catch frame env: [catcher, recovery, k_fn, k_env, qbarrier] —
        // allocated before the frame's marks, so it survived the rewind.
        let f = cp.env as usize;
        let catcher = m.heap[f];
        let recovery = m.heap[f + 1];
        let err = m.error.take().unwrap();
        let ball_w = crate::copyterm::restore_from_buf(m, &err.ball);
        let tmark = m.trail.len();
        if crate::unify::unify(m, catcher, ball_w) {
            let kf: crate::machine::ContFn = unsafe { std::mem::transmute(m.heap[f + 2] as usize) };
            m.k_fn = kf;
            m.k_env = m.heap[f + 3];
            m.qbarrier = m.heap[f + 4] as usize;
            return Some(call_goal(m, recovery));
        }
        // Catcher doesn't match: undo the attempt, keep unwinding with
        // the error re-armed. (The restored ball leaks a few heap cells
        // until the next rewind — harmless.)
        while m.trail.len() > tmark {
            let idx = m.trail.pop().unwrap() as usize;
            m.heap[idx] = make_ref(idx);
        }
        m.error = Some(err);
    }
    None
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
            crate::errors::instantiation(m, "Goal is an unbound variable");
            0
        }
        _ => {
            crate::errors::type_error(m, "callable", goal, "Goal is not callable");
            0
        }
    }
}

fn dispatch(m: &mut Machine, functor: u32, arity: u32, args_idx: usize) -> i32 {
    let Some(f) = m.registry_lookup(functor, arity) else {
        let name = m.atoms.resolve(functor).to_string();
        // Query-side / metacall undefined goals have no compiled call site.
        crate::errors::existence_procedure(m, &name, arity, crate::machine::NO_SITE);
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
