//! Atom/number-string builtins: `atom_length/2`, `atom_chars/2`,
//! `number_chars/2`, `number_codes/2`.
//!
//! Modes follow ISO. Notes:
//! - `atom_concat/3` is nondeterministic (split mode) and so lives with the
//!   other predicate-dispatched builtins in `control.rs`, not here.
//! - `atom_chars/2` works both directions (atom→one-char atoms,
//!   list→atom).
//! - `number_chars/2` and `number_codes/2` work both directions; garbage
//!   input on the reverse direction raises a bare `syntax_error` formal.
//! - float→chars/codes uses v1's `format_float` ("3.0", not "3").

use crate::cell::*;
use crate::machine::Machine;
use crate::unify::unify;
use plg_shared::atom::ATOM_NIL;

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
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

/// v1 `format_float`: append ".0" to a whole-valued float (so number_*/2
/// renders 3.0 → "3.0", not "3"). NaN/Inf pass through `{}`.
fn format_float(f: f64) -> String {
    if f.is_nan() || f.is_infinite() {
        return format!("{f}");
    }
    let s = format!("{f}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Render a numeric word the way number_chars/number_codes expects.
fn number_string(m: &Machine, w: Word) -> Option<String> {
    match tag_of(w) {
        TAG_INT => Some(int_value(w).to_string()),
        TAG_BIG => Some((m.heap[payload(w) as usize] as i64).to_string()),
        TAG_FLT => Some(format_float(f64::from_bits(m.heap[payload(w) as usize]))),
        _ => None,
    }
}

/// Raise v1's bare `syntax_error` formal with the given context.
fn syntax_error(m: &mut Machine, context: &str) {
    let f = make_atom(m.atoms.intern("syntax_error"));
    crate::errors::set_formal(m, f, context, false);
}

/// Parse a numeric string into an INT or FLT word, mirroring v1's
/// int-then-float fallback. `None` on a parse failure or a NaN/Inf float.
fn parse_number(m: &mut Machine, s: &str) -> Option<Word> {
    if let Ok(n) = s.parse::<i64>() {
        if (INT_MIN..=INT_MAX).contains(&n) {
            return Some(make_int(n));
        }
        let idx = m.heap.len();
        m.heap.push(n as u64);
        return Some(make(TAG_BIG, idx as u64));
    }
    if let Ok(f) = s.parse::<f64>() {
        if f.is_nan() || f.is_infinite() {
            return None;
        }
        let idx = m.heap.len();
        m.heap.push(f.to_bits());
        return Some(make(TAG_FLT, idx as u64));
    }
    None
}

/// `atom_length/2`: length (in chars) of an atom. 1 = success.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_atom_length_2(
    m: *mut Machine,
    atom: u64,
    len: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(atom);
    if tag_of(w) == TAG_ATOM {
        let n = m.atoms.resolve(atom_id(w)).chars().count() as i64;
        unify(m, len, make_int(n)) as i32
    } else {
        crate::errors::type_error(
            m,
            "atom",
            w,
            "atom_length/2: first argument must be an atom",
        );
        0
    }
}

/// `atom_chars/2`: both directions (atom ↔ list of one-char atoms).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_atom_chars_2(
    m: *mut Machine,
    atom: u64,
    list: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(atom);
    match tag_of(w) {
        TAG_ATOM => {
            // Forward: atom → list of single-char atoms.
            let name = m.atoms.resolve(atom_id(w)).to_string();
            let chars: Vec<Word> = name
                .chars()
                .map(|c| make_atom(m.atoms.intern(&c.to_string())))
                .collect();
            let lst = build_list(m, &chars);
            unify(m, list, lst) as i32
        }
        TAG_REF => {
            // Reverse: list of single-char atoms → atom.
            let Some(elems) = collect_list(m, list) else {
                let culprit = m.deref(list);
                crate::errors::type_error(
                    m,
                    "list",
                    culprit,
                    "atom_chars/2: second argument must be a character list",
                );
                return 0;
            };
            match chars_to_string(m, &elems) {
                Some(s) => {
                    let id = m.atoms.intern(&s);
                    unify(m, atom, make_atom(id)) as i32
                }
                None => 0, // a non-single-char element → fail (v1 backtrack)
            }
        }
        _ => {
            crate::errors::type_error(
                m,
                "atom",
                w,
                "atom_chars/2: first argument must be an atom or variable",
            );
            0
        }
    }
}

/// Join single-character atoms into a string; `None` if any element is
/// not a one-character atom.
fn chars_to_string(m: &Machine, elems: &[Word]) -> Option<String> {
    let mut s = String::new();
    for &e in elems {
        let e = m.deref(e);
        if tag_of(e) != TAG_ATOM {
            return None;
        }
        let ch = m.atoms.resolve(atom_id(e));
        if ch.chars().count() != 1 {
            return None;
        }
        s.push_str(ch);
    }
    Some(s)
}

/// `number_chars/2`: both directions (number ↔ list of one-char atoms).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_number_chars_2(
    m: *mut Machine,
    num: u64,
    chars: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(num);
    if let Some(s) = number_string(m, w) {
        let elems: Vec<Word> = s
            .chars()
            .map(|c| make_atom(m.atoms.intern(&c.to_string())))
            .collect();
        let lst = build_list(m, &elems);
        return unify(m, chars, lst) as i32;
    }
    if tag_of(w) == TAG_REF {
        return number_from_chars(m, num, chars);
    }
    crate::errors::type_error(
        m,
        "number",
        w,
        "number_chars/2: first argument must be a number",
    );
    0
}

/// Reverse direction for `number_chars/2`.
fn number_from_chars(m: &mut Machine, num: u64, chars: u64) -> i32 {
    let Some(elems) = collect_list(m, chars) else {
        crate::errors::instantiation(m, "number_chars/2: at least one argument must be bound");
        return 0;
    };
    let Some(s) = chars_to_string(m, &elems) else {
        let culprit = m.deref(chars);
        crate::errors::domain_error(
            m,
            "single_character_atoms",
            culprit,
            "number_chars/2: list elements must be single-character atoms",
        );
        return 0;
    };
    match parse_number(m, &s) {
        Some(n) => unify(m, num, n) as i32,
        None => {
            syntax_error(m, "number_chars/2: invalid number syntax");
            0
        }
    }
}

/// `number_codes/2`: both directions (number ↔ list of char codes).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_b_number_codes_2(
    m: *mut Machine,
    num: u64,
    codes: u64,
    site_id: u32,
) -> i32 {
    let _site = crate::machine::ErrorSiteGuard::enter(m, site_id);
    let m = mref(m);
    let w = m.deref(num);
    if let Some(s) = number_string(m, w) {
        let elems: Vec<Word> = s.chars().map(|c| make_int(c as i64)).collect();
        let lst = build_list(m, &elems);
        return unify(m, codes, lst) as i32;
    }
    if tag_of(w) == TAG_REF {
        return number_from_codes(m, num, codes);
    }
    crate::errors::type_error(
        m,
        "number",
        w,
        "number_codes/2: first argument must be a number",
    );
    0
}

/// Reverse direction for `number_codes/2`.
fn number_from_codes(m: &mut Machine, num: u64, codes: u64) -> i32 {
    let Some(elems) = collect_list(m, codes) else {
        crate::errors::instantiation(m, "number_codes/2: at least one argument must be bound");
        return 0;
    };
    let mut s = String::new();
    for &e in &elems {
        let e = m.deref(e);
        let code = match tag_of(e) {
            TAG_INT => int_value(e),
            TAG_BIG => m.heap[payload(e) as usize] as i64,
            _ => return codes_domain_error(m, codes),
        };
        match (0..=0x10FFFF)
            .contains(&code)
            .then(|| char::from_u32(code as u32))
        {
            Some(Some(c)) => s.push(c),
            _ => return codes_domain_error(m, codes),
        }
    }
    match parse_number(m, &s) {
        Some(n) => unify(m, num, n) as i32,
        None => {
            syntax_error(m, "number_codes/2: invalid number syntax");
            0
        }
    }
}

fn codes_domain_error(m: &mut Machine, codes: u64) -> i32 {
    let culprit = m.deref(codes);
    crate::errors::domain_error(
        m,
        "character_codes",
        culprit,
        "number_codes/2: list elements must be valid character codes",
    );
    0
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
    fn alen(m: *mut Machine, a: u64, l: u64) -> i32 {
        plg_rt_b_atom_length_2(m, a, l, NO_SITE)
    }
    fn achars(m: *mut Machine, a: u64, l: u64) -> i32 {
        plg_rt_b_atom_chars_2(m, a, l, NO_SITE)
    }
    fn nchars(m: *mut Machine, n: u64, c: u64) -> i32 {
        plg_rt_b_number_chars_2(m, n, c, NO_SITE)
    }
    fn ncodes(m: *mut Machine, n: u64, c: u64) -> i32 {
        plg_rt_b_number_codes_2(m, n, c, NO_SITE)
    }

    fn msg(m: &Machine) -> &str {
        m.error.as_ref().unwrap().message.as_str()
    }

    fn atom_word(m: &mut Machine, s: &str) -> Word {
        make_atom(m.atoms.intern(s))
    }

    fn flt(m: &mut Machine, f: f64) -> Word {
        let idx = m.heap.len();
        m.heap.push(f.to_bits());
        make(TAG_FLT, idx as u64)
    }

    #[test]
    fn atom_length_ok_and_error() {
        let mut m = machine();
        let foo = atom_word(&mut m, "foo");
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(alen(mp, foo, x), 1);
        assert_eq!(int_value(m.deref(x)), 3);
        // mismatch fails
        let mp = &mut *m as *mut Machine;
        assert_eq!(alen(mp, foo, make_int(5)), 0);
        // non-atom errors
        let mp = &mut *m as *mut Machine;
        let y = m.new_var();
        assert_eq!(alen(mp, make_int(123), y), 0);
        assert_eq!(
            msg(&m),
            "error(type_error(atom, 123), atom_length/2: first argument must be an atom)"
        );
    }

    #[test]
    fn atom_chars_both_directions() {
        let mut m = machine();
        let foo = atom_word(&mut m, "foo");
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(achars(mp, foo, x), 1);
        let elems = collect_list(&m, x).unwrap();
        assert_eq!(elems.len(), 3);
        let f = m.atoms.lookup("f").unwrap();
        assert_eq!(m.deref(elems[0]), make_atom(f));

        // reverse: [f,o,o] → foo
        let f = atom_word(&mut m, "f");
        let o = atom_word(&mut m, "o");
        let inlist = build_list(&mut m, &[f, o, o]);
        let a = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(achars(mp, a, inlist), 1);
        let foo = m.atoms.lookup("foo").unwrap();
        assert_eq!(m.deref(a), make_atom(foo));
    }

    #[test]
    fn number_chars_both_directions() {
        let mut m = machine();
        // 123 → ['1','2','3']
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(nchars(mp, make_int(123), x), 1);
        let elems = collect_list(&m, x).unwrap();
        assert_eq!(elems.len(), 3);
        let one = m.atoms.lookup("1").unwrap();
        assert_eq!(m.deref(elems[0]), make_atom(one));

        // ['1','2','3'] → 123
        let c1 = atom_word(&mut m, "1");
        let c2 = atom_word(&mut m, "2");
        let c3 = atom_word(&mut m, "3");
        let inlist = build_list(&mut m, &[c1, c2, c3]);
        let n = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(nchars(mp, n, inlist), 1);
        assert_eq!(int_value(m.deref(n)), 123);
    }

    #[test]
    fn number_chars_float_uses_dot_zero() {
        let mut m = machine();
        let three = flt(&mut m, 3.0);
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(nchars(mp, three, x), 1);
        let s = chars_to_string(&m, &collect_list(&m, x).unwrap()).unwrap();
        assert_eq!(s, "3.0");
    }

    #[test]
    fn number_chars_garbage_is_syntax_error() {
        let mut m = machine();
        let a = atom_word(&mut m, "a");
        let inlist = build_list(&mut m, &[a]);
        let n = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(nchars(mp, n, inlist), 0);
        assert_eq!(
            msg(&m),
            "error(syntax_error, number_chars/2: invalid number syntax)"
        );
    }

    #[test]
    fn number_codes_both_directions() {
        let mut m = machine();
        // 123 → [49,50,51]
        let x = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(ncodes(mp, make_int(123), x), 1);
        let elems = collect_list(&m, x).unwrap();
        assert_eq!(int_value(m.deref(elems[0])), 49);

        // [49,50,51] → 123
        let inlist = build_list(&mut m, &[make_int(49), make_int(50), make_int(51)]);
        let n = m.new_var();
        let mp = &mut *m as *mut Machine;
        assert_eq!(ncodes(mp, n, inlist), 1);
        assert_eq!(int_value(m.deref(n)), 123);
    }
}
