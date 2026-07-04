//! Type-checking builtins: `var/1`, `nonvar/1`, `atom/1`, `number/1`,
//! `integer/1`, `float/1`, `compound/1`, `is_list/1`.
//!
//! Each deref-tests a single tag. Notable decisions:
//! - `integer/1` accepts both the immediate `TAG_INT` and the boxed
//!   `TAG_BIG` (the full i64 range).
//! - `compound/1` is TRUE for lists: a `TAG_LST` counts as compound
//!   (`compound([1])` succeeds).
//! - `is_list/1` walks the tail iteratively and is true only for a
//!   proper, nil-terminated list (`is_list([a|b])` fails).

use crate::cell::*;
use crate::machine::Machine;
use plg_shared::atom::ATOM_NIL;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// `var/1`: succeeds iff the dereferenced argument is an unbound variable.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_var_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    (tag_of(m.deref(a)) == TAG_REF) as i32
}

/// `nonvar/1`: the negation of `var/1`.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_nonvar_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    (tag_of(m.deref(a)) != TAG_REF) as i32
}

/// `atom/1`: succeeds iff the argument is an atom.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_atom_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    (tag_of(m.deref(a)) == TAG_ATOM) as i32
}

/// `number/1`: any numeric tag (immediate int, boxed big, or float).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_number_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    matches!(tag_of(m.deref(a)), TAG_INT | TAG_BIG | TAG_FLT) as i32
}

/// `integer/1`: immediate `TAG_INT` or boxed `TAG_BIG`.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_integer_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    matches!(tag_of(m.deref(a)), TAG_INT | TAG_BIG) as i32
}

/// `float/1`: a boxed float.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_float_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    (tag_of(m.deref(a)) == TAG_FLT) as i32
}

/// `compound/1`: a structure OR a list cell (lists count as compound).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_compound_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    matches!(tag_of(m.deref(a)), TAG_STR | TAG_LST) as i32
}

/// `is_list/1`: a proper (nil-terminated) list. Iterative tail walk so a
/// long list can't blow the C stack; a partial or improper list fails.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_is_list_1(m: *mut Machine, a: u64) -> i32 {
    let m = mref(m);
    let mut cur = m.deref(a);
    loop {
        match tag_of(cur) {
            TAG_ATOM if atom_id(cur) == ATOM_NIL => return 1,
            TAG_LST => {
                let idx = payload(cur) as usize;
                cur = m.deref(m.heap[idx + 1]); // follow the tail
            }
            _ => return 0, // var tail, improper tail, or non-list
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn big(m: &mut Machine, n: i64) -> Word {
        let idx = m.heap.len();
        m.heap.push(n as u64);
        make(TAG_BIG, idx as u64)
    }

    fn flt(m: &mut Machine, f: f64) -> Word {
        let idx = m.heap.len();
        m.heap.push(f.to_bits());
        make(TAG_FLT, idx as u64)
    }

    fn str_term(m: &mut Machine, name: &str, args: &[Word]) -> Word {
        let f = m.atoms.intern(name);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, args.len() as u32));
        m.heap.extend_from_slice(args);
        make(TAG_STR, idx as u64)
    }

    fn list(m: &mut Machine, head: Word, tail: Word) -> Word {
        let idx = m.heap.len();
        m.heap.push(head);
        m.heap.push(tail);
        make(TAG_LST, idx as u64)
    }

    #[test]
    fn var_nonvar() {
        let mut m = machine();
        let v = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_var_1(mp, v), 1);
        assert_eq!(plg_rt_b_nonvar_1(mp, v), 0);
        assert_eq!(plg_rt_b_var_1(mp, make_int(3)), 0);
        assert_eq!(plg_rt_b_nonvar_1(mp, make_int(3)), 1);
        // bound var derefs to its value (nonvar)
        m.bind(payload(v) as usize, make_atom(7));
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_var_1(mp, v), 0);
        assert_eq!(plg_rt_b_nonvar_1(mp, v), 1);
    }

    #[test]
    fn atom_number_integer_float() {
        let mut m = machine();
        let a = make_atom(m.atoms.intern("a"));
        let b = big(&mut m, 1 << 62);
        let f = flt(&mut m, 1.5);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_atom_1(mp, a), 1);
        assert_eq!(plg_rt_b_atom_1(mp, make_int(1)), 0);

        assert_eq!(plg_rt_b_number_1(mp, make_int(1)), 1);
        assert_eq!(plg_rt_b_number_1(mp, b), 1);
        assert_eq!(plg_rt_b_number_1(mp, f), 1);
        assert_eq!(plg_rt_b_number_1(mp, a), 0);

        assert_eq!(plg_rt_b_integer_1(mp, make_int(1)), 1);
        assert_eq!(plg_rt_b_integer_1(mp, b), 1); // TAG_BIG counts
        assert_eq!(plg_rt_b_integer_1(mp, f), 0);

        assert_eq!(plg_rt_b_float_1(mp, f), 1);
        assert_eq!(plg_rt_b_float_1(mp, make_int(1)), 0);
        assert_eq!(plg_rt_b_float_1(mp, b), 0);
    }

    #[test]
    fn compound_includes_lists() {
        let mut m = machine();
        let nil = make_atom(ATOM_NIL);
        let s = str_term(&mut m, "foo", &[make_int(1)]);
        let l = list(&mut m, make_int(1), nil);
        let a = make_atom(m.atoms.intern("a"));
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_compound_1(mp, s), 1); // foo(1)
        assert_eq!(plg_rt_b_compound_1(mp, l), 1); // [1] — list IS compound
        assert_eq!(plg_rt_b_compound_1(mp, a), 0);
        assert_eq!(plg_rt_b_compound_1(mp, make_int(1)), 0);
    }

    #[test]
    fn is_list_proper_partial_improper() {
        let mut m = machine();
        let nil = make_atom(ATOM_NIL);
        // [a, b]
        let proper = {
            let inner = list(&mut m, make_atom(2), nil);
            list(&mut m, make_atom(1), inner)
        };
        // [a|b] improper
        let improper = list(&mut m, make_atom(1), make_atom(2));
        // [a|V] partial
        let v = m.new_var();
        let partial = list(&mut m, make_atom(1), v);
        let mp = &mut *m as *mut Machine;
        assert_eq!(plg_rt_b_is_list_1(mp, proper), 1);
        assert_eq!(plg_rt_b_is_list_1(mp, nil), 1); // [] is a proper list
        assert_eq!(plg_rt_b_is_list_1(mp, improper), 0);
        assert_eq!(plg_rt_b_is_list_1(mp, partial), 0);
        assert_eq!(plg_rt_b_is_list_1(mp, make_int(1)), 0);
    }
}
