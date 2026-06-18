//! List-sorting builtins: `msort/2`, `sort/2`.
//!
//! Ported byte-for-byte from patch-prolog v1 (`solver.rs` MSort/Sort
//! arms). Both sort by the standard order of terms (`compare_terms`):
//! - `msort/2` is a stable sort that KEEPS duplicates.
//! - `sort/2` sorts and then removes adjacent `Equal` duplicates.
//!
//! The first argument must be a proper list; otherwise v1 raises a
//! `type_error(list, Culprit)` (oracle-verified). The result is built on
//! the heap.

use crate::builtins::order::compare_terms;
use crate::cell::*;
use crate::machine::Machine;
use crate::unify::unify;
use plg_shared::atom::ATOM_NIL;
use std::cmp::Ordering;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// Collect a proper list's element words; `None` if not nil-terminated.
fn collect_list(m: &Machine, w: Word) -> Option<Vec<Word>> {
    let mut out = Vec::new();
    let mut cur = m.deref(w);
    loop {
        match tag_of(cur) {
            TAG_ATOM if atom_id(cur) == ATOM_NIL => return Some(out),
            TAG_LST => {
                let idx = payload(cur) as usize;
                out.push(m.heap[idx]);
                cur = m.deref(m.heap[idx + 1]);
            }
            _ => return None,
        }
    }
}

/// Build a nil-terminated list from words; return its word.
fn build_list(m: &mut Machine, elems: &[Word]) -> Word {
    let mut tail = make_atom(ATOM_NIL);
    for &e in elems.iter().rev() {
        let idx = m.heap.len();
        m.heap.push(e);
        m.heap.push(tail);
        tail = make(TAG_LST, idx as u64);
    }
    tail
}

/// `msort/2`: stable sort by standard order, duplicates kept.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_msort_2(m: *mut Machine, list: u64, sorted: u64, site_id: u32) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let Some(mut elems) = collect_list(m, list) else {
        let culprit = m.deref(list);
        crate::errors::type_error(m, "list", culprit, "msort/2: first argument must be a list");
        return 0;
    };
    elems.sort_by(|&a, &b| compare_terms(m, a, b));
    let lst = build_list(m, &elems);
    unify(m, sorted, lst) as i32
}

/// `sort/2`: sort by standard order then drop adjacent `Equal` dups.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_sort_2(m: *mut Machine, list: u64, sorted: u64, site_id: u32) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let Some(mut elems) = collect_list(m, list) else {
        let culprit = m.deref(list);
        crate::errors::type_error(m, "list", culprit, "sort/2: first argument must be a list");
        return 0;
    };
    elems.sort_by(|&a, &b| compare_terms(m, a, b));
    elems.dedup_by(|&mut a, &mut b| compare_terms(m, a, b) == Ordering::Equal);
    let lst = build_list(m, &elems);
    unify(m, sorted, lst) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::NO_SITE;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    // Thin wrappers: existing tests exercise behavior, not provenance.
    fn msort(m: *mut Machine, l: u64, s: u64) -> i32 {
        plg_rt_b_msort_2(m, l, s, NO_SITE)
    }
    fn sort(m: *mut Machine, l: u64, s: u64) -> i32 {
        plg_rt_b_sort_2(m, l, s, NO_SITE)
    }

    fn msg(m: &Machine) -> &str {
        m.error.as_ref().unwrap().message.as_str()
    }

    fn ints(m: &mut Machine, vals: &[i64]) -> Word {
        let ws: Vec<Word> = vals.iter().map(|&v| make_int(v)).collect();
        build_list(m, &ws)
    }

    #[test]
    fn msort_keeps_duplicates() {
        let mut m = machine();
        let l = ints(&mut m, &[3, 1, 2, 1]);
        let out = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(msort(mp, l, out), 1);
        let got: Vec<i64> = collect_list(&m, out)
            .unwrap()
            .iter()
            .map(|&w| int_value(m.deref(w)))
            .collect();
        assert_eq!(got, vec![1, 1, 2, 3]);
    }

    #[test]
    fn sort_dedups_adjacent_equal() {
        let mut m = machine();
        let l = ints(&mut m, &[3, 1, 2, 1]);
        let out = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(sort(mp, l, out), 1);
        let got: Vec<i64> = collect_list(&m, out)
            .unwrap()
            .iter()
            .map(|&w| int_value(m.deref(w)))
            .collect();
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn sort_non_list_errors() {
        let mut m = machine();
        let foo = make_atom(m.atoms.intern("foo"));
        let out = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(sort(mp, foo, out), 0);
        assert_eq!(
            msg(&m),
            "error(type_error(list, foo), sort/2: first argument must be a list)"
        );

        let mut m = machine();
        let foo = make_atom(m.atoms.intern("foo"));
        let out = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(msort(mp, foo, out), 0);
        assert_eq!(
            msg(&m),
            "error(type_error(list, foo), msort/2: first argument must be a list)"
        );
    }
}
