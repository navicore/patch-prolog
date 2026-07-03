//! The wire layer: the fixed envelope *shape* as a typed value, plus a
//! plural `Encoder` trait whose impls are the *encodings*. Splits what was
//! collapsed in `core.rs` — where the JSON byte-emitters implicitly defined
//! the shape — so the shape is owned once (here) and the encodings vary
//! independently. See docs/design/IO.md.
//!
//! Both the CLI/WASI shell (`entry.rs`) and the Tier-2 reactor (`reactor.rs`)
//! build an `Envelope` and hand it to a chosen `Encoder`, so the envelope
//! shape has a single source and can't drift between transports.
//!
//! Two encodings: **json** (the text wire encoding, v1 byte-identical) and
//! **bson** (binary, dense, typed). Term values inside bson are carried as
//! `BinData(0x00)` wrapping a `copyterm::TermBuf` — the same cell ABI the
//! fact tables and `copy_term/2` use, single-sourced in `plg-shared::cell`,
//! lossless including cyclic terms. Because a binary format can't coexist
//! with streamed text bytes, bson forces capture mode (`can_stream() ==
//! false`); the encoding dictates the sink.

use crate::copyterm::{self, TermBuf};
use crate::machine::Machine;
use crate::render::{RenderedSolution, json_escape};
use std::io::{self, Write};

/// The fixed engine-output shape — the contract, made a type. Every
/// `Encoder` reads the same fields; only their byte encoding varies.
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],
    /// Captured `write/1` bytes, when the sink is in capture mode. `None` when
    /// output streamed to stdout (the CLI json path, v1 byte-identical).
    /// Encoders that can't stream (`can_stream() == false`) require the caller
    /// to have run in capture mode so this is `Some`.
    pub program_output: Option<&'a str>,
}

impl<'a> Envelope<'a> {
    /// Build from the machine's solved state. `program_output` follows the
    /// machine's output sink: `None` when streaming to stdout, `Some(..)` when
    /// capturing — so one constructor serves both the CLI and the reactor.
    pub fn from_machine(m: &'a Machine, exhausted: bool) -> Self {
        Self {
            count: m.solutions.len(),
            exhausted,
            solutions: &m.solutions,
            program_output: m.captured_output(),
        }
    }
}

/// The two failure classes the engine distinguishes, carrying the message the
/// wire contract puts on the wire. Callers map these to their surface (exit
/// codes 2/3 for the CLI); the message bytes come from `core::run_query`.
pub enum WireError {
    Parse(String),
    Runtime(String),
}

/// The plural encoding. One impl per wire format.
pub trait Encoder {
    /// Serialise a solved envelope. The `Machine` is needed by encodings that
    /// walk term structure directly (bson walks each binding's root word via
    /// `copyterm::copy_to_buf`); the json encoder ignores it (its bindings are
    /// pre-rendered to strings at capture time).
    fn write_envelope(&self, w: &mut dyn Write, m: &Machine, e: &Envelope) -> io::Result<()>;
    /// Serialise an error.
    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()>;
    /// Whether this encoding can coexist with streamed `write/1` bytes on the
    /// same stdout. json: yes (v1-preserved). bson: no — a binary format can't
    /// tolerate interleaved foreign bytes, so bson forces capture mode.
    fn can_stream(&self) -> bool;
}

/// Look up an encoder by the `--format` name. Returns `None` for unknown names
/// (the CLI reports a usage error → exit 2). The capability-table gating
/// (`io_format/1` → which encodings a binary advertises) lands in a later PR;
/// until then both encodings are reachable from any binary.
pub fn encoder_for(name: &str) -> Option<Box<dyn Encoder>> {
    match name {
        "json" => Some(Box::new(Json)),
        "bson" => Some(Box::new(Bson)),
        _ => None,
    }
}

// ── json: the text wire encoding ────────────────────────────────────────────

/// The text wire encoding: the v1 JSON envelope, byte-identical to the
/// pre-refactor output (`core::write_solutions_json` / `write_error_json`).
/// Registered as `"json"` to preserve the current CLI contract; the rename to
/// `text` (and the demotion of the human `X = foo` form to `--pretty`) comes
/// with the capability-table work.
pub struct Json;

impl Encoder for Json {
    fn write_envelope(&self, w: &mut dyn Write, _m: &Machine, e: &Envelope) -> io::Result<()> {
        write!(w, "{{\"count\":{},\"exhausted\":{}", e.count, e.exhausted)?;
        if let Some(out) = e.program_output {
            write!(w, ",\"output\":\"{}\"", json_escape(out))?;
        }
        w.write_all(b",\"solutions\":[")?;
        for (i, sol) in e.solutions.iter().enumerate() {
            if i > 0 {
                w.write_all(b",")?;
            }
            w.write_all(b"{")?;
            for (j, b) in sol.bindings.iter().enumerate() {
                if j > 0 {
                    w.write_all(b",")?;
                }
                write!(w, "\"{}\":{}", json_escape(&b.name), b.json)?;
            }
            w.write_all(b"}")?;
        }
        w.write_all(b"]}")
    }

    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()> {
        let msg = match e {
            WireError::Parse(m) | WireError::Runtime(m) => m,
        };
        write!(w, "{{\"error\":\"{}\"}}", json_escape(msg))
    }

    fn can_stream(&self) -> bool {
        true
    }
}

// ── bson: the binary wire encoding ──────────────────────────────────────────
//
// Hand-rolled (no serde in the runtime — footprint). BSON is little-endian,
// length-prefixed, and self-delimiting; because every document needs its total
// size up front and `dyn Write` isn't seekable, bson is built into a `Vec<u8>`
// and flushed in one `write_all`. This is the non-streaming path by design
// (`can_stream() == false`).
//
// Envelope as a bson document, field order = insertion order (bson preserves
// it, unlike JSON's sorted keys):
//     { count: int32, exhausted: bool, output?: string,
//       solutions: [ { <var>: BinData(0x00, <TermBuf bytes>), ... }, ... ] }
//
// Scalars map to native bson types (`bsondump` reads them); term values are
// opaque `BinData(0x00)` (a caller speaking bson to a patch-prolog binary has
// opted into this engine's cell ABI). See `serialize_termbuf` for the payload.

pub struct Bson;

/// BinData(0x00) payload layout for a term (the TermBuf cell format, framed):
///     byte 0      format version (0x01)
///     bytes 1..5  cell count  (u32 LE)
///     bytes 5..13 root word   (u64 LE — a tagged cell word; an immediate for
///                              scalar terms, a buffer index for structured)
///     bytes 13..  cells, each u64 LE (the `plg-shared::cell` ABI, verbatim)
///
/// `root` is a full tagged word, not a bare index: when the term is a scalar
/// (atom/integer), `copy_to_buf` returns `cells == []` and `root` carries the
/// value itself. A decoder rebuilds `TermBuf { cells, root }` and either
/// `restore_from_buf`s it or walks it with the cell ABI.
fn serialize_termbuf(tb: &TermBuf) -> Vec<u8> {
    let mut out = Vec::with_capacity(13 + tb.cells.len() * 8);
    out.push(0x01); // version
    out.extend_from_slice(&(tb.cells.len() as u32).to_le_bytes());
    out.extend_from_slice(&tb.root.to_le_bytes());
    for c in &tb.cells {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

// BSON element type bytes.
const T_STRING: u8 = 0x02;
const T_DOCUMENT: u8 = 0x03;
const T_ARRAY: u8 = 0x04;
const T_BINARY: u8 = 0x05;
const T_BOOL: u8 = 0x08;
const T_INT32: u8 = 0x10;

fn bson_cstring(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0x00);
}

fn bson_doc_begin(buf: &mut Vec<u8>) -> usize {
    let start = buf.len();
    buf.extend_from_slice(&[0; 4]); // length placeholder
    start
}

fn bson_doc_end(buf: &mut Vec<u8>, start: usize) {
    buf.push(0x00); // null terminator
    let len = i32::try_from(buf.len() - start).expect("bson doc < 2GB");
    buf[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

impl Encoder for Bson {
    fn write_envelope(&self, w: &mut dyn Write, m: &Machine, e: &Envelope) -> io::Result<()> {
        let mut buf = Vec::new();
        let doc = bson_doc_begin(&mut buf);

        // count: int32 (solution counts far below i32::MAX; saturate as a guard).
        buf.push(T_INT32);
        bson_cstring(&mut buf, "count");
        buf.extend_from_slice(&(e.count.min(i32::MAX as usize) as i32).to_le_bytes());

        // exhausted: bool.
        buf.push(T_BOOL);
        bson_cstring(&mut buf, "exhausted");
        buf.push(if e.exhausted { 0x01 } else { 0x00 });

        // output: string, only when captured (capture mode is required for bson).
        if let Some(out) = e.program_output {
            buf.push(T_STRING);
            bson_cstring(&mut buf, "output");
            let len = i32::try_from(out.len() + 1).expect("output string < 2GB");
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(out.as_bytes());
            buf.push(0x00);
        }

        // solutions: array (a bson array is a document with keys "0","1",...).
        buf.push(T_ARRAY);
        bson_cstring(&mut buf, "solutions");
        let arr = bson_doc_begin(&mut buf);
        for (i, sol) in e.solutions.iter().enumerate() {
            buf.push(T_DOCUMENT);
            bson_cstring(&mut buf, &i.to_string());
            let sdoc = bson_doc_begin(&mut buf);
            for b in &sol.bindings {
                let tb = copyterm::copy_to_buf(m, b.word);
                let payload = serialize_termbuf(&tb);
                buf.push(T_BINARY);
                bson_cstring(&mut buf, &b.name);
                let len = i32::try_from(payload.len()).expect("termbuf < 2GB");
                buf.extend_from_slice(&len.to_le_bytes());
                buf.push(0x00); // subtype: generic binary
                buf.extend_from_slice(&payload);
            }
            bson_doc_end(&mut buf, sdoc);
        }
        bson_doc_end(&mut buf, arr);

        bson_doc_end(&mut buf, doc);
        w.write_all(&buf)
    }

    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()> {
        let msg = match e {
            WireError::Parse(m) | WireError::Runtime(m) => m,
        };
        let mut buf = Vec::new();
        let doc = bson_doc_begin(&mut buf);
        buf.push(T_STRING);
        bson_cstring(&mut buf, "error");
        let len = i32::try_from(msg.len() + 1).expect("error message < 2GB");
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(msg.as_bytes());
        buf.push(0x00);
        bson_doc_end(&mut buf, doc);
        w.write_all(&buf)
    }

    fn can_stream(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{TAG_STR, make, make_atom, make_int, pack_functor, payload, tag_of};
    use plg_shared::StringInterner;
    use plg_shared::atom::ATOM_NIL;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn env<'a>(count: usize, exhausted: bool, output: Option<&'a str>) -> Envelope<'a> {
        Envelope {
            count,
            exhausted,
            solutions: &[],
            program_output: output,
        }
    }

    fn bytes(f: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> Vec<u8> {
        let mut buf = Vec::new();
        f(&mut buf).unwrap();
        buf
    }

    #[test]
    fn json_empty_success_matches_v1_shape() {
        let m = machine();
        let e = env(0, true, None);
        assert_eq!(
            String::from_utf8(bytes(|w| Json.write_envelope(w, &m, &e))).unwrap(),
            "{\"count\":0,\"exhausted\":true,\"solutions\":[]}"
        );
    }

    #[test]
    fn json_output_field_present_when_captured() {
        let m = machine();
        let e = env(0, false, Some("hi\n"));
        assert_eq!(
            String::from_utf8(bytes(|w| Json.write_envelope(w, &m, &e))).unwrap(),
            "{\"count\":0,\"exhausted\":false,\"output\":\"hi\\n\",\"solutions\":[]}"
        );
    }

    #[test]
    fn json_error_object_is_escaped() {
        assert_eq!(
            String::from_utf8(bytes(
                |w| Json.write_error(w, &WireError::Parse("a\"b".into()))
            ))
            .unwrap(),
            "{\"error\":\"a\\\"b\"}"
        );
    }

    #[test]
    fn encoder_for_known_and_unknown() {
        assert!(encoder_for("json").is_some());
        assert!(encoder_for("bson").is_some());
        assert!(encoder_for("garbage").is_none());
    }

    #[test]
    fn json_can_stream_and_bson_cannot() {
        assert!(Json.can_stream());
        assert!(!Bson.can_stream());
    }

    // ── bson structure ──────────────────────────────────────────────────────

    /// Decode just the leading int32 length of a bson document.
    fn bson_doc_len(buf: &[u8]) -> i32 {
        i32::from_le_bytes(buf[0..4].try_into().unwrap())
    }

    /// A bson document is valid iff its leading length equals the buffer length
    /// (the length covers itself + terminator). Used to check framing without a
    /// full bson parser.
    fn assert_valid_bson_doc(buf: &[u8]) {
        assert_eq!(
            bson_doc_len(buf) as usize,
            buf.len(),
            "bson doc self-delimits"
        );
        assert_eq!(
            *buf.last().unwrap(),
            0x00,
            "bson doc ends in null terminator"
        );
    }

    #[test]
    fn bson_empty_envelope_is_self_delimiting_and_carries_scalars() {
        let m = machine();
        let e = env(3, true, None);
        let buf = bytes(|w| Bson.write_envelope(w, &m, &e));
        assert_valid_bson_doc(&buf);
        // The scalar keys are present as bson cstrings.
        assert!(contains_cstring_key(&buf, b"count"));
        assert!(contains_cstring_key(&buf, b"exhausted"));
        assert!(contains_cstring_key(&buf, b"solutions"));
    }

    #[test]
    fn bson_error_document_is_valid() {
        let buf = bytes(|w| Bson.write_error(w, &WireError::Runtime("boom".into())));
        assert_valid_bson_doc(&buf);
        assert!(contains_cstring_key(&buf, b"error"));
    }

    /// Crude key scan: a bson element is `<type byte> <key bytes> 0x00`. We just
    /// check the key's cstring appears somewhere (sufficient for these fixtures;
    /// not a general parser).
    fn contains_cstring_key(buf: &[u8], key: &[u8]) -> bool {
        let mut needle = key.to_vec();
        needle.push(0x00);
        buf.windows(needle.len()).any(|w| w == needle.as_slice())
    }

    // ── TermBuf framing round-trip (the losslessness claim) ─────────────────

    /// Mirror of `serialize_termbuf` on the decode side, for testing.
    fn deserialize_termbuf(data: &[u8]) -> TermBuf {
        assert_eq!(data[0], 0x01, "format version");
        let n = u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize;
        let root = u64::from_le_bytes(data[5..13].try_into().unwrap());
        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let off = 13 + i * 8;
            cells.push(u64::from_le_bytes(data[off..off + 8].try_into().unwrap()));
        }
        TermBuf { cells, root }
    }

    fn term_buf_of(m: &Machine, w: u64) -> TermBuf {
        copyterm::copy_to_buf(m, w)
    }

    #[test]
    fn termbuf_framing_roundtrips_a_scalar_atom() {
        let m = machine();
        let a = make_atom(7);
        let tb = term_buf_of(&m, a);
        assert!(tb.cells.is_empty(), "scalar copies to an empty cell vec");
        let rt = deserialize_termbuf(&serialize_termbuf(&tb));
        assert_eq!(rt.cells, tb.cells);
        assert_eq!(rt.root, tb.root);
        assert_eq!(rt.root, a, "scalar root carries the value");
    }

    #[test]
    fn termbuf_framing_roundtrips_a_cyclic_term_losslessly() {
        // X = f(X): legal without occurs check; text/JSON can't round-trip this
        // (render cuts it to f(_N)), but bson/TermBuf must. (IO.md flag #1.)
        let mut m = machine();
        let x = m.new_var();
        let s = {
            let i = m.heap.len();
            m.heap.push(pack_functor(3, 1));
            m.heap.push(x);
            make(TAG_STR, i as u64)
        };
        m.bind(payload(x) as usize, s);

        let tb = term_buf_of(&m, s);
        let payload_bytes = serialize_termbuf(&tb);
        let rt = deserialize_termbuf(&payload_bytes);
        let restored = copyterm::restore_from_buf(&mut m, &rt);
        assert_eq!(tag_of(restored), TAG_STR, "cycle restored as a structure");
        // The structure's single arg must be itself (the cycle survived).
        let ri = payload(restored) as usize;
        assert_eq!(
            m.deref(m.heap[ri + 1]),
            restored,
            "f(X) arg is the term itself"
        );
    }

    #[test]
    fn termbuf_framing_roundtrips_a_list() {
        let mut m = machine();
        let nil = make_atom(ATOM_NIL);
        let l = {
            let i = m.heap.len();
            m.heap.push(make_int(1));
            m.heap.push(nil);
            make(crate::cell::TAG_LST, i as u64)
        };
        let tb = term_buf_of(&m, l);
        let rt = deserialize_termbuf(&serialize_termbuf(&tb));
        let restored = copyterm::restore_from_buf(&mut m, &rt);
        assert_eq!(tag_of(restored), crate::cell::TAG_LST);
    }
}
