//! Generic unification over tagged heap words.
//!
//! Ports patch-prolog's `unify.rs` semantics to the cell heap: no occurs
//! check (ISO 8.3.2), trail-recorded bindings, float equality via
//! `to_bits` (NaN unifies with NaN). Iterative worklist — no recursion,
//! so pathological terms can't blow the C stack.

use crate::cell::*;
use crate::machine::Machine;

pub fn unify(m: &mut Machine, a: Word, b: Word) -> bool {
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        let a = m.deref(a);
        let b = m.deref(b);
        if a == b {
            continue; // same atom/int word, same cell, or same structure
        }
        match (tag_of(a), tag_of(b)) {
            (TAG_REF, _) => m.bind(payload(a) as usize, b),
            (_, TAG_REF) => m.bind(payload(b) as usize, a),
            (TAG_ATOM, TAG_ATOM) | (TAG_INT, TAG_INT) => return false, // a != b
            (TAG_FLT, TAG_FLT) => {
                // to_bits comparison: NaN == NaN, -0.0 != 0.0 (v1 rule)
                let fa = m.heap[payload(a) as usize];
                let fb = m.heap[payload(b) as usize];
                if fa != fb {
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
                work.push((m.heap[ia + 1], m.heap[ib + 1])); // tails
                work.push((m.heap[ia], m.heap[ib])); // heads
            }
            _ => return false, // different tags never unify
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn put_struct(m: &mut Machine, f: u32, args: &[Word]) -> Word {
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, args.len() as u32));
        m.heap.extend_from_slice(args);
        make(TAG_STR, idx as u64)
    }

    fn put_list(m: &mut Machine, head: Word, tail: Word) -> Word {
        let idx = m.heap.len();
        m.heap.push(head);
        m.heap.push(tail);
        make(TAG_LST, idx as u64)
    }

    #[test]
    fn atoms_unify_only_with_same_id() {
        let mut m = machine();
        assert!(unify(&mut m, make_atom(1), make_atom(1)));
        assert!(!unify(&mut m, make_atom(1), make_atom(2)));
    }

    #[test]
    fn var_binds_to_atom_and_is_trailed() {
        let mut m = machine();
        let v = m.new_var();
        let t0 = m.trail.len();
        assert!(unify(&mut m, v, make_atom(3)));
        assert_eq!(m.deref(v), make_atom(3));
        assert_eq!(m.trail.len(), t0 + 1);
    }

    #[test]
    fn var_var_aliasing() {
        let mut m = machine();
        let x = m.new_var();
        let y = m.new_var();
        assert!(unify(&mut m, x, y));
        assert!(unify(&mut m, x, make_int(9)));
        assert_eq!(int_value(m.deref(y)), 9);
    }

    #[test]
    fn structs_unify_recursively() {
        let mut m = machine();
        let x = m.new_var();
        // f(a, X) = f(a, 42)
        let s1 = put_struct(&mut m, 5, &[make_atom(1), x]);
        let s2 = put_struct(&mut m, 5, &[make_atom(1), make_int(42)]);
        assert!(unify(&mut m, s1, s2));
        assert_eq!(int_value(m.deref(x)), 42);
        // arity mismatch
        let s3 = put_struct(&mut m, 5, &[make_atom(1)]);
        assert!(!unify(&mut m, s1, s3));
        // functor mismatch
        let s4 = put_struct(&mut m, 6, &[make_atom(1), make_int(42)]);
        assert!(!unify(&mut m, s1, s4));
    }

    #[test]
    fn lists_unify_elementwise() {
        let mut m = machine();
        let x = m.new_var();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        // [1|X] = [1, 2]
        let l2 = {
            let inner = put_list(&mut m, make_int(2), nil);
            put_list(&mut m, make_int(1), inner)
        };
        let l1 = put_list(&mut m, make_int(1), x);
        assert!(unify(&mut m, l1, l2));
        // X is now [2]
        let xd = m.deref(x);
        assert_eq!(tag_of(xd), TAG_LST);
    }

    #[test]
    fn nan_unifies_with_nan() {
        let mut m = machine();
        let bits = f64::NAN.to_bits();
        let f1 = {
            let i = m.heap.len();
            m.heap.push(bits);
            make(TAG_FLT, i as u64)
        };
        let f2 = {
            let i = m.heap.len();
            m.heap.push(bits);
            make(TAG_FLT, i as u64)
        };
        assert!(unify(&mut m, f1, f2));
    }

    #[test]
    fn failed_unify_leaves_partial_bindings_for_caller_rewind() {
        // The engine relies on choice-point rewind (not unify itself) to
        // undo partial bindings — same as v1. Verify the trail records them.
        let mut m = machine();
        let x = m.new_var();
        let s1 = put_struct(&mut m, 5, &[x, make_atom(1)]);
        let s2 = put_struct(&mut m, 5, &[make_atom(2), make_atom(9)]);
        let tmark = m.trail.len();
        let hmark = m.heap.len();
        assert!(!unify(&mut m, s1, s2));
        m.rewind_to(tmark, hmark);
        assert_eq!(m.deref(x), x);
    }
}
