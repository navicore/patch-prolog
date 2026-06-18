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
use crate::machine::{ContFn, Machine, NO_SITE};
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
    let a: Vec<Word> = (0..arity as usize).map(|i| m.heap[args_idx + i]).collect();
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
        ("catch", 3) => catch_impl(m, arg(0), arg(1), arg(2)),
        ("throw", 1) => {
            crate::errors::throw_term(m, arg(0));
            0
        }
        ("findall", 3) => findall_impl(m, arg(0), arg(1), arg(2)),
        ("call", n) if n >= 1 => metacall_extend(m, arg(0), &a[1..]),
        ("between", 3) => between_impl(m, arg(0), arg(1), arg(2)),
        ("=", 2) => {
            let ok = unify(m, arg(0), arg(1));
            det(m, ok)
        }
        ("\\=", 2) => {
            let ok = pred::plg_rt_b_neq(mp, arg(0), arg(1)) != 0;
            det(m, ok)
        }
        ("is", 2) => {
            // Runtime-walked (query/metacall): no compiled call site.
            let ok = pred::plg_rt_b_is(mp, arg(0), arg(1), crate::machine::NO_SITE) != 0;
            det(m, ok)
        }
        ("compare", 3) => {
            let ok = pred::plg_rt_b_compare(mp, arg(0), arg(1), arg(2)) != 0;
            det(m, ok)
        }
        (op, 2) if arith_op(op).is_some() => {
            let ok = pred::plg_rt_b_arith_cmp(
                mp,
                arith_op(op).unwrap(),
                arg(0),
                arg(1),
                crate::machine::NO_SITE,
            ) != 0;
            det(m, ok)
        }
        (op, 2) if order_op(op).is_some() => {
            let ok = pred::plg_rt_b_term_cmp(mp, order_op(op).unwrap(), arg(0), arg(1)) != 0;
            det(m, ok)
        }
        _ => {
            let ok = det_builtin(mp, name, arity, &a)?;
            det(m, ok)
        }
    };
    Some(r)
}

/// Query-side dispatch for the deterministic builtin vocabulary —
/// mirrors codegen's DET_BUILTINS table (lower.rs); the diff corpus
/// guards the pair. Returns None for non-builtins.
fn det_builtin(mp: *mut Machine, name: &str, arity: u32, a: &[Word]) -> Option<bool> {
    use crate::builtins::{atomops, miscops, sortops, termops, typecheck};
    let r = match (name, arity) {
        ("var", 1) => typecheck::plg_rt_b_var_1(mp, a[0]),
        ("nonvar", 1) => typecheck::plg_rt_b_nonvar_1(mp, a[0]),
        ("atom", 1) => typecheck::plg_rt_b_atom_1(mp, a[0]),
        ("number", 1) => typecheck::plg_rt_b_number_1(mp, a[0]),
        ("integer", 1) => typecheck::plg_rt_b_integer_1(mp, a[0]),
        ("float", 1) => typecheck::plg_rt_b_float_1(mp, a[0]),
        ("compound", 1) => typecheck::plg_rt_b_compound_1(mp, a[0]),
        ("is_list", 1) => typecheck::plg_rt_b_is_list_1(mp, a[0]),
        // Runtime-walked (query/metacall): raising builtins get NO_SITE.
        ("functor", 3) => termops::plg_rt_b_functor_3(mp, a[0], a[1], a[2], NO_SITE),
        ("arg", 3) => termops::plg_rt_b_arg_3(mp, a[0], a[1], a[2], NO_SITE),
        ("=..", 2) => termops::plg_rt_b_univ_2(mp, a[0], a[1], NO_SITE),
        ("copy_term", 2) => termops::plg_rt_b_copy_term_2(mp, a[0], a[1]),
        ("atom_length", 2) => atomops::plg_rt_b_atom_length_2(mp, a[0], a[1], NO_SITE),
        ("atom_concat", 3) => atomops::plg_rt_b_atom_concat_3(mp, a[0], a[1], a[2], NO_SITE),
        ("atom_chars", 2) => atomops::plg_rt_b_atom_chars_2(mp, a[0], a[1], NO_SITE),
        ("number_chars", 2) => atomops::plg_rt_b_number_chars_2(mp, a[0], a[1], NO_SITE),
        ("number_codes", 2) => atomops::plg_rt_b_number_codes_2(mp, a[0], a[1], NO_SITE),
        ("msort", 2) => sortops::plg_rt_b_msort_2(mp, a[0], a[1], NO_SITE),
        ("sort", 2) => sortops::plg_rt_b_sort_2(mp, a[0], a[1], NO_SITE),
        ("succ", 2) => miscops::plg_rt_b_succ_2(mp, a[0], a[1], NO_SITE),
        ("plus", 3) => miscops::plg_rt_b_plus_3(mp, a[0], a[1], a[2], NO_SITE),
        ("unify_with_occurs_check", 2) => {
            miscops::plg_rt_b_unify_with_occurs_check_2(mp, a[0], a[1])
        }
        ("write", 1) => miscops::plg_rt_b_write_1(mp, a[0]),
        ("writeln", 1) => miscops::plg_rt_b_writeln_1(mp, a[0]),
        _ => return None,
    };
    Some(r != 0)
}

/// Atom-only goals (`true`, `fail`, `!`).
pub fn try_atom_builtin(m: &mut Machine, name: &str) -> Option<i32> {
    match name {
        "true" => Some(invoke_k(m)),
        "nl" => {
            let ok = crate::builtins::miscops::plg_rt_b_nl_0(m as *mut Machine) != 0;
            Some(det(m, ok))
        }
        "fail" | "false" => Some(0),
        "!" => {
            // Cut to the walker barrier: 0 at the query top level,
            // local inside call-like constructs (\+, once, ->-condition,
            // call/N, findall goals). cut_to stops at catch frames.
            let h = m.qbarrier;
            m.cut_to(h);
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

/// Snapshot the continuation AND the walker cut barrier (3 cells:
/// k_fn, k_env, qbarrier) — the runtime mirror of compiled cut slots.
fn save_k(m: &mut Machine, frame: usize, at: usize) {
    m.heap[frame + at] = m.k_fn as usize as u64;
    m.heap[frame + at + 1] = m.k_env;
    m.heap[frame + at + 2] = m.qbarrier as u64;
}

/// Restore continuation + barrier from a frame.
fn load_k(m: &mut Machine, frame: usize, at: usize) -> (ContFn, u64) {
    let k: ContFn = unsafe { std::mem::transmute(m.heap[frame + at] as usize) };
    m.qbarrier = m.heap[frame + at + 2] as usize;
    (k, m.heap[frame + at + 1])
}

/// `,`/2: run A with a continuation that runs B.
fn conjunction(m: &mut Machine, a: Word, b: Word) -> i32 {
    let frame = m.frame_alloc(4);
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
    let frame = m.frame_alloc(4);
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
        let ef = m.frame_alloc(4);
        m.heap[ef] = e;
        save_k(m, ef, 1);
        m.push_cp(disj_retry, ef as u64);
    }
    let tf = m.frame_alloc(5);
    m.heap[tf] = t;
    save_k(m, tf, 1);
    m.heap[tf + 4] = h;
    m.k_fn = ite_then;
    m.k_env = tf as u64;
    // The condition is call-like: `!` inside it is local (cuts to the
    // height AFTER the else CP).
    m.qbarrier = m.cps.len();
    call_goal(m, c)
}

unsafe extern "C" fn ite_then(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let h = m.heap[frame + 4] as usize;
    m.cps.truncate(h); // commit: kill E and C's alternatives
    let t = m.heap[frame];
    let (kf, ke) = load_k(m, frame, 1); // also restores the outer barrier
    m.k_fn = kf;
    m.k_env = ke;
    call_goal(m, t)
}

/// `once(G)`: commit to G's first solution, then continue.
fn once(m: &mut Machine, g: Word) -> i32 {
    let h = m.cps.len() as u64;
    let frame = m.frame_alloc(4);
    save_k(m, frame, 0);
    m.heap[frame + 3] = h;
    m.k_fn = once_then;
    m.k_env = frame as u64;
    m.qbarrier = m.cps.len(); // call-like: `!` inside G is local
    call_goal(m, g)
}

unsafe extern "C" fn once_then(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let h = m.heap[frame + 3] as usize;
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
    let cf = m.frame_alloc(3);
    save_k(m, cf, 0);
    m.push_cp(naf_continue, cf as u64);
    let ff = m.frame_alloc(1);
    m.heap[ff] = h;
    m.k_fn = naf_found;
    m.k_env = ff as u64;
    // Call-like: `!` inside G is local (cannot prune the continue CP).
    m.qbarrier = m.cps.len();
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

/// `catch(Goal, Catcher, Recovery)`: push a catch frame (transparent to
/// normal backtracking, a target for error unwinding in solve::drive
/// and a barrier for cut), then run Goal with the current continuation.
fn catch_impl(m: &mut Machine, goal: Word, catcher: Word, recovery: Word) -> i32 {
    let frame = m.frame_alloc(5);
    m.heap[frame] = catcher;
    m.heap[frame + 1] = recovery;
    save_k(m, frame, 2);
    m.push_catch_cp(catch_retry, frame as u64);
    call_goal(m, goal)
}

/// Backtracking INTO a catch frame (no error in flight) is transparent:
/// the frame offers no alternatives.
unsafe extern "C" fn catch_retry(_m: *mut Machine, _env: u64) -> i32 {
    0
}

/// `findall(Template, Goal, Bag)`: run Goal to exhaustion in a bounded
/// sub-search, snapshotting Template per solution; rewind everything;
/// unify Bag with the collected list. Errors propagate out (a catch
/// inside Goal is handled by the shared driver within our floor).
fn findall_impl(m: &mut Machine, template: Word, goal: Word, bag: Word) -> i32 {
    let floor = m.cps.len();
    let tmark = m.trail.len();
    let hmark = m.heap.len();
    let saved_k = (m.k_fn, m.k_env, m.qbarrier);
    m.findall_stack.push(Vec::new());
    let cf = m.frame_alloc(1);
    m.heap[cf] = template;
    m.k_fn = findall_collect;
    m.k_env = cf as u64;
    m.qbarrier = m.cps.len(); // call-like: `!` inside Goal is local
    let r = call_goal(m, goal);
    crate::solve::drive(m, floor, r); // collector always returns 0
    let results = m.findall_stack.pop().unwrap();
    m.k_fn = saved_k.0;
    m.k_env = saved_k.1;
    m.qbarrier = saved_k.2;
    if m.error.is_some() {
        return 0;
    }
    // Discard the goal's bindings and workspace, then build the bag.
    m.rewind_to(tmark, hmark);
    let mut w = make_atom(plg_shared::atom::ATOM_NIL);
    for buf in results.iter().rev() {
        let e = crate::copyterm::restore_from_buf(m, buf);
        let idx = m.heap.len();
        m.heap.push(e);
        m.heap.push(w);
        w = make(TAG_LST, idx as u64);
    }
    let ok = unify(m, bag, w);
    det(m, ok)
}

unsafe extern "C" fn findall_collect(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let template = m.heap[env as usize];
    let buf = crate::copyterm::copy_to_buf(m, template);
    m.findall_stack
        .last_mut()
        .expect("collector outside findall")
        .push(buf);
    0 // force backtracking into the next solution
}

/// `call/N`: extend the goal with extra arguments (ISO 7.8.3 partial
/// application), then dispatch it.
fn metacall_extend(m: &mut Machine, goal: Word, extras: &[Word]) -> i32 {
    let goal = m.deref(goal);
    // call/N is opaque to cut (ISO): `!` inside the called goal is local.
    m.qbarrier = m.cps.len();
    if extras.is_empty() {
        return call_goal(m, goal);
    }
    let extended = match tag_of(goal) {
        TAG_ATOM => {
            let f = atom_id(goal);
            let idx = m.heap.len();
            m.heap.push(pack_functor(f, extras.len() as u32));
            m.heap.extend_from_slice(extras);
            make(TAG_STR, idx as u64)
        }
        TAG_STR => {
            let sidx = payload(goal) as usize;
            let (f, n) = unpack_functor(m.heap[sidx]);
            let idx = m.heap.len();
            m.heap.push(pack_functor(f, n + extras.len() as u32));
            for i in 0..n as usize {
                let w = m.heap[sidx + 1 + i];
                m.heap.push(w);
            }
            m.heap.extend_from_slice(extras);
            make(TAG_STR, idx as u64)
        }
        TAG_REF => {
            crate::errors::instantiation(m, "call/N requires a bound goal");
            return 0;
        }
        _ => {
            crate::errors::type_error(m, "callable", goal, "Goal is not callable");
            return 0;
        }
    };
    call_goal(m, extended)
}

/// `between(Low, High, X)` — the one nondeterministic builtin: with X
/// unbound it enumerates Low..=High via a runtime-retried choice point.
fn between_impl(m: &mut Machine, lo_w: Word, hi_w: Word, x_w: Word) -> i32 {
    let Some(lo) = int_arg(m, lo_w, "between/3") else {
        return 0;
    };
    let Some(hi) = int_arg(m, hi_w, "between/3") else {
        return 0;
    };
    let x = m.deref(x_w);
    match tag_of(x) {
        TAG_INT | TAG_BIG => {
            let xv = if tag_of(x) == TAG_INT {
                int_value(x)
            } else {
                m.heap[payload(x) as usize] as i64
            };
            det(m, lo <= xv && xv <= hi)
        }
        TAG_REF => {
            if lo > hi {
                return 0;
            }
            // Frame: [x, current, high, k_fn, k_env, qbarrier].
            // `current` is mutated in place across retries (untrailed
            // by design — the frame is control state, not a term).
            let frame = m.frame_alloc(6);
            m.heap[frame] = x;
            m.heap[frame + 1] = lo as u64;
            m.heap[frame + 2] = hi as u64;
            save_k(m, frame, 3);
            if lo < hi {
                m.push_cp(between_retry, frame as u64);
            }
            let w = int_to_word(m, lo);
            let ok = unify(m, x, w);
            debug_assert!(ok, "binding a fresh var cannot fail");
            invoke_k(m)
        }
        _ => {
            crate::errors::type_error(m, "integer", x, "between/3 requires an integer");
            0
        }
    }
}

unsafe extern "C" fn between_retry(m: *mut Machine, env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = env as usize;
    let x = m.heap[frame];
    let cur = m.heap[frame + 1] as i64 + 1;
    let hi = m.heap[frame + 2] as i64;
    m.heap[frame + 1] = cur as u64;
    if cur < hi {
        m.push_cp(between_retry, frame as u64);
    }
    let (kf, ke) = load_k(m, frame, 3);
    m.k_fn = kf;
    m.k_env = ke;
    let w = int_to_word(m, cur);
    let ok = unify(m, x, w);
    debug_assert!(ok, "x rewound to unbound before retry");
    invoke_k(m)
}

/// Build an integer word: immediate when it fits i61, boxed otherwise.
fn int_to_word(m: &mut Machine, n: i64) -> Word {
    if (INT_MIN..=INT_MAX).contains(&n) {
        make_int(n)
    } else {
        let idx = m.heap.len();
        m.heap.push(n as u64);
        make(TAG_BIG, idx as u64)
    }
}

/// Deref an integer argument for between/3 (v1: bounds must be bound
/// integers).
fn int_arg(m: &mut Machine, w: Word, who: &str) -> Option<i64> {
    let w = m.deref(w);
    match tag_of(w) {
        TAG_INT => Some(int_value(w)),
        TAG_BIG => Some(m.heap[payload(w) as usize] as i64),
        TAG_REF => {
            let ctx = format!("{who} requires bound integer bounds");
            crate::errors::instantiation(m, &ctx);
            None
        }
        _ => {
            let ctx = format!("{who} requires an integer");
            crate::errors::type_error(m, "integer", w, &ctx);
            None
        }
    }
}

/// Compiled-code entry for between/3 (uniform predicate signature,
/// arguments in the A registers — dispatched like a user predicate).
/// # Safety
/// Called from generated code with the live Machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_pred_between_3(m: *mut Machine, _env: u64) -> i32 {
    let m = unsafe { &mut *m };
    let (lo, hi, x) = (m.areg[0], m.areg[1], m.areg[2]);
    between_impl(m, lo, hi, x)
}

/// Compiled-code entries for control builtins taking goal terms.
/// # Safety
/// Called from generated code with the live Machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_metacall(m: *mut Machine, goal: u64) -> i32 {
    let m = unsafe { &mut *m };
    // Goals reaching the walker from compiled code are call-like:
    // a runtime-walked `!` inside them is local.
    m.qbarrier = m.cps.len();
    call_goal(m, goal)
}

/// # Safety
/// Called from generated code with the live Machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_b_catch_3(m: *mut Machine, g: u64, c: u64, r: u64) -> i32 {
    catch_impl(unsafe { &mut *m }, g, c, r)
}

/// # Safety
/// Called from generated code with the live Machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_b_throw_1(m: *mut Machine, ball: u64) -> i32 {
    crate::errors::throw_term(unsafe { &mut *m }, ball);
    0
}

/// # Safety
/// Called from generated code with the live Machine pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_b_findall_3(m: *mut Machine, t: u64, g: u64, b: u64) -> i32 {
    findall_impl(unsafe { &mut *m }, t, g, b)
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

#[cfg(test)]
mod m4_tests {
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
    fn throw_uncaught_propagates_with_rendered_ball() {
        let (sols, err) = run("throw(my_ball)");
        assert!(sols.is_empty());
        assert_eq!(err.unwrap(), "my_ball");
    }

    #[test]
    fn catch_catches_matching_ball_and_runs_recovery() {
        let (sols, err) = run("catch(throw(oops(1)), oops(N), X = caught(N))");
        assert!(err.is_none());
        assert_eq!(sols, vec!["N=1,X=caught(1)"]);
    }

    #[test]
    fn catch_passes_nonmatching_ball_outward() {
        let (sols, err) = run("catch(throw(other), oops(_), X = no)");
        assert!(sols.is_empty());
        assert_eq!(err.unwrap(), "other");
    }

    #[test]
    fn nested_catch_inner_first() {
        let (sols, err) = run("catch(catch(throw(b), a, X = inner_a), b, X = outer_b)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["X=outer_b"]);
    }

    #[test]
    fn catch_traps_builtin_errors() {
        let (sols, err) = run("catch(X is 1 // 0, error(evaluation_error(E), _), Y = E)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["E=zero_divisor,X=_0,Y=zero_divisor"]);
    }

    #[test]
    fn catch_is_transparent_to_normal_backtracking() {
        let (sols, err) = run("catch((X = 1 ; X = 2), _, fail)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["X=1", "X=2"]);
    }

    #[test]
    fn step_limit_is_not_catchable() {
        let mut m = Machine::new(StringInterner::new(), Vec::new());
        m.step_limit = 1;
        m.steps = 1; // next step() trips the limit
        assert!(!m.step());
        let goal = parse_query(&mut m, "catch(true, _, true)").unwrap();
        // error pre-armed and uncatchable: solve returns Error untouched
        assert!(matches!(solve(&mut m, goal), Outcome::Error));
        assert!(m.error.as_ref().unwrap().uncatchable);
    }

    #[test]
    fn findall_collects_and_rewinds() {
        let (sols, err) = run("findall(X, (X = 1 ; X = 2 ; X = 3), L)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["L=[1, 2, 3],X=_0"]);
    }

    #[test]
    fn findall_empty_on_failure() {
        let (sols, err) = run("findall(X, fail, L)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["L=[],X=_0"]);
    }

    #[test]
    fn findall_propagates_errors() {
        let (sols, err) = run("findall(X, throw(bad), L)");
        assert!(sols.is_empty());
        assert_eq!(err.unwrap(), "bad");
    }

    #[test]
    fn nested_findall() {
        let (sols, err) = run("findall(L1, (Y = 2, findall(X, (X = 1 ; X = Y), L1)), L)");
        assert!(err.is_none());
        assert_eq!(sols.len(), 1);
        assert!(sols[0].contains("L=[[1, 2]]"), "{sols:?}");
    }

    #[test]
    fn call_n_extends_goals() {
        let (sols, err) = run("call(=, X, 7)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["X=7"]);
        let (sols, _) = run("G = (X = 1 ; X = 2), call(G)");
        assert_eq!(sols.len(), 2);
    }

    #[test]
    fn call_unbound_is_instantiation_error() {
        let (sols, err) = run("call(X)");
        assert!(sols.is_empty());
        assert!(err.unwrap().contains("instantiation_error"));
    }

    #[test]
    fn between_enumerates_and_checks() {
        let (sols, err) = run("between(1, 4, X)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["X=1", "X=2", "X=3", "X=4"]);
        let (sols, _) = run("between(1, 4, 3)");
        assert_eq!(sols.len(), 1);
        let (sols, _) = run("between(1, 4, 9)");
        assert!(sols.is_empty());
        let (sols, _) = run("between(3, 1, X)");
        assert!(sols.is_empty());
        // single-value range: no choice point churn
        let (sols, _) = run("between(2, 2, X)");
        assert_eq!(sols, vec!["X=2"]);
    }

    #[test]
    fn findall_with_between() {
        let (sols, err) = run("findall(X, between(1, 5, X), L)");
        assert!(err.is_none());
        assert_eq!(sols, vec!["L=[1, 2, 3, 4, 5],X=_0"]);
    }
}

#[cfg(test)]
mod vocab_invariant {
    //! Runtime half of the `plg-shared::builtins` invariant
    //! (docs/design/BUILTIN_VOCAB.md). `det_builtin` above is the
    //! query-side mirror of codegen's `DET_BUILTINS`; this asserts the
    //! arms it handles are EXACTLY the arity>0 `Det` rows of `BUILTINS`
    //! (`nl/0` is Det but dispatched via `try_atom_builtin`, so it is
    //! excluded here). Referenced only under `#[cfg(test)]` — the `doc`
    //! strings never reach a compiled program binary.
    use plg_shared::{BUILTINS, builtins::BuiltinKind};
    use std::collections::BTreeSet;

    /// Hand mirror of the `det_builtin` match arms. Adding an arm there
    /// without updating this (or `BUILTINS`) turns the test red.
    #[rustfmt::skip]
    const DET_DISPATCH: &[(&str, u32)] = &[
        ("var", 1), ("nonvar", 1), ("atom", 1), ("number", 1), ("integer", 1),
        ("float", 1), ("compound", 1), ("is_list", 1), ("functor", 3), ("arg", 3),
        ("=..", 2), ("copy_term", 2), ("atom_length", 2), ("atom_concat", 3),
        ("atom_chars", 2), ("number_chars", 2), ("number_codes", 2), ("msort", 2),
        ("sort", 2), ("succ", 2), ("plus", 3), ("unify_with_occurs_check", 2),
        ("write", 1), ("writeln", 1),
    ];

    #[test]
    fn det_dispatch_equals_shared_det_subset() {
        let dispatch: BTreeSet<(&str, u32)> = DET_DISPATCH.iter().copied().collect();
        let shared_det: BTreeSet<(&str, u32)> = BUILTINS
            .iter()
            .filter(|s| s.kind == BuiltinKind::Det && s.arity > 0)
            .map(|s| (s.name, s.arity))
            .collect();
        assert_eq!(
            dispatch, shared_det,
            "runtime det dispatch diverges from BUILTINS Det subset \
             (left = control.rs, right = shared table)"
        );
    }
}
