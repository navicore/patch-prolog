//! Tagged 64-bit term words — the single source of truth for the word/cell
//! ABI shared by the runtime (which builds and reads these on the heap) and
//! the compiler (which emits the same encoding into `.rodata` fact tables and
//! blobs). Keeping the tag values, the `<< 3` / `& 7` split, and the
//! functor-packing layout in ONE place means codegen and runtime can never
//! drift one-sidedly.
//!
//! A word's low 3 bits are the tag; the payload sits in the high 61.
//! Pointer-like tags (REF/STR/LST/FLT/BIG) carry **heap cell indices**, not
//! raw pointers — the heap is a growable `Vec<u64>` and indices stay valid
//! across reallocation (and across backtracking truncation).

pub type Word = u64;

pub const TAG_REF: u64 = 0; // payload = heap index of a variable cell
pub const TAG_ATOM: u64 = 1; // payload = AtomId
pub const TAG_INT: u64 = 2; // payload = signed 61-bit immediate
pub const TAG_STR: u64 = 3; // payload = heap index of [functor|arity][args...]
pub const TAG_LST: u64 = 4; // payload = heap index of [head][tail]
pub const TAG_FLT: u64 = 5; // payload = heap index of an f64-bits cell
pub const TAG_BIG: u64 = 6; // payload = heap index of a raw i64 cell

/// Largest magnitude representable as an immediate integer; values
/// outside box to a TAG_BIG heap cell (full i64 range).
pub const INT_MAX: i64 = (1 << 60) - 1;
pub const INT_MIN: i64 = -(1 << 60);

#[inline]
pub fn make(tag: u64, payload: u64) -> Word {
    (payload << 3) | tag
}

#[inline]
pub fn tag_of(w: Word) -> u64 {
    w & 7
}

#[inline]
pub fn payload(w: Word) -> u64 {
    w >> 3
}

#[inline]
pub fn make_atom(id: u32) -> Word {
    make(TAG_ATOM, id as u64)
}

#[inline]
pub fn make_int(n: i64) -> Word {
    debug_assert!((INT_MIN..=INT_MAX).contains(&n));
    ((n as u64) << 3) | TAG_INT
}

/// Signed payload extraction (arithmetic shift preserves the sign).
#[inline]
pub fn int_value(w: Word) -> i64 {
    (w as i64) >> 3
}

#[inline]
pub fn make_ref(idx: usize) -> Word {
    make(TAG_REF, idx as u64)
}

#[inline]
pub fn atom_id(w: Word) -> u32 {
    payload(w) as u32
}

/// Pack a STR header cell: functor id in the high half, arity in the low.
#[inline]
pub fn pack_functor(functor: u32, arity: u32) -> u64 {
    ((functor as u64) << 32) | arity as u64
}

#[inline]
pub fn unpack_functor(cell: u64) -> (u32, u32) {
    ((cell >> 32) as u32, cell as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_roundtrip_preserves_sign() {
        for n in [0i64, 1, -1, 42, -42, INT_MAX, INT_MIN] {
            assert_eq!(int_value(make_int(n)), n, "roundtrip {n}");
            assert_eq!(tag_of(make_int(n)), TAG_INT);
        }
    }

    #[test]
    fn functor_packing() {
        let cell = pack_functor(7, 3);
        assert_eq!(unpack_functor(cell), (7, 3));
    }

    #[test]
    fn tags_distinct() {
        assert_eq!(tag_of(make_atom(5)), TAG_ATOM);
        assert_eq!(tag_of(make_ref(9)), TAG_REF);
        assert_eq!(atom_id(make_atom(5)), 5);
    }
}
