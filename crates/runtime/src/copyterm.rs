//! Relocatable off-heap term copies.
//!
//! One mechanism, three users: thrown error balls must survive the
//! heap rewinding that backtracks to a catch frame; findall/3 must
//! preserve each solution's template instance across the rewind that
//! discards the goal's bindings; copy_term/2 is the user-facing form.
//!
//! A `TermBuf` stores cells exactly like the heap, except pointer
//! payloads are indices into the buffer itself. Copy and restore are
//! both ITERATIVE (a long list is as deep as it is long — recursion
//! would overflow the C stack) and memoized per source cell, which
//! preserves variable sharing and keeps cyclic terms (legal without
//! occurs check) from looping.

use crate::cell::*;
use crate::machine::Machine;
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct TermBuf {
    pub cells: Vec<u64>,
    pub root: Word,
}

/// Copy `root` (a heap word) into a relocatable buffer.
pub fn copy_to_buf(m: &Machine, root: Word) -> TermBuf {
    let mut buf = TermBuf::default();
    let mut memo: HashMap<usize, usize> = HashMap::new();
    // (source heap cell, destination buffer cell) pairs left to fill.
    let mut work: Vec<(usize, usize)> = Vec::new();
    buf.root = xlate_out(m, root, &mut buf.cells, &mut memo, &mut work);
    while let Some((src, dst)) = work.pop() {
        let w = m.heap[src];
        buf.cells[dst] = xlate_out(m, w, &mut buf.cells, &mut memo, &mut work);
    }
    buf
}

fn xlate_out(
    m: &Machine,
    w: Word,
    cells: &mut Vec<u64>,
    memo: &mut HashMap<usize, usize>,
    work: &mut Vec<(usize, usize)>,
) -> Word {
    let w = m.deref(w);
    let idx = payload(w) as usize;
    match tag_of(w) {
        TAG_ATOM | TAG_INT => w,
        TAG_REF => {
            // Unbound: one buffer cell per distinct variable (sharing).
            if let Some(&b) = memo.get(&idx) {
                return make(TAG_REF, b as u64);
            }
            let b = cells.len();
            cells.push(make(TAG_REF, b as u64)); // self-ref = unbound
            memo.insert(idx, b);
            make(TAG_REF, b as u64)
        }
        TAG_STR => {
            if let Some(&b) = memo.get(&idx) {
                return make(TAG_STR, b as u64);
            }
            let (_, arity) = unpack_functor(m.heap[idx]);
            let b = cells.len();
            cells.push(m.heap[idx]); // header copies verbatim
            memo.insert(idx, b);
            for k in 0..arity as usize {
                cells.push(0);
                work.push((idx + 1 + k, b + 1 + k));
            }
            make(TAG_STR, b as u64)
        }
        TAG_LST => {
            if let Some(&b) = memo.get(&idx) {
                return make(TAG_LST, b as u64);
            }
            let b = cells.len();
            cells.push(0);
            cells.push(0);
            memo.insert(idx, b);
            work.push((idx, b));
            work.push((idx + 1, b + 1));
            make(TAG_LST, b as u64)
        }
        TAG_FLT | TAG_BIG => {
            if let Some(&b) = memo.get(&idx) {
                return make(tag_of(w), b as u64);
            }
            let b = cells.len();
            cells.push(m.heap[idx]); // raw bits copy verbatim
            memo.insert(idx, b);
            make(tag_of(w), b as u64)
        }
        _ => unreachable!("bad tag"),
    }
}

/// Materialize a buffer back onto the machine heap; returns the root.
pub fn restore_from_buf(m: &mut Machine, buf: &TermBuf) -> Word {
    let mut memo: HashMap<usize, usize> = HashMap::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    let root = xlate_in(m, buf, buf.root, &mut memo, &mut work);
    while let Some((src, dst)) = work.pop() {
        let w = buf.cells[src];
        m.heap[dst] = xlate_in(m, buf, w, &mut memo, &mut work);
    }
    root
}

fn xlate_in(
    m: &mut Machine,
    buf: &TermBuf,
    w: Word,
    memo: &mut HashMap<usize, usize>,
    work: &mut Vec<(usize, usize)>,
) -> Word {
    let idx = payload(w) as usize;
    match tag_of(w) {
        TAG_ATOM | TAG_INT => w,
        TAG_REF => {
            if let Some(&h) = memo.get(&idx) {
                return make(TAG_REF, h as u64);
            }
            let v = m.new_var();
            memo.insert(idx, payload(v) as usize);
            v
        }
        TAG_STR => {
            if let Some(&h) = memo.get(&idx) {
                return make(TAG_STR, h as u64);
            }
            let (_, arity) = unpack_functor(buf.cells[idx]);
            let h = m.heap.len();
            m.heap.push(buf.cells[idx]);
            memo.insert(idx, h);
            for k in 0..arity as usize {
                m.heap.push(0);
                work.push((idx + 1 + k, h + 1 + k));
            }
            make(TAG_STR, h as u64)
        }
        TAG_LST => {
            if let Some(&h) = memo.get(&idx) {
                return make(TAG_LST, h as u64);
            }
            let h = m.heap.len();
            m.heap.push(0);
            m.heap.push(0);
            memo.insert(idx, h);
            work.push((idx, h));
            work.push((idx + 1, h + 1));
            make(TAG_LST, h as u64)
        }
        TAG_FLT | TAG_BIG => {
            if let Some(&h) = memo.get(&idx) {
                return make(tag_of(w), h as u64);
            }
            let h = m.heap.len();
            m.heap.push(buf.cells[idx]);
            memo.insert(idx, h);
            make(tag_of(w), h as u64)
        }
        _ => unreachable!("bad tag"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    #[test]
    fn copy_restore_roundtrips_structures() {
        let mut m = machine();
        let x = m.new_var();
        // f(a, X, [1, X])
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let l1 = {
            let i = m.heap.len();
            m.heap.push(x);
            m.heap.push(nil);
            make(TAG_LST, i as u64)
        };
        let l0 = {
            let i = m.heap.len();
            m.heap.push(make_int(1));
            m.heap.push(l1);
            make(TAG_LST, i as u64)
        };
        let s = {
            let i = m.heap.len();
            m.heap.push(pack_functor(9, 3));
            m.heap.push(make_atom(7));
            m.heap.push(x);
            m.heap.push(l0);
            make(TAG_STR, i as u64)
        };
        let buf = copy_to_buf(&m, s);
        // Bind X afterwards; the copy must NOT see the binding.
        let xi = payload(x) as usize;
        m.bind(xi, make_int(42));

        let r = restore_from_buf(&mut m, &buf);
        let ri = payload(r) as usize;
        let (f, n) = unpack_functor(m.heap[ri]);
        assert_eq!((f, n), (9, 3));
        // Arg 2 (the copied X) is a FRESH unbound var, distinct from x.
        let copied_x = m.deref(m.heap[ri + 2]);
        assert_eq!(tag_of(copied_x), TAG_REF);
        assert_ne!(payload(copied_x) as usize, xi);
        // Sharing: the X inside the list is the SAME fresh var.
        let l = m.deref(m.heap[ri + 3]);
        let li = payload(l) as usize;
        let l2 = m.deref(m.heap[li + 1]);
        let l2i = payload(l2) as usize;
        assert_eq!(m.deref(m.heap[l2i]), copied_x, "var sharing preserved");
    }

    #[test]
    fn copy_snapshots_bindings_at_copy_time() {
        let mut m = machine();
        let x = m.new_var();
        m.bind(payload(x) as usize, make_atom(5));
        let buf = copy_to_buf(&m, x);
        assert_eq!(buf.root, make_atom(5), "bound var copies its value");
    }

    #[test]
    fn long_list_copy_is_iterative() {
        // 100k-deep list: recursion would overflow the stack.
        let mut m = machine();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let mut w = nil;
        for i in 0..100_000 {
            let idx = m.heap.len();
            m.heap.push(make_int(i));
            m.heap.push(w);
            w = make(TAG_LST, idx as u64);
        }
        let buf = copy_to_buf(&m, w);
        let r = restore_from_buf(&mut m, &buf);
        assert_eq!(tag_of(r), TAG_LST);
    }

    #[test]
    fn cyclic_terms_terminate() {
        let mut m = machine();
        let x = m.new_var();
        // X = f(X): legal without occurs check.
        let s = {
            let i = m.heap.len();
            m.heap.push(pack_functor(3, 1));
            m.heap.push(x);
            make(TAG_STR, i as u64)
        };
        m.bind(payload(x) as usize, s);
        let buf = copy_to_buf(&m, s);
        let r = restore_from_buf(&mut m, &buf);
        assert_eq!(tag_of(r), TAG_STR, "cycle copied without divergence");
    }
}
