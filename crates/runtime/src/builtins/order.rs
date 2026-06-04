//! Standard order of terms (ISO 8.4.2), ported from patch-prolog v1's
//! `term_compare`.
//!
//! Order:   Var < Number < Atom < Compound
//!   - vars among themselves by heap-cell index (v1: VarId)
//!   - numbers by value; Float < Integer when numerically equal; NaN last
//!   - atoms alphabetically by name
//!   - compounds by arity, then functor name, then args left-to-right
//!
//! A `LST` cell IS the compound `'.'(Head, Tail)` — arity 2, functor ".".
//! We treat it as such so a `LST` and a `STR` with functor "." / arity 2
//! compare structurally equal, exactly as v1 (which has a distinct
//! `Term::List` but compares it against `Compound('.', [h, t])` as equal —
//! see the List-vs-Compound arms in v1 `term_compare`).

use crate::cell::*;
use crate::machine::Machine;
use plg_shared::atom::ATOM_DOT;
use std::cmp::Ordering;

/// Top-level type rank: Var=0, Number=1, Atom=2, Compound(incl. list)=3.
fn type_rank(w: Word) -> u8 {
    match tag_of(w) {
        TAG_REF => 0,
        TAG_INT | TAG_FLT => 1,
        TAG_ATOM => 2,
        TAG_STR | TAG_LST => 3,
        _ => unreachable!("bad tag in term order"),
    }
}

/// Numeric value of an INT/FLT word as an f64, for cross-type comparison.
fn float_of(m: &Machine, w: Word) -> f64 {
    match tag_of(w) {
        TAG_INT => int_value(w) as f64,
        TAG_FLT => f64::from_bits(m.heap[payload(w) as usize]),
        _ => unreachable!(),
    }
}

/// Compare two numbers (INT or FLT). Float < Integer at numeric equality;
/// NaN sorts after every other float (v1 rule).
fn compare_numbers(m: &Machine, a: Word, b: Word) -> Ordering {
    match (tag_of(a), tag_of(b)) {
        (TAG_INT, TAG_INT) => int_value(a).cmp(&int_value(b)),
        (TAG_FLT, TAG_FLT) => {
            let fa = float_of(m, a);
            let fb = float_of(m, b);
            fa.partial_cmp(&fb)
                .unwrap_or_else(|| match (fa.is_nan(), fb.is_nan()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => unreachable!(),
                })
        }
        (TAG_INT, TAG_FLT) => {
            let fb = float_of(m, b);
            if fb.is_nan() {
                return Ordering::Less; // NaN sorts after everything
            }
            match (int_value(a) as f64)
                .partial_cmp(&fb)
                .unwrap_or(Ordering::Less)
            {
                Ordering::Equal => Ordering::Greater, // integer > float at equality
                other => other,
            }
        }
        (TAG_FLT, TAG_INT) => {
            let fa = float_of(m, a);
            if fa.is_nan() {
                return Ordering::Greater;
            }
            match fa
                .partial_cmp(&(int_value(b) as f64))
                .unwrap_or(Ordering::Greater)
            {
                Ordering::Equal => Ordering::Less, // float < integer at equality
                other => other,
            }
        }
        _ => unreachable!(),
    }
}

/// Functor name id and arity for a compound-like word (STR or LST). For a
/// `LST` this yields (ATOM_DOT, 2). Returns also the heap base of the args.
fn compound_view(m: &Machine, w: Word) -> (u32, u32, ArgsKind) {
    match tag_of(w) {
        TAG_STR => {
            let idx = payload(w) as usize;
            let (f, n) = unpack_functor(m.heap[idx]);
            (f, n, ArgsKind::Str(idx))
        }
        TAG_LST => (ATOM_DOT, 2, ArgsKind::Lst(payload(w) as usize)),
        _ => unreachable!(),
    }
}

enum ArgsKind {
    Str(usize), // heap index of the functor header; args at idx+1..
    Lst(usize), // heap index of [head, tail]
}

impl ArgsKind {
    /// The i-th argument word (0-based).
    fn arg(&self, m: &Machine, i: usize) -> Word {
        match self {
            ArgsKind::Str(idx) => m.heap[idx + 1 + i],
            ArgsKind::Lst(idx) => m.heap[idx + i],
        }
    }
}

/// Total standard order over two heap words (after dereferencing).
pub fn compare_terms(m: &Machine, a: Word, b: Word) -> Ordering {
    // Iterative worklist to avoid C-stack blowups on deep/long terms; each
    // entry is a pair still to compare. We process in order and short-circuit
    // on the first non-equal result.
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        let a = m.deref(a);
        let b = m.deref(b);
        if a == b {
            continue; // identical word: same var cell, atom, int, or structure
        }
        let ra = type_rank(a);
        let rb = type_rank(b);
        if ra != rb {
            return ra.cmp(&rb);
        }
        let c = match ra {
            0 => {
                // both vars: order by heap-cell index (v1 VarId)
                payload(a).cmp(&payload(b))
            }
            1 => compare_numbers(m, a, b),
            2 => m.atoms.resolve(atom_id(a)).cmp(m.atoms.resolve(atom_id(b))),
            3 => {
                let (fa, na, ka) = compound_view(m, a);
                let (fb, nb, kb) = compound_view(m, b);
                let head = (na as usize)
                    .cmp(&(nb as usize))
                    .then_with(|| m.atoms.resolve(fa).cmp(m.atoms.resolve(fb)));
                if head != Ordering::Equal {
                    return head;
                }
                // Same arity and functor: compare args left-to-right. Push in
                // reverse so the leftmost arg is compared first (LIFO stack).
                for i in (0..na as usize).rev() {
                    work.push((ka.arg(m, i), kb.arg(m, i)));
                }
                Ordering::Equal
            }
            _ => unreachable!(),
        };
        if c != Ordering::Equal {
            return c;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        let mut atoms = StringInterner::new();
        // ATOM_NIL/DOT/TRUE pre-interned by StringInterner::new.
        atoms.intern("a");
        atoms.intern("b");
        atoms.intern("f");
        atoms.intern("g");
        Machine::new(atoms, Vec::new())
    }

    fn atom(m: &Machine, name: &str) -> Word {
        make_atom(m.atoms.lookup(name).unwrap())
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
    fn rank_var_num_atom_compound() {
        let mut m = machine();
        let v = m.new_var();
        let n = make_int(1);
        let at = atom(&m, "a");
        let c = str_term(&mut m, "f", &[make_int(1)]);
        assert_eq!(compare_terms(&m, v, n), Ordering::Less);
        assert_eq!(compare_terms(&m, n, at), Ordering::Less);
        assert_eq!(compare_terms(&m, at, c), Ordering::Less);
        assert_eq!(compare_terms(&m, c, v), Ordering::Greater);
    }

    #[test]
    fn float_less_than_int_at_equality() {
        let mut m = machine();
        let f = flt(&mut m, 1.0);
        let i = make_int(1);
        assert_eq!(compare_terms(&m, f, i), Ordering::Less);
        assert_eq!(compare_terms(&m, i, f), Ordering::Greater);
        // by value otherwise
        let f2 = flt(&mut m, 0.5);
        assert_eq!(compare_terms(&m, f2, i), Ordering::Less);
    }

    #[test]
    fn nan_sorts_after_floats() {
        let mut m = machine();
        let nan = flt(&mut m, f64::NAN);
        let big = flt(&mut m, 1.0e9);
        assert_eq!(compare_terms(&m, nan, big), Ordering::Greater);
        assert_eq!(compare_terms(&m, big, nan), Ordering::Less);
    }

    #[test]
    fn atoms_alphabetical() {
        let m = machine();
        let a = atom(&m, "a");
        let b = atom(&m, "b");
        assert_eq!(compare_terms(&m, a, b), Ordering::Less);
        assert_eq!(compare_terms(&m, b, a), Ordering::Greater);
        assert_eq!(compare_terms(&m, a, a), Ordering::Equal);
    }

    #[test]
    fn compounds_arity_then_name_then_args() {
        let mut m = machine();
        // arity differs: f(1) < g(1,2)
        let f1 = str_term(&mut m, "f", &[make_int(1)]);
        let g2 = str_term(&mut m, "g", &[make_int(1), make_int(2)]);
        assert_eq!(compare_terms(&m, f1, g2), Ordering::Less);
        // same arity, name differs: f(1) < g(1)
        let g1 = str_term(&mut m, "g", &[make_int(1)]);
        let f1b = str_term(&mut m, "f", &[make_int(1)]);
        assert_eq!(compare_terms(&m, f1b, g1), Ordering::Less);
        // same arity+name, args differ: f(1) < f(2)
        let fa = str_term(&mut m, "f", &[make_int(1)]);
        let fb = str_term(&mut m, "f", &[make_int(2)]);
        assert_eq!(compare_terms(&m, fa, fb), Ordering::Less);
    }

    #[test]
    fn list_equals_dot_struct() {
        // [1,2] structurally equals '.'(1, '.'(2, [])).
        let mut m = machine();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let lst = {
            let inner = list(&mut m, make_int(2), nil);
            list(&mut m, make_int(1), inner)
        };
        let dotstruct = {
            let inner = str_term(&mut m, ".", &[make_int(2), nil]);
            str_term(&mut m, ".", &[make_int(1), inner])
        };
        assert_eq!(compare_terms(&m, lst, dotstruct), Ordering::Equal);
        assert_eq!(compare_terms(&m, dotstruct, lst), Ordering::Equal);
    }

    #[test]
    fn list_vs_improper_dot() {
        // [1,2] vs '.'(1, 2): tails [2] (compound) vs 2 (integer) → list >.
        let mut m = machine();
        let nil = make_atom(plg_shared::atom::ATOM_NIL);
        let lst = {
            let inner = list(&mut m, make_int(2), nil);
            list(&mut m, make_int(1), inner)
        };
        let improper = str_term(&mut m, ".", &[make_int(1), make_int(2)]);
        assert_eq!(compare_terms(&m, lst, improper), Ordering::Greater);
        assert_eq!(compare_terms(&m, improper, lst), Ordering::Less);
    }

    #[test]
    fn vars_ordered_by_index() {
        let mut m = machine();
        let v0 = m.new_var();
        let v1 = m.new_var();
        assert_eq!(compare_terms(&m, v0, v1), Ordering::Less);
        assert_eq!(compare_terms(&m, v1, v0), Ordering::Greater);
    }
}
