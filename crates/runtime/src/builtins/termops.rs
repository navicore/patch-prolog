//! Term-introspection builtins: `functor/3`, `arg/3`, `=../2` (univ),
//! `copy_term/2`.
//!
//! Tag decisions:
//! - `functor(T, '.', 2)` and `T =.. ['.', a, b]` both build a **STR**
//!   (functor ".", arity 2), NOT a list cell.
//! - decomposing a `TAG_LST` yields functor `.` / arity 2 and args
//!   [Head, Tail] (univ produces `['.', H, T]`).
//! - errors are structured balls (see the captured strings in the unit
//!   tests).

use crate::cell::*;
use crate::machine::Machine;
use crate::unify::unify;
use plg_shared::atom::ATOM_DOT;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// Build `[e0, e1, ...]` on the heap, nil-terminated; return its word.
fn build_list(m: &mut Machine, elems: &[Word]) -> Word {
    let mut tail = make_atom(plg_shared::atom::ATOM_NIL);
    for &e in elems.iter().rev() {
        let idx = m.heap.len();
        m.heap.push(e);
        m.heap.push(tail);
        tail = make(TAG_LST, idx as u64);
    }
    tail
}

/// Collect a proper list's elements; `None` if not nil-terminated.
fn collect_list(m: &Machine, w: Word) -> Option<Vec<Word>> {
    let mut out = Vec::new();
    let mut cur = m.deref(w);
    loop {
        match tag_of(cur) {
            TAG_ATOM if atom_id(cur) == plg_shared::atom::ATOM_NIL => return Some(out),
            TAG_LST => {
                let idx = payload(cur) as usize;
                out.push(m.heap[idx]);
                cur = m.deref(m.heap[idx + 1]);
            }
            _ => return None,
        }
    }
}

/// `functor/3`: decompose or construct. 1 = success, 0 = failure/error.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_functor_3(
    m: *mut Machine,
    term: u64,
    name: u64,
    arity: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(term);
    match tag_of(w) {
        TAG_ATOM => {
            let ok = unify(m, name, w) && unify(m, arity, make_int(0));
            ok as i32
        }
        TAG_INT | TAG_BIG | TAG_FLT => {
            let ok = unify(m, name, w) && unify(m, arity, make_int(0));
            ok as i32
        }
        TAG_STR => {
            let idx = payload(w) as usize;
            let (f, n) = unpack_functor(m.heap[idx]);
            let ok = unify(m, name, make_atom(f)) && unify(m, arity, make_int(n as i64));
            ok as i32
        }
        TAG_LST => {
            // Lists are ./2.
            let ok = unify(m, name, make_atom(ATOM_DOT)) && unify(m, arity, make_int(2));
            ok as i32
        }
        TAG_REF => functor_construct(m, term, name, arity),
        _ => 0,
    }
}

/// `functor/3` with an unbound first argument: build a term from
/// name + arity.
fn functor_construct(m: &mut Machine, term: u64, name: u64, arity: u64) -> i32 {
    let wname = m.deref(name);
    let warity = m.deref(arity);
    // Arity must be a bound integer to proceed.
    let arity_val = match tag_of(warity) {
        TAG_INT => int_value(warity),
        TAG_BIG => m.heap[payload(warity) as usize] as i64,
        _ => {
            crate::errors::instantiation(m, "functor/3: insufficient arguments");
            return 0;
        }
    };
    // arity 0: the term is the name itself (atom/number).
    if arity_val == 0 {
        match tag_of(wname) {
            TAG_ATOM | TAG_INT | TAG_BIG | TAG_FLT => return unify(m, term, wname) as i32,
            _ => {
                crate::errors::instantiation(m, "functor/3: insufficient arguments");
                return 0;
            }
        }
    }
    if arity_val < 0 {
        // The culprit is the negative arity itself.
        crate::errors::domain_error(
            m,
            "not_less_than_zero",
            warity,
            "functor/3: arity must be non-negative",
        );
        return 0;
    }
    if arity_val > 1024 {
        // representation_error(max_arity)
        let re = m.atoms.intern("representation_error");
        let flag = make_atom(m.atoms.intern("max_arity"));
        let idx = m.heap.len();
        m.heap.push(pack_functor(re, 1));
        m.heap.push(flag);
        crate::errors::set_formal(
            m,
            make(TAG_STR, idx as u64),
            "functor/3: arity too large (max 1024)",
            false,
        );
        return 0;
    }
    // arity > 0: name must be an atom; build name(_,_,...).
    if tag_of(wname) != TAG_ATOM {
        crate::errors::instantiation(m, "functor/3: insufficient arguments");
        return 0;
    }
    let f = atom_id(wname);
    let n = arity_val as u32;
    let base = m.heap.len();
    m.heap.push(pack_functor(f, n));
    // Each argument slot must be its OWN fresh variable. Do NOT use
    // `m.new_var()` here: it pushes a self-ref cell *and* returns a ref to it,
    // so pushing that ref would lay down two cells per slot and make every
    // later arg alias the first variable (issue #31). Write a self-referencing
    // REF directly into each contiguous arg cell instead.
    for _ in 0..n {
        let idx = m.heap.len();
        m.heap.push(make_ref(idx));
    }
    let constructed = make(TAG_STR, base as u64);
    unify(m, term, constructed) as i32
}

/// `arg/3`: the N-th argument of a compound. 1 = success, 0 = fail/error.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_arg_3(
    m: *mut Machine,
    n: u64,
    term: u64,
    result: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let wn = m.deref(n);
    let n_val = match tag_of(wn) {
        TAG_INT => int_value(wn),
        TAG_BIG => m.heap[payload(wn) as usize] as i64,
        _ => {
            crate::errors::type_error(m, "integer", wn, "arg/3: first argument must be integer");
            return 0;
        }
    };
    let wt = m.deref(term);
    match tag_of(wt) {
        TAG_STR => {
            let idx = payload(wt) as usize;
            let (_, arity) = unpack_functor(m.heap[idx]);
            if n_val >= 1 && (n_val as u64) <= arity as u64 {
                let arg = m.heap[idx + n_val as usize];
                unify(m, result, arg) as i32
            } else {
                0 // out of range → fail
            }
        }
        TAG_LST => {
            let idx = payload(wt) as usize;
            match n_val {
                1 => unify(m, result, m.heap[idx]) as i32,
                2 => unify(m, result, m.heap[idx + 1]) as i32,
                _ => 0,
            }
        }
        _ => {
            crate::errors::type_error(m, "compound", wt, "arg/3: second argument must be compound");
            0
        }
    }
}

/// `=../2` (univ): decompose into / build from a list. 1 = success.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_univ_2(m: *mut Machine, term: u64, list: u64, site_id: u32) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(term);
    match tag_of(w) {
        TAG_REF => univ_construct(m, term, list),
        TAG_ATOM | TAG_INT | TAG_BIG | TAG_FLT => {
            let lst = build_list(m, &[w]);
            unify(m, list, lst) as i32
        }
        TAG_STR => {
            let idx = payload(w) as usize;
            let (f, n) = unpack_functor(m.heap[idx]);
            let mut elems = Vec::with_capacity(n as usize + 1);
            elems.push(make_atom(f));
            for i in 0..n as usize {
                elems.push(m.heap[idx + 1 + i]);
            }
            let lst = build_list(m, &elems);
            unify(m, list, lst) as i32
        }
        TAG_LST => {
            let idx = payload(w) as usize;
            let head = m.heap[idx];
            let tail = m.heap[idx + 1];
            let lst = build_list(m, &[make_atom(ATOM_DOT), head, tail]);
            unify(m, list, lst) as i32
        }
        _ => 0,
    }
}

/// `=../2` with an unbound first argument: build a term from the list.
fn univ_construct(m: &mut Machine, term: u64, list: u64) -> i32 {
    let Some(elems) = collect_list(m, list) else {
        let culprit = m.deref(list);
        crate::errors::type_error(m, "list", culprit, "=../2: second argument must be a list");
        return 0;
    };
    if elems.is_empty() {
        let culprit = m.deref(list);
        crate::errors::domain_error(
            m,
            "non_empty_list",
            culprit,
            "=../2: list must not be empty",
        );
        return 0;
    }
    let head = m.deref(elems[0]);
    if elems.len() == 1 {
        // Single element: term unifies with it directly (atom/number).
        if tag_of(head) == TAG_REF {
            crate::errors::instantiation(m, "=../2: instantiation error - element must be bound");
            return 0;
        }
        return unify(m, term, head) as i32;
    }
    // arity > 0: the functor must be an atom; build a STR.
    if tag_of(head) != TAG_ATOM {
        crate::errors::type_error(
            m,
            "atom",
            head,
            "=../2: functor must be an atom when arity > 0",
        );
        return 0;
    }
    let f = atom_id(head);
    let n = (elems.len() - 1) as u32;
    let base = m.heap.len();
    m.heap.push(pack_functor(f, n));
    for &e in &elems[1..] {
        m.heap.push(e);
    }
    let constructed = make(TAG_STR, base as u64);
    unify(m, term, constructed) as i32
}

/// `copy_term/2`: a fresh copy of `orig` with consistent renamed vars.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_copy_term_2(m: *mut Machine, orig: u64, copy: u64) -> i32 {
    let m = mref(m);
    let buf = crate::copyterm::copy_to_buf(m, orig);
    let fresh = crate::copyterm::restore_from_buf(m, &buf);
    unify(m, copy, fresh) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::NO_SITE;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    // Thin wrappers so the existing tests need no site (they exercise
    // behavior, not provenance).
    fn functor3(m: *mut Machine, t: u64, n: u64, a: u64) -> i32 {
        plg_rt_b_functor_3(m, t, n, a, NO_SITE)
    }
    fn arg3(m: *mut Machine, n: u64, t: u64, r: u64) -> i32 {
        plg_rt_b_arg_3(m, n, t, r, NO_SITE)
    }
    fn univ2(m: *mut Machine, t: u64, l: u64) -> i32 {
        plg_rt_b_univ_2(m, t, l, NO_SITE)
    }

    fn str_term(m: &mut Machine, name: &str, args: &[Word]) -> Word {
        let f = m.atoms.intern(name);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, args.len() as u32));
        m.heap.extend_from_slice(args);
        make(TAG_STR, idx as u64)
    }

    fn lst(m: &mut Machine, head: Word, tail: Word) -> Word {
        let idx = m.heap.len();
        m.heap.push(head);
        m.heap.push(tail);
        make(TAG_LST, idx as u64)
    }

    fn msg(m: &Machine) -> &str {
        m.error.as_ref().unwrap().message.as_str()
    }

    #[test]
    fn functor_decompose() {
        let mut m = machine();
        let s = str_term(&mut m, "foo", &[make_int(1), make_int(2)]);
        let name = m.new_var();
        let ar = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, s, name, ar), 1);
        let foo = m.atoms.lookup("foo").unwrap();
        assert_eq!(m.deref(name), make_atom(foo));
        assert_eq!(int_value(m.deref(ar)), 2);

        // atom: functor(a, N, A) → N=a, A=0
        let a = make_atom(m.atoms.intern("a"));
        let name = m.new_var();
        let ar = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, a, name, ar), 1);
        assert_eq!(int_value(m.deref(ar)), 0);
    }

    #[test]
    fn functor_list_is_dot_2() {
        let mut m = machine();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let l = lst(&mut m, make_int(1), nil);
        let name = m.new_var();
        let ar = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, l, name, ar), 1);
        assert_eq!(m.deref(name), make_atom(ATOM_DOT));
        assert_eq!(int_value(m.deref(ar)), 2);
    }

    #[test]
    fn functor_construct_builds_str_even_for_dot() {
        // functor(T, '.', 2) → STR './2', NOT a list cell.
        let mut m = machine();
        let t = m.new_var();
        let dot = make_atom(ATOM_DOT);
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, t, dot, make_int(2)), 1);
        let w = m.deref(t);
        assert_eq!(tag_of(w), TAG_STR);
        let (f, n) = unpack_functor(m.heap[payload(w) as usize]);
        assert_eq!((f, n), (ATOM_DOT, 2));

        // functor(T, foo, 2) → foo(_,_)
        let t = m.new_var();
        let foo = make_atom(m.atoms.intern("foo"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, t, foo, make_int(2)), 1);
        assert_eq!(tag_of(m.deref(t)), TAG_STR);
    }

    #[test]
    fn functor_construct_uses_distinct_fresh_vars() {
        // Regression for #31: functor(T, point, 2) must build point(A, B) with
        // two DISTINCT vars, so T = point(3, 4) succeeds (it failed when both
        // slots aliased one variable, reducing the unify to 3 = 4).
        let mut m = machine();
        let t = m.new_var();
        let point = make_atom(m.atoms.intern("point"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, t, point, make_int(2)), 1);

        let w = m.deref(t);
        assert_eq!(tag_of(w), TAG_STR);
        let base = payload(w) as usize;
        // The two arg cells must be different unbound variables.
        let a0 = m.deref(m.heap[base + 1]);
        let a1 = m.deref(m.heap[base + 2]);
        assert_eq!(tag_of(a0), TAG_REF);
        assert_eq!(tag_of(a1), TAG_REF);
        assert_ne!(a0, a1, "argument slots must be distinct variables");

        // The behavioral check: T = point(3, 4) succeeds and binds each slot.
        let concrete = str_term(&mut m, "point", &[make_int(3), make_int(4)]);
        assert!(unify(&mut m, t, concrete));
        let w = m.deref(t);
        let base = payload(w) as usize;
        assert_eq!(int_value(m.deref(m.heap[base + 1])), 3);
        assert_eq!(int_value(m.deref(m.heap[base + 2])), 4);
    }

    #[test]
    fn functor_errors() {
        // all unbound → instantiation
        let mut m = machine();
        let t = m.new_var();
        let n = m.new_var();
        let a = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, t, n, a), 0);
        assert_eq!(
            msg(&m),
            "error(instantiation_error, functor/3: insufficient arguments)"
        );

        // negative arity → domain_error
        let mut m = machine();
        let t = m.new_var();
        let foo = make_atom(m.atoms.intern("foo"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(functor3(mp, t, foo, make_int(-1)), 0);
        assert_eq!(
            msg(&m),
            "error(domain_error(not_less_than_zero, -1), functor/3: arity must be non-negative)"
        );
    }

    #[test]
    fn arg_in_and_out_of_range() {
        let mut m = machine();
        let s = str_term(&mut m, "foo", &[make_atom(7), make_atom(8)]);
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(arg3(mp, make_int(1), s, x), 1);
        assert_eq!(m.deref(x), make_atom(7));
        // out of range → fail (no error)
        let y = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(arg3(mp, make_int(3), s, y), 0);
        assert!(m.error.is_none());
        // arg 0 → fail
        let mp = &mut *m as *mut Machine;
        assert_eq!(arg3(mp, make_int(0), s, y), 0);
        assert!(m.error.is_none());
    }

    #[test]
    fn arg_non_compound_errors() {
        let mut m = machine();
        let x = m.new_var();
        let a = make_atom(m.atoms.intern("a"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(arg3(mp, make_int(1), a, x), 0);
        assert_eq!(
            msg(&m),
            "error(type_error(compound, a), arg/3: second argument must be compound)"
        );
    }

    #[test]
    fn univ_decompose_and_construct() {
        let mut m = machine();
        let s = str_term(&mut m, "foo", &[make_atom(7), make_int(2)]);
        let l = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(univ2(mp, s, l), 1);
        let elems = collect_list(&m, l).unwrap();
        let foo = m.atoms.lookup("foo").unwrap();
        assert_eq!(m.deref(elems[0]), make_atom(foo));
        assert_eq!(elems.len(), 3);

        // construct: T =.. ['.', a, b] builds STR './2'
        let a = make_atom(m.atoms.intern("a"));
        let b = make_atom(m.atoms.intern("b"));
        let dot = make_atom(ATOM_DOT);
        let inlist = build_list(&mut m, &[dot, a, b]);
        let t = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(univ2(mp, t, inlist), 1);
        let w = m.deref(t);
        assert_eq!(tag_of(w), TAG_STR);
        let (f, n) = unpack_functor(m.heap[payload(w) as usize]);
        assert_eq!((f, n), (ATOM_DOT, 2));
    }

    #[test]
    fn univ_single_element_and_errors() {
        // [42] → term 42
        let mut m = machine();
        let inlist = build_list(&mut m, &[make_int(42)]);
        let t = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(univ2(mp, t, inlist), 1);
        assert_eq!(int_value(m.deref(t)), 42);

        // empty list → domain_error(non_empty_list)
        let mut m = machine();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let t = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(univ2(mp, t, nil), 0);
        assert_eq!(
            msg(&m),
            "error(domain_error(non_empty_list, []), =../2: list must not be empty)"
        );
    }

    #[test]
    fn copy_term_renames_and_shares() {
        let mut m = machine();
        let x = m.new_var();
        // f(X, X)
        let s = str_term(&mut m, "f", &[x, x]);
        let c = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_copy_term_2(mp, s, c), 1);
        let cw = m.deref(c);
        assert_eq!(tag_of(cw), TAG_STR);
        let idx = payload(cw) as usize;
        let a0 = m.deref(m.heap[idx + 1]);
        let a1 = m.deref(m.heap[idx + 2]);
        assert_eq!(a0, a1, "shared var preserved");
        assert_ne!(a0, m.deref(x), "fresh var distinct from original");
    }
}
