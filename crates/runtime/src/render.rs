//! Solution rendering: the v1 wire contract, byte-compatible.
//!
//! JSON is hand-rolled (no serde in the runtime — binary size) but must
//! match serde_json's output for the value shapes v1 produced: object
//! keys in sorted order (serde_json's default BTreeMap), the same
//! string escaping, and `{"functor":...,"args":[...]}` sorting to
//! `{"args":...,"functor":...}` exactly as v1 emitted it.

use crate::cell::*;
use crate::machine::Machine;
use plg_shared::atom::ATOM_NIL;

/// One captured solution: bindings sorted by variable name (v1 rule),
/// rendered immediately (terms are undone by backtracking afterwards).
pub struct RenderedSolution {
    /// (name, json_value, text_value) per query variable, `_` excluded.
    pub bindings: Vec<(String, String, String)>,
}

/// Capture the current solution from the machine's query variables.
pub fn capture_solution(m: &Machine) -> RenderedSolution {
    let mut vars: Vec<_> = m.query_vars.iter().collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));
    let bindings = vars
        .into_iter()
        .filter(|(name, _)| name != "_")
        .map(|(name, idx)| {
            let w = m.deref(make_ref(*idx));
            (name.clone(), term_to_json(m, w), term_to_string(m, w))
        })
        .collect();
    RenderedSolution { bindings }
}

/// serde_json-compatible string escaping.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Float formatting compatible with v1: text used Rust `{}`; JSON used
/// serde_json (ryu). Both print 3.14 as "3.14"; ryu prints whole floats
/// as "3.0" where `{}` prints "3". Force the ".0" for whole floats.
fn fmt_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

pub fn term_to_json(m: &Machine, w: Word) -> String {
    term_to_json_v(m, w, &mut Vec::new())
}

/// `visiting` holds the heap indices of STR/LST cells currently being
/// expanded: re-encountering one means the term is cyclic (legal
/// without occurs check), and the cycle is cut by rendering a variable
/// — exactly v1's cycle-safe `apply()` behavior (`X = f(X)` renders as
/// `f(_N)`).
fn term_to_json_v(m: &Machine, w: Word, visiting: &mut Vec<usize>) -> String {
    let w = m.deref(w);
    match tag_of(w) {
        TAG_ATOM => format!("\"{}\"", json_escape(m.atoms.resolve(atom_id(w)))),
        TAG_INT => int_value(w).to_string(),
        TAG_BIG => (m.heap[payload(w) as usize] as i64).to_string(),
        TAG_FLT => fmt_float(f64::from_bits(m.heap[payload(w) as usize])),
        TAG_REF => format!("\"_{}\"", payload(w)),
        TAG_STR => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                return format!("\"_{idx}\""); // cycle cut (v1 behavior)
            }
            visiting.push(idx);
            let (f, n) = unpack_functor(m.heap[idx]);
            let args: Vec<String> = (0..n as usize)
                .map(|i| term_to_json_v(m, m.heap[idx + 1 + i], visiting))
                .collect();
            visiting.pop();
            // serde_json sorted keys: "args" < "functor"
            format!(
                "{{\"args\":[{}],\"functor\":\"{}\"}}",
                args.join(","),
                json_escape(m.atoms.resolve(f))
            )
        }
        TAG_LST => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                return format!("\"_{idx}\"");
            }
            visiting.push(idx);
            let (elements, tail) = collect_list_v(m, w, visiting);
            let items: Vec<String> = elements
                .iter()
                .map(|e| term_to_json_v(m, *e, visiting))
                .collect();
            let out = match tail {
                None => format!("[{}]", items.join(",")),
                // serde_json sorted keys: "list" < "tail"
                Some(t) => format!(
                    "{{\"list\":[{}],\"tail\":{}}}",
                    items.join(","),
                    term_to_json_v(m, t, visiting)
                ),
            };
            visiting.pop();
            out
        }
        _ => unreachable!("bad tag"),
    }
}

/// v1's infix-operator set for human-readable compound rendering.
const INFIX: &[&str] = &[
    "+", "-", "*", "/", "mod", "is", "=", "\\=", "<", ">", "=<", ">=", "=:=", "=\\=",
];

pub fn term_to_string(m: &Machine, w: Word) -> String {
    term_to_string_v(m, w, &mut Vec::new())
}

fn term_to_string_v(m: &Machine, w: Word, visiting: &mut Vec<usize>) -> String {
    let w = m.deref(w);
    match tag_of(w) {
        TAG_ATOM => m.atoms.resolve(atom_id(w)).to_string(),
        TAG_INT => int_value(w).to_string(),
        TAG_BIG => (m.heap[payload(w) as usize] as i64).to_string(),
        TAG_FLT => format!("{}", f64::from_bits(m.heap[payload(w) as usize])),
        TAG_REF => format!("_{}", payload(w)),
        TAG_STR => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                return format!("_{idx}"); // cycle cut (v1 behavior)
            }
            visiting.push(idx);
            let (f, n) = unpack_functor(m.heap[idx]);
            let name = m.atoms.resolve(f).to_string();
            let out = if n == 2 && INFIX.contains(&name.as_str()) {
                format!(
                    "{} {} {}",
                    term_to_string_v(m, m.heap[idx + 1], visiting),
                    name,
                    term_to_string_v(m, m.heap[idx + 2], visiting)
                )
            } else {
                let args: Vec<String> = (0..n as usize)
                    .map(|i| term_to_string_v(m, m.heap[idx + 1 + i], visiting))
                    .collect();
                format!("{}({})", name, args.join(", "))
            };
            visiting.pop();
            out
        }
        TAG_LST => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                return format!("_{idx}");
            }
            visiting.push(idx);
            let (elements, tail) = collect_list_v(m, w, visiting);
            let items: Vec<String> = elements
                .iter()
                .map(|e| term_to_string_v(m, *e, visiting))
                .collect();
            let out = match tail {
                None => format!("[{}]", items.join(", ")),
                Some(t) => format!(
                    "[{}|{}]",
                    items.join(", "),
                    term_to_string_v(m, t, visiting)
                ),
            };
            visiting.pop();
            out
        }
        _ => unreachable!("bad tag"),
    }
}

/// v1's `format_term` rendering: plain functional notation (no infix),
/// atoms unquoted, vars `_<idx>`, lists `[a, b|T]`. This is the byte
/// contract for error messages ("Runtime error: error(...)").
pub fn format_term(m: &Machine, w: Word, out: &mut String) {
    format_term_v(m, w, out, &mut Vec::new())
}

fn format_term_v(m: &Machine, w: Word, out: &mut String, visiting: &mut Vec<usize>) {
    let w = m.deref(w);
    match tag_of(w) {
        TAG_ATOM => out.push_str(m.atoms.resolve(atom_id(w))),
        TAG_INT => out.push_str(&int_value(w).to_string()),
        TAG_BIG => out.push_str(&(m.heap[payload(w) as usize] as i64).to_string()),
        TAG_FLT => out.push_str(&f64::from_bits(m.heap[payload(w) as usize]).to_string()),
        TAG_REF => {
            out.push('_');
            out.push_str(&payload(w).to_string());
        }
        TAG_STR => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                out.push('_');
                out.push_str(&idx.to_string());
                return;
            }
            visiting.push(idx);
            let (f, n) = unpack_functor(m.heap[idx]);
            out.push_str(m.atoms.resolve(f));
            out.push('(');
            for i in 0..n as usize {
                if i > 0 {
                    out.push_str(", ");
                }
                format_term_v(m, m.heap[idx + 1 + i], out, visiting);
            }
            out.push(')');
            visiting.pop();
        }
        TAG_LST => {
            let idx = payload(w) as usize;
            if visiting.contains(&idx) {
                out.push('_');
                out.push_str(&idx.to_string());
                return;
            }
            visiting.push(idx);
            out.push('[');
            let (elements, tail) = collect_list_v(m, w, visiting);
            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_term_v(m, *e, out, visiting);
            }
            if let Some(t) = tail {
                out.push('|');
                format_term_v(m, t, out, visiting);
            }
            out.push(']');
            visiting.pop();
        }
        _ => unreachable!("bad tag"),
    }
}

/// Walk a LST chain. Returns the element words and `None` if the list
/// is proper (nil-terminated) or `Some(tail)` for a partial list. A
/// spine cell already being rendered (cyclic list) terminates the walk
/// as an improper tail so the cycle cut renders as a variable.
fn collect_list_v(m: &Machine, w: Word, visiting: &[usize]) -> (Vec<Word>, Option<Word>) {
    let mut elements = Vec::new();
    let mut cur = m.deref(w);
    let mut seen: Vec<usize> = Vec::new();
    loop {
        match tag_of(cur) {
            TAG_LST => {
                let idx = payload(cur) as usize;
                if seen.contains(&idx) || (visiting.contains(&idx) && !elements.is_empty()) {
                    return (elements, Some(cur));
                }
                seen.push(idx);
                elements.push(m.heap[idx]);
                cur = m.deref(m.heap[idx + 1]);
            }
            TAG_ATOM if atom_id(cur) == ATOM_NIL => return (elements, None),
            _ => return (elements, Some(cur)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        let mut atoms = StringInterner::new();
        atoms.intern("foo");
        atoms.intern("bar");
        Machine::new(atoms, Vec::new())
    }

    #[test]
    fn json_escape_matches_serde() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(json_escape("\u{01}"), "\\u0001");
    }

    #[test]
    fn atoms_ints_render() {
        let m = machine();
        let foo = m.atoms.lookup("foo").unwrap();
        assert_eq!(term_to_json(&m, make_atom(foo)), "\"foo\"");
        assert_eq!(term_to_json(&m, make_int(-7)), "-7");
        assert_eq!(term_to_string(&m, make_int(-7)), "-7");
    }

    #[test]
    fn compound_renders_sorted_keys() {
        let mut m = machine();
        let foo = m.atoms.lookup("foo").unwrap();
        let bar = m.atoms.lookup("bar").unwrap();
        let idx = m.heap.len();
        m.heap.push(pack_functor(foo, 2));
        m.heap.push(make_atom(bar));
        m.heap.push(make_int(1));
        let w = make(TAG_STR, idx as u64);
        assert_eq!(
            term_to_json(&m, w),
            "{\"args\":[\"bar\",1],\"functor\":\"foo\"}"
        );
        assert_eq!(term_to_string(&m, w), "foo(bar, 1)");
    }

    #[test]
    fn proper_and_partial_lists() {
        let mut m = machine();
        let nil = make_atom(ATOM_NIL);
        let i2 = m.heap.len();
        m.heap.push(make_int(2));
        m.heap.push(nil);
        let l2 = make(TAG_LST, i2 as u64);
        let i1 = m.heap.len();
        m.heap.push(make_int(1));
        m.heap.push(l2);
        let l1 = make(TAG_LST, i1 as u64);
        assert_eq!(term_to_json(&m, l1), "[1,2]");
        assert_eq!(term_to_string(&m, l1), "[1, 2]");

        let v = m.new_var();
        let ip = m.heap.len();
        m.heap.push(make_int(1));
        m.heap.push(v);
        let lp = make(TAG_LST, ip as u64);
        let json = term_to_json(&m, lp);
        assert!(json.starts_with("{\"list\":[1],\"tail\":\"_"), "{json}");
        assert!(term_to_string(&m, lp).starts_with("[1|_"));
    }
}
