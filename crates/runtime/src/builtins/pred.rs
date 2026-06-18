//! C-ABI surface for the M3 builtins: arithmetic (`is/2`, comparisons),
//! term comparison (`==`, `@<`, `compare/3`), structural inequality
//! (`\=/2`), cut, and a few codegen helpers. Mirrors the style of
//! `abi.rs` (`#[unsafe(no_mangle)] pub extern "C" fn plg_rt_*`).

use crate::builtins::arith::{self, ArithValue};
use crate::builtins::order::compare_terms;
use crate::cell::{self, INT_MAX, INT_MIN, Word};
use crate::machine::Machine;
use std::cmp::Ordering;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// Materialize an evaluated value as a heap word.
///
/// Integers that fit the i61 immediate become an INT word. An integer that
/// fits i64 but NOT i61 cannot be boxed until M4; for M3 we raise an error
/// (see the deviation note in the milestone report). Floats are boxed via a
/// FLT cell, like `plg_rt_put_float`.
fn value_to_word(m: &mut Machine, v: ArithValue) -> Option<Word> {
    match v {
        ArithValue::Int(n) => {
            if (INT_MIN..=INT_MAX).contains(&n) {
                Some(cell::make_int(n))
            } else {
                // M4: full i64 range via a boxed BIG cell (v1 parity).
                let idx = m.heap.len();
                m.heap.push(n as u64);
                Some(cell::make(cell::TAG_BIG, idx as u64))
            }
        }
        ArithValue::Float(f) => {
            let idx = m.heap.len();
            m.heap.push(f.to_bits());
            Some(cell::make(cell::TAG_FLT, idx as u64))
        }
    }
}

/// `is/2`: evaluate `expr`, unify `lhs` with the result. 1 = success,
/// 0 = failure or error (error already set in `m.error`). `site_id` carries
/// source provenance (SPANS.md Layer 3): set around the eval so any error
/// constructor it reaches appends ` at file:line:col` via `set_formal`.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_is(m: *mut Machine, lhs: u64, expr: u64, site_id: u32) -> i32 {
    let m = mref(m);
    m.error_site = site_id;
    let r = match arith::eval(m, expr) {
        Err(()) => 0,
        Ok(v) => match value_to_word(m, v) {
            None => 0,
            Some(w) => crate::unify::unify(m, lhs, w) as i32,
        },
    };
    m.error_site = crate::machine::NO_SITE;
    r
}

/// Arithmetic comparison. op: 0:'<' 1:'>' 2:'=<' 3:'>=' 4:'=:=' 5:'=\\='.
/// 1 = holds, 0 = does not hold or error. `site_id`: see `plg_rt_b_is`.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_arith_cmp(
    m: *mut Machine,
    op: i32,
    a: u64,
    b: u64,
    site_id: u32,
) -> i32 {
    let m = mref(m);
    m.error_site = site_id;
    let r = 'cmp: {
        let av = match arith::eval(m, a) {
            Ok(v) => v,
            Err(()) => break 'cmp 0,
        };
        let bv = match arith::eval(m, b) {
            Ok(v) => v,
            Err(()) => break 'cmp 0,
        };
        let holds = match op {
            0 => arith::arith_lt(av, bv),
            1 => arith::arith_gt(av, bv),
            2 => !arith::arith_gt(av, bv), // =< : not greater
            3 => !arith::arith_lt(av, bv), // >= : not less
            4 => arith::arith_eq(av, bv),  // =:=
            5 => !arith::arith_eq(av, bv), // =\=
            _ => break 'cmp 0,
        };
        holds as i32
    };
    m.error_site = crate::machine::NO_SITE;
    r
}

/// `\=/2`: succeed iff `a` and `b` do NOT unify. Always undoes any bindings
/// made during the trial unification (rewinds the trail to the entry mark);
/// the heap top is left untouched (no terms are constructed here).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_neq(m: *mut Machine, a: u64, b: u64) -> i32 {
    let m = mref(m);
    let trail_mark = m.trail.len();
    let unified = crate::unify::unify(m, a, b);
    // Undo bindings unconditionally (do not truncate the heap).
    while m.trail.len() > trail_mark {
        let idx = m.trail.pop().unwrap() as usize;
        m.heap[idx] = cell::make_ref(idx);
    }
    (!unified) as i32
}

/// Term comparison via standard order. op: 0:'==' 1:'\\==' 2:'@<' 3:'@>'
/// 4:'@=<' 5:'@>='. 1 = holds, 0 = does not.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_term_cmp(m: *mut Machine, op: i32, a: u64, b: u64) -> i32 {
    let m = mref(m);
    let ord = compare_terms(m, a, b);
    let holds = match op {
        0 => ord == Ordering::Equal,   // ==
        1 => ord != Ordering::Equal,   // \==
        2 => ord == Ordering::Less,    // @<
        3 => ord == Ordering::Greater, // @>
        4 => ord != Ordering::Greater, // @=<
        5 => ord != Ordering::Less,    // @>=
        _ => return 0,
    };
    holds as i32
}

/// `compare/3`: unify `order` with the atom '<', '=', or '>' according to
/// the standard order of `a` and `b`. 1 = success, 0 = failure.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_compare(m: *mut Machine, order: u64, a: u64, b: u64) -> i32 {
    let m = mref(m);
    let name = match compare_terms(m, a, b) {
        Ordering::Less => "<",
        Ordering::Equal => "=",
        Ordering::Greater => ">",
    };
    let id = m.atoms.intern(name);
    crate::unify::unify(m, order, cell::make_atom(id)) as i32
}

/// Cut: truncate the choice-point stack to `height`.
///
/// M4 will make this catch-frame aware (a cut must not discard a CATCH
/// barrier installed by `catch/3`); for M3 it is a plain truncate, matching
/// the absence of exception frames in the current runtime.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_cut(m: *mut Machine, height: u64) {
    // Stops at catch frames (v1 rule: catch/3 is opaque to cut).
    mref(m).cut_to(height as usize);
}

/// Current choice-point stack height (the cut barrier to capture on entry).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_cp_top(m: *mut Machine) -> u64 {
    mref(m).cps.len() as u64
}

/// Dereference a word through REF chains (exposes `Machine::deref`).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_deref(m: *mut Machine, w: u64) -> u64 {
    mref(m).deref(w)
}

/// First-argument indexing key for a DEREFED word: for an STR, the packed
/// functor cell (`heap[idx]`, i.e. `pack_functor(functor, arity)`); for
/// anything else, `u64::MAX`. Codegen switches on this to pick a clause set.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_str_key(m: *mut Machine, w: u64) -> u64 {
    let m = mref(m);
    if cell::tag_of(w) == cell::TAG_STR {
        m.heap[cell::payload(w) as usize]
    } else {
        u64::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    // These exercise arithmetic, not provenance; call with no site.
    fn is_(m: *mut Machine, lhs: u64, expr: u64) -> i32 {
        plg_rt_b_is(m, lhs, expr, crate::machine::NO_SITE)
    }
    fn cmp_(m: *mut Machine, op: i32, a: u64, b: u64) -> i32 {
        plg_rt_b_arith_cmp(m, op, a, b, crate::machine::NO_SITE)
    }

    fn bin_str(m: &mut Machine, op: &str, a: Word, b: Word) -> Word {
        let f = m.atoms.intern(op);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, 2));
        m.heap.push(a);
        m.heap.push(b);
        make(TAG_STR, idx as u64)
    }

    #[test]
    fn b_is_unifies_bound_lhs() {
        let mut m = machine();
        let mp = &mut *m as *mut Machine;
        // 5 is 2+3 → success
        let e = bin_str(&mut m, "+", make_int(2), make_int(3));
        assert_eq!(is_(mp, make_int(5), e), 1);
        // 6 is 2+3 → fail
        let e = bin_str(&mut m, "+", make_int(2), make_int(3));
        assert_eq!(is_(mp, make_int(6), e), 0);
    }

    #[test]
    fn b_is_binds_unbound_lhs() {
        let mut m = machine();
        let v = m.new_var();
        let e = bin_str(&mut m, "*", make_int(4), make_int(5));
        let mp = &mut *m as *mut Machine;
        assert_eq!(is_(mp, v, e), 1);
        assert_eq!(int_value(m.deref(v)), 20);
    }

    #[test]
    fn b_is_boxes_float() {
        let mut m = machine();
        let v = m.new_var();
        // 2.0 yields a float via /(1.0)... use 1/2 = 0.5 (always float).
        let e = bin_str(&mut m, "/", make_int(1), make_int(2));
        let mp = &mut *m as *mut Machine;
        assert_eq!(is_(mp, v, e), 1);
        let d = m.deref(v);
        assert_eq!(tag_of(d), TAG_FLT);
        assert_eq!(f64::from_bits(m.heap[payload(d) as usize]), 0.5);
    }

    #[test]
    fn b_is_boxes_big_integers() {
        // M4: results beyond the i61 immediate box to a BIG cell — the
        // full i64 range works, matching v1.
        let mut m = machine();
        let v = m.new_var();
        let big = INT_MAX; // 2^60 - 1, the largest immediate
        let e = bin_str(&mut m, "+", make_int(big), make_int(big));
        let mp = &mut *m as *mut Machine;
        assert_eq!(is_(mp, v, e), 1);
        assert!(m.error.is_none());
        let d = m.deref(v);
        assert_eq!(tag_of(d), TAG_BIG);
        assert_eq!(m.heap[payload(d) as usize] as i64, 2 * big);
    }

    #[test]
    fn arith_cmp_ops() {
        let mut m = machine();
        let mp = &mut *m as *mut Machine;
        assert_eq!(cmp_(mp, 0, make_int(1), make_int(2)), 1); // <
        assert_eq!(cmp_(mp, 0, make_int(2), make_int(1)), 0);
        assert_eq!(cmp_(mp, 1, make_int(2), make_int(1)), 1); // >
        assert_eq!(cmp_(mp, 2, make_int(1), make_int(1)), 1); // =<
        assert_eq!(cmp_(mp, 3, make_int(1), make_int(1)), 1); // >=
        assert_eq!(cmp_(mp, 4, make_int(3), make_int(3)), 1); // =:=
        assert_eq!(cmp_(mp, 5, make_int(3), make_int(4)), 1); // =\=
    }

    #[test]
    fn arith_cmp_mixed_int_float() {
        // 1 =:= 1.0 holds; 1.0 < 1 does not.
        let mut m = machine();
        let one_f = {
            let idx = m.heap.len();
            m.heap.push(1.0f64.to_bits());
            make(TAG_FLT, idx as u64)
        };
        let mp = &mut *m as *mut Machine;
        assert_eq!(cmp_(mp, 4, make_int(1), one_f), 1);
        assert_eq!(cmp_(mp, 0, one_f, make_int(1)), 0);
    }

    #[test]
    fn arith_cmp_propagates_error() {
        let mut m = machine();
        let v = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(cmp_(mp, 0, make_int(1), v), 0);
        assert!(m.error.is_some());
    }

    #[test]
    fn b_neq_undoes_bindings() {
        let mut m = machine();
        let x = m.new_var();
        let t0 = m.trail.len();
        let h0 = m.heap.len();
        let mp = &mut *m as *mut Machine;
        // X \= a : X and a unify, so \= fails (0), and X is left unbound.
        assert_eq!(plg_rt_b_neq(mp, x, make_atom(7)), 0);
        assert_eq!(m.deref(x), x, "binding undone");
        assert_eq!(m.trail.len(), t0, "trail rewound");
        assert_eq!(m.heap.len(), h0, "heap top untouched");
        // a \= b : distinct atoms do not unify → succeeds (1).
        assert_eq!(plg_rt_b_neq(mp, make_atom(7), make_atom(8)), 1);
    }

    #[test]
    fn term_cmp_ops() {
        let mut m = machine();
        let a = m.atoms.intern("a");
        let b = m.atoms.intern("b");
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_term_cmp(mp, 0, make_atom(a), make_atom(a)), 1); // ==
        assert_eq!(plg_rt_b_term_cmp(mp, 1, make_atom(a), make_atom(b)), 1); // \==
        assert_eq!(plg_rt_b_term_cmp(mp, 2, make_atom(a), make_atom(b)), 1); // @<
        assert_eq!(plg_rt_b_term_cmp(mp, 3, make_atom(b), make_atom(a)), 1); // @>
        assert_eq!(plg_rt_b_term_cmp(mp, 4, make_atom(a), make_atom(a)), 1); // @=<
        assert_eq!(plg_rt_b_term_cmp(mp, 5, make_atom(a), make_atom(a)), 1); // @>=
    }

    #[test]
    fn compare3_binds_order_atom() {
        let mut m = machine();
        let o = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_compare(mp, o, make_int(1), make_int(2)), 1);
        let lt = m.atoms.lookup("<").unwrap();
        assert_eq!(m.deref(o), make_atom(lt));

        let o2 = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_compare(mp, o2, make_int(5), make_int(5)), 1);
        let eq = m.atoms.lookup("=").unwrap();
        assert_eq!(m.deref(o2), make_atom(eq));
    }

    #[test]
    fn cut_truncates_cps() {
        let mut m = machine();
        // push three dummy choice points
        unsafe extern "C" fn dummy(_m: *mut Machine, _e: u64) -> i32 {
            0
        }
        for _ in 0..3 {
            m.push_cp(dummy, 0);
        }
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_cp_top(mp), 3);
        plg_rt_cut(mp, 1);
        assert_eq!(plg_rt_cp_top(mp), 1);
    }

    #[test]
    fn deref_and_str_key() {
        let mut m = machine();
        let f = m.atoms.intern("foo");
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, 2));
        m.heap.push(make_int(1));
        m.heap.push(make_int(2));
        let s = make(TAG_STR, idx as u64);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_str_key(mp, s), pack_functor(f, 2));
        assert_eq!(plg_rt_str_key(mp, make_int(9)), u64::MAX);
        assert_eq!(plg_rt_str_key(mp, make_atom(f)), u64::MAX);

        // deref follows a bound var chain
        let mut m = machine();
        let v = m.new_var();
        m.bind(payload(v) as usize, make_int(42));
        let mp = &mut *m as *mut Machine;
        assert_eq!(int_value(plg_rt_deref(mp, v)), 42);
    }
}
