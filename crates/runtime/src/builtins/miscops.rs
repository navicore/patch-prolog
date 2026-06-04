//! Miscellaneous deterministic builtins: `succ/2`, `plus/3`,
//! `unify_with_occurs_check/2`, `write/1`, `writeln/1`, `nl/0`.
//!
//! Ported byte-for-byte from patch-prolog v1 (`solver.rs` arms,
//! `unify.rs` occurs-check unifier). Notes:
//! - `succ/2` both modes; `succ(X, 0)` fails; negatives raise
//!   `domain_error(not_less_than_zero, _)`; both-unbound raises
//!   instantiation.
//! - `plus/3` supports the three integer modes (any one argument
//!   unbound); fewer than two bound raises instantiation.
//! - `unify_with_occurs_check/2` uses a LOCAL occurs-checking unifier
//!   (iterative; does not touch the shared `unify.rs`).
//! - `write/1` / `writeln/1` use v1's `term_to_string` (infix operators,
//!   `[a, b]` lists, floats via `{}`), printed immediately. `write` adds
//!   no newline; `writeln` and `nl` add one.

use crate::cell::*;
use crate::machine::Machine;
use crate::render::term_to_string;
use crate::unify::unify;
use std::io::Write as _;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// Extract an i64 from an integer word (immediate or boxed).
fn int_of(m: &Machine, w: Word) -> Option<i64> {
    match tag_of(w) {
        TAG_INT => Some(int_value(w)),
        TAG_BIG => Some(m.heap[payload(w) as usize] as i64),
        _ => None,
    }
}

/// Materialize an i64 as a heap word (immediate INT or boxed BIG).
fn int_word(m: &mut Machine, n: i64) -> Word {
    if (INT_MIN..=INT_MAX).contains(&n) {
        make_int(n)
    } else {
        let idx = m.heap.len();
        m.heap.push(n as u64);
        make(TAG_BIG, idx as u64)
    }
}

/// `succ/2`: `S = X + 1` over non-negative integers, both modes.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_succ_2(m: *mut Machine, x: u64, s: u64) -> i32 {
    let m = mref(m);
    let wx = m.deref(x);
    let ws = m.deref(s);
    let xi = int_of(m, wx);
    let si = int_of(m, ws);
    match (xi, si) {
        (Some(xv), _) if xv >= 0 => match xv.checked_add(1) {
            Some(r) => {
                let rw = int_word(m, r);
                unify(m, s, rw) as i32
            }
            None => {
                crate::errors::evaluation(m, "int_overflow", "succ/2: integer overflow");
                0
            }
        },
        (_, Some(sv)) if sv > 0 => {
            let rw = int_word(m, sv - 1);
            unify(m, x, rw) as i32
        }
        (_, Some(0)) => 0, // succ(X, 0) fails
        (Some(_), _) => {
            // X is a negative integer.
            crate::errors::domain_error(
                m,
                "not_less_than_zero",
                wx,
                "succ/2: argument must be non-negative",
            );
            0
        }
        (_, Some(_)) => {
            // S is a negative integer.
            crate::errors::domain_error(
                m,
                "not_less_than_zero",
                ws,
                "succ/2: successor must be non-negative",
            );
            0
        }
        _ => {
            crate::errors::instantiation(m, "succ/2: at least one argument must be an integer");
            0
        }
    }
}

/// `plus/3`: `Z = X + Y` over integers; any single unbound is solved for.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_plus_3(m: *mut Machine, x: u64, y: u64, z: u64) -> i32 {
    let m = mref(m);
    let wx = int_of(m, m.deref(x));
    let wy = int_of(m, m.deref(y));
    let wz = int_of(m, m.deref(z));
    let overflow = |m: &mut Machine| {
        crate::errors::evaluation(m, "int_overflow", "plus/3: integer overflow");
        0
    };
    match (wx, wy, wz) {
        (Some(xv), Some(yv), _) => match xv.checked_add(yv) {
            Some(r) => {
                let rw = int_word(m, r);
                unify(m, z, rw) as i32
            }
            None => overflow(m),
        },
        (Some(xv), _, Some(zv)) => match zv.checked_sub(xv) {
            Some(r) => {
                let rw = int_word(m, r);
                unify(m, y, rw) as i32
            }
            None => overflow(m),
        },
        (_, Some(yv), Some(zv)) => match zv.checked_sub(yv) {
            Some(r) => {
                let rw = int_word(m, r);
                unify(m, x, rw) as i32
            }
            None => overflow(m),
        },
        _ => {
            crate::errors::instantiation(m, "plus/3: at least two arguments must be integers");
            0
        }
    }
}

/// `unify_with_occurs_check/2`: like `=/2` but fails rather than build a
/// cyclic term. Local iterative implementation — bindings are trailed so
/// the caller's choice-point rewind undoes a partial unification.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_unify_with_occurs_check_2(m: *mut Machine, a: u64, b: u64) -> i32 {
    let m = mref(m);
    unify_oc(m, a, b) as i32
}

/// Iterative occurs-checking unification over tagged heap words.
fn unify_oc(m: &mut Machine, a: Word, b: Word) -> bool {
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        let a = m.deref(a);
        let b = m.deref(b);
        if a == b {
            continue;
        }
        match (tag_of(a), tag_of(b)) {
            (TAG_REF, _) => {
                if occurs(m, payload(a) as usize, b) {
                    return false;
                }
                m.bind(payload(a) as usize, b);
            }
            (_, TAG_REF) => {
                if occurs(m, payload(b) as usize, a) {
                    return false;
                }
                m.bind(payload(b) as usize, a);
            }
            (TAG_ATOM, TAG_ATOM) | (TAG_INT, TAG_INT) => return false,
            (TAG_BIG, TAG_BIG) => {
                if m.heap[payload(a) as usize] as i64 != m.heap[payload(b) as usize] as i64 {
                    return false;
                }
            }
            (TAG_INT, TAG_BIG) => {
                if int_value(a) != m.heap[payload(b) as usize] as i64 {
                    return false;
                }
            }
            (TAG_BIG, TAG_INT) => {
                if m.heap[payload(a) as usize] as i64 != int_value(b) {
                    return false;
                }
            }
            (TAG_FLT, TAG_FLT) => {
                if m.heap[payload(a) as usize] != m.heap[payload(b) as usize] {
                    return false;
                }
            }
            (TAG_STR, TAG_STR) => {
                let ia = payload(a) as usize;
                let ib = payload(b) as usize;
                let (fa, na) = unpack_functor(m.heap[ia]);
                let (fb, nb) = unpack_functor(m.heap[ib]);
                if fa != fb || na != nb {
                    return false;
                }
                for k in 0..na as usize {
                    work.push((m.heap[ia + 1 + k], m.heap[ib + 1 + k]));
                }
            }
            (TAG_LST, TAG_LST) => {
                let ia = payload(a) as usize;
                let ib = payload(b) as usize;
                work.push((m.heap[ia + 1], m.heap[ib + 1]));
                work.push((m.heap[ia], m.heap[ib]));
            }
            _ => return false,
        }
    }
    true
}

/// Does the variable at heap index `var` occur within `term`? Iterative
/// walk following bound refs; structures and lists are descended.
fn occurs(m: &Machine, var: usize, term: Word) -> bool {
    let mut work = vec![term];
    while let Some(w) = work.pop() {
        let w = m.deref(w);
        match tag_of(w) {
            TAG_REF if payload(w) as usize == var => return true,
            TAG_REF => {}
            TAG_STR => {
                let idx = payload(w) as usize;
                let (_, n) = unpack_functor(m.heap[idx]);
                for k in 0..n as usize {
                    work.push(m.heap[idx + 1 + k]);
                }
            }
            TAG_LST => {
                let idx = payload(w) as usize;
                work.push(m.heap[idx]);
                work.push(m.heap[idx + 1]);
            }
            _ => {}
        }
    }
    false
}

/// `write/1`: print the term (v1 `term_to_string`), no trailing newline.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_write_1(m: *mut Machine, term: u64) -> i32 {
    let m = mref(m);
    let s = term_to_string(m, term);
    print!("{s}");
    let _ = std::io::stdout().flush();
    1
}

/// `writeln/1`: `write/1` followed by a newline.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_writeln_1(m: *mut Machine, term: u64) -> i32 {
    let m = mref(m);
    let s = term_to_string(m, term);
    println!("{s}");
    1
}

/// `nl/0`: print a newline. Always succeeds.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_nl_0(_m: *mut Machine) -> i32 {
    println!();
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn msg(m: &Machine) -> &str {
        m.error.as_ref().unwrap().message.as_str()
    }

    fn str_term(m: &mut Machine, name: &str, args: &[Word]) -> Word {
        let f = m.atoms.intern(name);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, args.len() as u32));
        m.heap.extend_from_slice(args);
        make(TAG_STR, idx as u64)
    }

    #[test]
    fn succ_both_modes() {
        let mut m = machine();
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_succ_2(mp, make_int(3), x), 1);
        assert_eq!(int_value(m.deref(x)), 4);

        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_succ_2(mp, y, make_int(5)), 1);
        assert_eq!(int_value(m.deref(y)), 4);
    }

    #[test]
    fn succ_zero_and_negative() {
        let mut m = machine();
        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        // succ(X, 0) fails (no predecessor)
        assert_eq!(plg_rt_b_succ_2(mp, y, make_int(0)), 0);
        assert!(m.error.is_none());

        // succ(-1, X) → domain_error
        let mut m = machine();
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_succ_2(mp, make_int(-1), x), 0);
        assert_eq!(
            msg(&m),
            "error(domain_error(not_less_than_zero, -1), succ/2: argument must be non-negative)"
        );

        // succ(X, Y) both unbound → instantiation
        let mut m = machine();
        let x = m.new_var();
        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_succ_2(mp, x, y), 0);
        assert_eq!(
            msg(&m),
            "error(instantiation_error, succ/2: at least one argument must be an integer)"
        );
    }

    #[test]
    fn plus_three_modes_and_error() {
        let mut m = machine();
        let z = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_plus_3(mp, make_int(2), make_int(3), z), 1);
        assert_eq!(int_value(m.deref(z)), 5);

        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_plus_3(mp, make_int(2), y, make_int(5)), 1);
        assert_eq!(int_value(m.deref(y)), 3);

        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_plus_3(mp, x, make_int(3), make_int(5)), 1);
        assert_eq!(int_value(m.deref(x)), 2);

        // two unbound → instantiation
        let mut m = machine();
        let x = m.new_var();
        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_plus_3(mp, x, y, make_int(5)), 0);
        assert_eq!(
            msg(&m),
            "error(instantiation_error, plus/3: at least two arguments must be integers)"
        );
    }

    #[test]
    fn occurs_check_rejects_cycle() {
        let mut m = machine();
        let x = m.new_var();
        // unify_with_occurs_check(X, f(X)) must FAIL.
        let fx = str_term(&mut m, "f", &[x]);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_unify_with_occurs_check_2(mp, x, fx), 0);
        assert!(m.error.is_none());
        // X still unbound (no binding committed).
        assert_eq!(m.deref(x), x);
    }

    #[test]
    fn occurs_check_allows_acyclic() {
        let mut m = machine();
        let x = m.new_var();
        let a = make_atom(m.atoms.intern("a"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_unify_with_occurs_check_2(mp, x, a), 1);
        assert_eq!(m.deref(x), a);

        // structural unify with shared subterm but no cycle
        let mut m = machine();
        let y = m.new_var();
        let s1 = str_term(&mut m, "g", &[y, make_int(1)]);
        let s2 = str_term(&mut m, "g", &[make_int(2), make_int(1)]);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_unify_with_occurs_check_2(mp, s1, s2), 1);
        assert_eq!(int_value(m.deref(y)), 2);
    }

    #[test]
    fn nl_always_succeeds() {
        let mut m = machine();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_nl_0(mp), 1);
    }

    #[test]
    fn write_returns_success() {
        // We can't easily capture stdout here, but the call must succeed
        // and not mutate the heap-visible term.
        let mut m = machine();
        let s = str_term(&mut m, "+", &[make_int(1), make_int(2)]);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_write_1(mp, s), 1);
        assert_eq!(plg_rt_b_writeln_1(mp, s), 1);
        // sanity: rendering matches v1 infix form
        assert_eq!(term_to_string(&m, s), "1 + 2");
    }
}
