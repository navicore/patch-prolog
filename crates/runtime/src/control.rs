//! Query-level control constructs and deterministic builtins.
//!
//! Clause bodies compile control flow to native code; this module only
//! serves goals built at RUNTIME — the `--query` string today, call/1
//! metacalls in M4. It walks goal TERMS, never clauses (the rule in
//! docs/design/LESSONS_FROM_V1.md stays intact).
//!
//! The implementations mirror the compiled lowering exactly (same
//! choice-point shapes, same commit heights), so a goal behaves
//! identically whether it appears in a clause body or a query.

use crate::builtins::pred;
use crate::cell::*;
use crate::machine::{ContFn, Machine};
use crate::solve::call_goal;
use crate::unify::unify;

/// Invoke the current continuation (a goal succeeded deterministically).
fn invoke_k(m: &mut Machine) -> i32 {
    let k = m.k_fn;
    let e = m.k_env;
    unsafe { k(m as *mut Machine, e) }
}

fn det(m: &mut Machine, ok: bool) -> i32 {
    if ok { invoke_k(m) } else { 0 }
}

/// Try to handle `name/arity` as a control construct or deterministic
/// builtin. Returns None when it's an ordinary predicate call.
pub fn try_builtin(m: &mut Machine, name: &str, args_idx: usize, arity: u32) -> Option<i32> {
    let mp = m as *mut Machine;
    // Snapshot the argument words: goal args are read-only here and the
    // heap may grow (frames) before they're consumed.
    let mut a = [0u64; 3];
    for (i, slot) in a.iter_mut().enumerate().take((arity as usize).min(3)) {
        *slot = m.heap[args_idx + i];
    }
    let arg = |i: usize| -> Word { a[i] };
    let r = match (name, arity) {
        (",", 2) => conjunction(m, arg(0), arg(1)),
        (";", 2) => {
            // `(C -> T ; E)` is if-then-else.
            let lhs = m.deref(arg(0));
            if tag_of(lhs) == TAG_STR {
                let idx = payload(lhs) as usize;
                let (f, n) = unpack_functor(m.heap[idx]);
                if n == 2 && m.atoms.resolve(f) == "->" {
                    let (c, t) = (m.heap[idx + 1], m.heap[idx + 2]);
                    return Some(if_then_else(m, c, t, Some(arg(1))));
                }
            }
            disjunction(m, arg(0), arg(1))
        }
        ("->", 2) => if_then_else(m, arg(0), arg(1), None),
        ("\\+", 1) => naf(m, arg(0)),
        ("once", 1) => once(m, arg(0)),
        ("=", 2) => {
            let ok = unify(m, arg(0), arg(1));
            det(m, ok)
        }
        ("\\=", 2) => {
            let ok = pred::plg_rt_b_neq(mp, arg(0), arg(1)) != 0;
            det(m, ok)
        }
        ("is", 2) => {
            let ok = pred::plg_rt_b_is(mp, arg(0), arg(1)) != 0;
            det(m, ok)
        }
        ("compare", 3) => {
            let ok = pred::plg_rt_b_compare(mp, arg(0), arg(1), arg(2)) != 0;
            det(m, ok)
        }
        (op, 2) if arith_op(op).is_some() => {
            let ok = pred::plg_rt_b_arith_cmp(mp, arith_op(op).unwrap(), arg(0), arg(1)) != 0;
            det(m, ok)
        }
        (op, 2) if order_op(op).is_some() => {
            let ok = pred::plg_rt_b_term_cmp(mp, order_op(op).unwrap(), arg(0), arg(1)) != 0;
            det(m, ok)
        }
        _ => return None,
    };
    Some(r)
}

/// Atom-only goals (`true`, `fail`, `!`).
pub fn try_atom_builtin(m: &mut Machine, name: &str) -> Option<i32> {
    match name {
        "true" => Some(invoke_k(m)),
        "fail" | "false" => Some(0),
        "!" => {
            // The query is a clause body with barrier 0 (v1 semantics).
            m.cps.truncate(0);
            Some(invoke_k(m))
        }
        _ => None,
    }
}

/// ABI op codes — must match codegen's lower.rs tables.
fn arith_op(name: &str) -> Option<i32> {
    Some(match name {
        "<" => 0,
        ">" => 1,
        "=<" => 2,
        ">=" => 3,
        "=:=" => 4,
        "=\\=" => 5,
        _ => return None,
    })
}

fn order_op(name: &str) -> Option<i32> {
    Some(match name {
        "==" => 0,
        "\\==" => 1,
        "@<" => 2,
        "@>" => 3,
        "@=<" => 4,
        "@>=" => 5,
        _ => return None,
    })
}

fn save_k(m: &mut Machine, frame: usize, at: usize) {
    m.heap[frame + at] = m.k_fn as usize as u64;
    m.heap[frame + at + 1] = m.k_env;
}

fn load_k(m: &mut Machine, frame: usize, at: usize) -> (ContFn, u64) {
    let k: ContFn = unsafe { std::mem::transmute(m.heap[frame + at] as usize) };
    (k, m.heap[frame + at + 1])
}

/// `,`/2: run A with a continuation that runs B.
fn conjunction(m: &mut Machine, a: Word, b: Word) -> i32 {
    let frame = m.frame_alloc(3);
    m.heap[frame] = b;
    save_k(m, frame, 1);
    m.k_fn = conj_k;
    m.k_env = frame as u64;
    call_goal(m, a)
}

unsafe extern "C" fn conj_k(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let b = m.heap[frame];
    let (kf, ke) = load_k(m, frame, 1);
    m.k_fn = kf;
    m.k_env = ke;
    call_goal(m, b)
}

/// `(A ; B)`: push a CP retrying B (with the current k restored), run A.
fn disjunction(m: &mut Machine, a: Word, b: Word) -> i32 {
    let frame = m.frame_alloc(3);
    m.heap[frame] = b;
    save_k(m, frame, 1);
    m.push_cp(disj_retry, frame as u64);
    call_goal(m, a)
}

unsafe extern "C" fn disj_retry(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let b = m.heap[frame];
    let (kf, ke) = load_k(m, frame, 1);
    m.k_fn = kf;
    m.k_env = ke;
    call_goal(m, b)
}

/// `(C -> T ; E)` / `(C -> T)`: commit to C's first solution.
fn if_then_else(m: &mut Machine, c: Word, t: Word, e: Option<Word>) -> i32 {
    let h = m.cps.len() as u64; // BEFORE the else CP
    if let Some(e) = e {
        let ef = m.frame_alloc(3);
        m.heap[ef] = e;
        save_k(m, ef, 1);
        m.push_cp(disj_retry, ef as u64);
    }
    let tf = m.frame_alloc(4);
    m.heap[tf] = t;
    save_k(m, tf, 1);
    m.heap[tf + 3] = h;
    m.k_fn = ite_then;
    m.k_env = tf as u64;
    call_goal(m, c)
}

unsafe extern "C" fn ite_then(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let h = m.heap[frame + 3] as usize;
    m.cps.truncate(h); // commit: kill E and C's alternatives
    let t = m.heap[frame];
    let (kf, ke) = load_k(m, frame, 1);
    m.k_fn = kf;
    m.k_env = ke;
    call_goal(m, t)
}

/// `once(G)`: commit to G's first solution, then continue.
fn once(m: &mut Machine, g: Word) -> i32 {
    let h = m.cps.len() as u64;
    let frame = m.frame_alloc(3);
    save_k(m, frame, 0);
    m.heap[frame + 2] = h;
    m.k_fn = once_then;
    m.k_env = frame as u64;
    call_goal(m, g)
}

unsafe extern "C" fn once_then(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let h = m.heap[frame + 2] as usize;
    m.cps.truncate(h);
    let (kf, ke) = load_k(m, frame, 0);
    m.k_fn = kf;
    m.k_env = ke;
    invoke_k(m)
}

/// `\+ G`: push a CP that CONTINUES (driver rewind undoes G's
/// bindings); if G succeeds, cut to the pre-NAF height and fail.
fn naf(m: &mut Machine, g: Word) -> i32 {
    let h = m.cps.len() as u64;
    let cf = m.frame_alloc(2);
    save_k(m, cf, 0);
    m.push_cp(naf_continue, cf as u64);
    let ff = m.frame_alloc(1);
    m.heap[ff] = h;
    m.k_fn = naf_found;
    m.k_env = ff as u64;
    call_goal(m, g)
}

unsafe extern "C" fn naf_continue(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let (kf, ke) = load_k(m, frame, 0);
    m.k_fn = kf;
    m.k_env = ke;
    invoke_k(m)
}

unsafe extern "C" fn naf_found(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let h = m.heap[env as usize] as usize;
    m.cps.truncate(h); // removes the continue-CP and G's alternatives
    0
}

#[cfg(test)]
mod tests {
    use crate::machine::Machine;
    use crate::query::parse_query;
    use crate::solve::{Outcome, solve};
    use plg_shared::StringInterner;

    fn run(query: &str) -> (Vec<String>, Option<String>) {
        let mut m = Machine::new(StringInterner::new(), Vec::new());
        let goal = parse_query(&mut m, query).unwrap();
        let outcome = solve(&mut m, goal);
        let err = match outcome {
            Outcome::Error => Some(m.error.take().unwrap().message),
            Outcome::Done => None,
        };
        let sols = m
            .solutions
            .iter()
            .map(|s| {
                s.bindings
                    .iter()
                    .map(|(n, _, t)| format!("{n}={t}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();
        (sols, err)
    }

    #[test]
    fn top_level_is_and_comparison() {
        assert_eq!(run("X is 2 + 3 * 4").0, vec!["X=14"]);
        assert_eq!(run("1 < 2").0, vec![""]);
        assert_eq!(run("2 < 1").0, Vec::<String>::new());
    }

    #[test]
    fn top_level_disjunction_enumerates() {
        assert_eq!(run("(X = 1 ; X = 2)").0, vec!["X=1", "X=2"]);
    }

    #[test]
    fn top_level_ite_and_naf() {
        assert_eq!(run("(1 < 2 -> X = yes ; X = no)").0, vec!["X=yes"]);
        assert_eq!(run("(2 < 1 -> X = yes ; X = no)").0, vec!["X=no"]);
        assert_eq!(run("\\+ 2 < 1").0, vec![""]);
        assert_eq!(run("\\+ 1 < 2").0, Vec::<String>::new());
        // NAF undoes inner bindings.
        assert_eq!(run("\\+ (X = 1, 2 < 1), X = ok").0, vec!["X=ok"]);
    }

    #[test]
    fn top_level_once_commits() {
        assert_eq!(run("once((X = 1 ; X = 2))").0, vec!["X=1"]);
    }

    #[test]
    fn errors_propagate() {
        let (_, err) = run("X is 1 // 0");
        assert!(err.unwrap().contains("zero_divisor"));
    }
}
