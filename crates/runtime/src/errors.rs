//! Structured runtime errors (ISO error terms).
//!
//! An error is carried as a relocatable ball (`TermBuf`) so it survives
//! the heap rewinding that unwinds to a catch frame, plus a
//! pre-rendered message for top-level output. The message format is
//! v1's `format_term` rendering of the ball — byte-compatible with the
//! M2/M3 preformatted strings.

use crate::cell::*;
use crate::copyterm::copy_to_buf;
use crate::machine::{Machine, RtError};
use crate::render::format_term;

/// Build `error(Formal, 'Context')`, snapshot it, and set `m.error`.
/// The formal term is built on the heap transiently and trimmed back.
pub fn set_formal(m: &mut Machine, formal: Word, context: &str, uncatchable: bool) {
    let mark = m.heap.len().min(payload_start(formal, m));
    let ctx_atom = make_atom(m.atoms.intern(context));
    let err_f = m.atoms.intern("error");
    let idx = m.heap.len();
    m.heap.push(pack_functor(err_f, 2));
    m.heap.push(formal);
    m.heap.push(ctx_atom);
    let ball_word = make(TAG_STR, idx as u64);
    let ball = copy_to_buf(m, ball_word);
    let mut message = String::new();
    format_term(m, ball_word, &mut message);
    m.heap.truncate(mark.max(idx)); // drop the wrapper (and formal if transient)
    m.error = Some(RtError {
        ball,
        message,
        uncatchable,
    });
}

/// Heap index where `formal` starts if heap-allocated (for trimming).
fn payload_start(w: Word, m: &Machine) -> usize {
    match tag_of(w) {
        TAG_STR | TAG_LST | TAG_FLT | TAG_BIG => payload(w) as usize,
        _ => m.heap.len(),
    }
}

/// `instantiation_error`
pub fn instantiation(m: &mut Machine, context: &str) {
    let f = make_atom(m.atoms.intern("instantiation_error"));
    set_formal(m, f, context, false);
}

/// `type_error(Type, Culprit)`
pub fn type_error(m: &mut Machine, expected: &str, culprit: Word, context: &str) {
    let te = m.atoms.intern("type_error");
    let ty = make_atom(m.atoms.intern(expected));
    let idx = m.heap.len();
    m.heap.push(pack_functor(te, 2));
    m.heap.push(ty);
    m.heap.push(culprit);
    set_formal(m, make(TAG_STR, idx as u64), context, false);
}

/// `domain_error(Domain, Culprit)`
pub fn domain_error(m: &mut Machine, domain: &str, culprit: Word, context: &str) {
    let de = m.atoms.intern("domain_error");
    let d = make_atom(m.atoms.intern(domain));
    let idx = m.heap.len();
    m.heap.push(pack_functor(de, 2));
    m.heap.push(d);
    m.heap.push(culprit);
    set_formal(m, make(TAG_STR, idx as u64), context, false);
}

/// `evaluation_error(Kind)`
pub fn evaluation(m: &mut Machine, kind: &str, context: &str) {
    let ee = m.atoms.intern("evaluation_error");
    let k = make_atom(m.atoms.intern(kind));
    let idx = m.heap.len();
    m.heap.push(pack_functor(ee, 1));
    m.heap.push(k);
    set_formal(m, make(TAG_STR, idx as u64), context, false);
}

/// `resource_error(Kind)` — the steps variant is uncatchable (v1 rule).
pub fn resource(m: &mut Machine, kind: &str, context: &str, uncatchable: bool) {
    let re = m.atoms.intern("resource_error");
    let k = make_atom(m.atoms.intern(kind));
    let idx = m.heap.len();
    m.heap.push(pack_functor(re, 1));
    m.heap.push(k);
    set_formal(m, make(TAG_STR, idx as u64), context, uncatchable);
}

/// `existence_error(procedure, Name/Arity)` with v1's exact context.
///
/// `site_id` indexes the codegen source-location table (SPANS.md Layer 3);
/// when it resolves, ` at file:line:col` is appended to the rendered
/// top-level message. The ISO ball (`error(Formal, Context)`) is unchanged —
/// the suffix lives only in `RtError.message`, so oracle/byte-exact tests
/// pass `NO_SITE` and see the v1 bytes verbatim.
pub fn existence_procedure(m: &mut Machine, name: &str, arity: u32, site_id: u32) {
    let ee = m.atoms.intern("existence_error");
    let proc = make_atom(m.atoms.intern("procedure"));
    let slash = m.atoms.intern("/");
    let name_atom = make_atom(m.atoms.intern(name));
    let pi = m.heap.len();
    m.heap.push(pack_functor(slash, 2));
    m.heap.push(name_atom);
    m.heap.push(make_int(arity as i64));
    let idx = m.heap.len();
    m.heap.push(pack_functor(ee, 2));
    m.heap.push(proc);
    m.heap.push(make(TAG_STR, pi as u64));
    let context = format!("Undefined procedure: {name}/{arity}");
    set_formal(m, make(TAG_STR, idx as u64), &context, false);
    append_site(m, site_id);
}

/// Append ` at file:line:col` to the in-flight error message when `site_id`
/// resolves. No-op for `NO_SITE` / provenance-free binaries.
fn append_site(m: &mut Machine, site_id: u32) {
    if let Some((file, line, col)) = m.site_location(site_id)
        && let Some(err) = m.error.as_mut()
    {
        err.message.push_str(&format!(" at {file}:{line}:{col}"));
    }
}

/// `throw/1` of an arbitrary user term: the ball IS the term; the
/// top-level message is its rendering (v1 runner behavior).
pub fn throw_term(m: &mut Machine, ball_word: Word) {
    let ball_word = m.deref(ball_word);
    if tag_of(ball_word) == TAG_REF {
        // ISO: throw/1 of an unbound variable is an instantiation error.
        instantiation(m, "throw/1 requires a bound argument");
        return;
    }
    let ball = copy_to_buf(m, ball_word);
    let mut message = String::new();
    format_term(m, ball_word, &mut message);
    m.error = Some(RtError {
        ball,
        message,
        uncatchable: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    #[test]
    fn existence_message_matches_v1_bytes() {
        let mut m = machine();
        existence_procedure(&mut m, "nosuch", 1, crate::machine::NO_SITE);
        assert_eq!(
            m.error.as_ref().unwrap().message,
            "error(existence_error(procedure, /(nosuch, 1)), Undefined procedure: nosuch/1)"
        );
    }

    #[test]
    fn evaluation_message_matches_v1_bytes() {
        let mut m = machine();
        evaluation(
            &mut m,
            "zero_divisor",
            "Division by zero (integer division)",
        );
        assert_eq!(
            m.error.as_ref().unwrap().message,
            "error(evaluation_error(zero_divisor), Division by zero (integer division))"
        );
    }

    #[test]
    fn existence_suffix_appears_when_site_resolves() {
        use crate::machine::SrcLoc;
        let mut m = machine();
        m.set_provenance(
            vec![SrcLoc {
                file: 0,
                line: 12,
                col: 7,
            }],
            vec!["examples/family.pl".to_string()],
        );
        existence_procedure(&mut m, "foo", 1, 0);
        // Ball/term shape unchanged; the suffix lives only in the message.
        assert_eq!(
            m.error.as_ref().unwrap().message,
            "error(existence_error(procedure, /(foo, 1)), Undefined procedure: foo/1) \
             at examples/family.pl:12:7"
        );
    }

    #[test]
    fn ball_survives_heap_truncation() {
        let mut m = machine();
        let heap_mark = m.heap.len();
        existence_procedure(&mut m, "gone", 2, crate::machine::NO_SITE);
        m.heap.truncate(heap_mark); // simulate backtrack rewind
        let err = m.error.take().unwrap();
        let w = crate::copyterm::restore_from_buf(&mut m, &err.ball);
        let mut s = String::new();
        format_term(&m, w, &mut s);
        assert!(s.starts_with("error(existence_error(procedure"), "{s}");
    }
}
