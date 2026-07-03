//! The wire layer: the fixed envelope *shape* as a typed value, plus plural
//! encodings exposed as **descriptors** (vtables of function pointers). Splits
//! what was collapsed in `core.rs` — where the JSON byte-emitters implicitly
//! defined the shape — so the shape is owned once (here) and the encodings
//! vary independently. See docs/design/IO.md.
//!
//! Both the CLI/WASI shell (`entry.rs`) and the Tier-2 reactor (`reactor.rs`)
//! build an `Envelope` and drive it through a chosen `EncoderDesc`.
//!
//! **Why descriptors, not a trait.** Codegen bakes a per-binary *capability
//! table* — pointers to the descriptors the program declared via
//! `io_format/1` — and `entry.rs` dispatches through those pointers, never
//! naming an encoder statically. Link-time `--gc-sections` then strips any
//! encoder whose descriptor isn't in the table (a `[text]`-only binary links
//! no bson code). A trait-object `encoder_for(name)` match would reference
//! every encoder statically and defeat that.
//!
//! Two encodings: **json** (the text wire encoding, v1 byte-identical) and
//! **bson** (binary, dense, typed). Term values inside bson are `BinData(0x00)`
//! wrapping a `copyterm::TermBuf` — the same cell ABI the fact tables and
//! `copy_term/2` use, single-sourced in `plg-shared::cell`, lossless including
//! cyclic terms. Because a binary format can't coexist with streamed text
//! bytes, bson forces capture mode; the encoding dictates the sink.

use crate::copyterm::{self, TermBuf};
use crate::machine::Machine;
use crate::render::{RenderedSolution, json_escape};
use std::io::{self, Write};

/// The fixed engine-output shape — the contract, made a type. Every encoding
/// reads the same fields; only their byte encoding varies.
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],
    /// Captured `write/1` bytes, when the sink is in capture mode. `None` when
    /// output streamed to stdout (the CLI json path, v1 byte-identical).
    /// Encodings that can't stream require the caller to have run in capture
    /// mode so this is `Some`.
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

/// An encoding as a vtable: a name plus the three operations the wire contract
/// needs. `#[repr(C)]` so codegen can reference a descriptor by address and
/// `entry.rs` can read its fields after dereferencing the pointer from the
/// capability table. A `#[no_mangle] static` of this type per encoding
/// (`PLG_ENC_JSON`, `PLG_ENC_BSON`) is what codegen's `@plg_caps` table points
/// at; encoders not listed there are unreferenced and get dead-stripped.
#[repr(C)]
pub struct EncoderDesc {
    /// The `--format` name this descriptor answers to ("json", "bson").
    pub name: &'static str,
    pub write_envelope: fn(&mut dyn Write, &Machine, &Envelope) -> io::Result<()>,
    pub write_error: fn(&mut dyn Write, &WireError) -> io::Result<()>,
    /// False ⇒ the encoding can't coexist with streamed `write/1` bytes on the
    /// same stdout (binary formats), so the caller must run in capture mode.
    pub can_stream: fn() -> bool,
}

impl EncoderDesc {
    /// Find a descriptor by name in a capability table (codegen-baked pointers
    /// to `#[no_mangle] static` descriptors). Returns `None` when the name
    /// isn't advertised — `entry.rs` maps that to a usage error (exit 2).
    /// # Safety
    /// `caps` must point at `len` valid pointers to static descriptors.
    pub unsafe fn find(
        caps: *const *const EncoderDesc,
        len: usize,
        name: &str,
    ) -> Option<&'static EncoderDesc> {
        let slice = unsafe { std::slice::from_raw_parts(caps, len) };
        for &p in slice {
            let d = unsafe { &*p };
            if d.name == name {
                return Some(d);
            }
        }
        None
    }
}

// ── json: the text wire encoding ────────────────────────────────────────────

fn json_write_envelope(w: &mut dyn Write, _m: &Machine, e: &Envelope) -> io::Result<()> {
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

fn json_write_error(w: &mut dyn Write, e: &WireError) -> io::Result<()> {
    let msg = match e {
        WireError::Parse(m) | WireError::Runtime(m) => m,
    };
    write!(w, "{{\"error\":\"{}\"}}", json_escape(msg))
}

const fn json_can_stream() -> bool {
    true
}

/// The json (text) wire encoding: v1 JSON, byte-identical to the pre-refactor
/// output. Registered as `"json"` to preserve the current CLI contract; the
/// rename to `text` (and the demotion of the human `X = foo` form to
/// `--pretty`) comes with the CLI-flag work.
#[unsafe(no_mangle)]
pub static PLG_ENC_JSON: EncoderDesc = EncoderDesc {
    name: "json",
    write_envelope: json_write_envelope,
    write_error: json_write_error,
    can_stream: json_can_stream,
};

// ── bson: the binary wire encoding ──────────────────────────────────────────
//
// Hand-rolled (no serde in the runtime — footprint). BSON is little-endian,
// length-prefixed, self-delimiting; because every document needs its total
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
const T_INT64: u8 = 0x12;

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

fn bson_write_envelope(w: &mut dyn Write, m: &Machine, e: &Envelope) -> io::Result<()> {
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

fn bson_write_error(w: &mut dyn Write, e: &WireError) -> io::Result<()> {
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

const fn bson_can_stream() -> bool {
    false
}

#[unsafe(no_mangle)]
pub static PLG_ENC_BSON: EncoderDesc = EncoderDesc {
    name: "bson",
    write_envelope: bson_write_envelope,
    write_error: bson_write_error,
    can_stream: bson_can_stream,
};

// ── bson input: the one-field request document ──────────────────────────────
//
// Input is transport framing around a query string (IO.md's "honest cheat"
// — the engine parses the inner string exactly as for argv `--query`, so there
// is no bson-term loader). The request document is:
//     { query: string, limit?: int32|int64 }
// Output format is NOT carried here — it stays on argv `--format` (input and
// output encodings are orthogonal). Unknown fields are skipped for forward-
// compat; `query` is required and must be a string.

/// A parsed bson request document. `limit` is `None` when absent.
#[derive(Debug)]
pub struct ParsedRequest {
    pub query: String,
    pub limit: Option<usize>,
}

/// Parse a bson request document from `buf`. `buf` may be longer than the
/// document (stdin may carry trailing bytes); only the leading document
/// (delimited by its int32 length) is consumed. Returns the `query` and an
/// optional `limit`.
pub fn parse_bson_request(buf: &[u8]) -> Result<ParsedRequest, String> {
    if buf.len() < 5 {
        return Err("bson request too short".to_string());
    }
    let total = i32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if total < 5 || total > buf.len() {
        return Err(format!(
            "bson request length mismatch: declared {total}, have {}",
            buf.len()
        ));
    }
    let body = &buf[..total];
    let mut off = 4; // skip the length prefix
    let end = total - 1; // last byte is the 0x00 terminator
    let mut query = None;
    let mut limit = None;
    while off < end {
        let ty = body[off];
        off += 1;
        let (key, after_key) = read_cstring(body, off)?;
        off = after_key;
        match (ty, key.as_str()) {
            (T_STRING, "query") => {
                let (s, next) = read_string(body, off)?;
                query = Some(s);
                off = next;
            }
            (T_INT32, "limit") => {
                let n = read_i32(body, off)?;
                limit = Some(n.max(0) as usize);
                off += 4;
            }
            (T_INT64, "limit") => {
                // int64
                let n = read_i64(body, off)?;
                limit = Some(n.max(0) as usize);
                off += 8;
            }
            _ => {
                // Unknown field (or known field of a different type): skip its
                // value for forward-compat, erroring on a type we can't size.
                off = skip_value(body, off, ty)?;
            }
        }
    }
    let query = query.ok_or_else(|| "bson request missing required 'query' string".to_string())?;
    Ok(ParsedRequest { query, limit })
}

fn read_cstring(buf: &[u8], mut off: usize) -> Result<(String, usize), String> {
    let end = buf[off..]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| "bson key not null-terminated".to_string())?;
    let s = std::str::from_utf8(&buf[off..off + end])
        .map_err(|_| "bson key not utf-8".to_string())?
        .to_string();
    off += end + 1;
    Ok((s, off))
}

fn read_string(buf: &[u8], off: usize) -> Result<(String, usize), String> {
    let n = read_i32(buf, off)? as usize;
    if n == 0 || off + 4 + n > buf.len() {
        return Err("bson string length out of range".to_string());
    }
    // the count includes a trailing null
    let s = std::str::from_utf8(&buf[off + 4..off + 4 + n - 1])
        .map_err(|_| "bson string not utf-8".to_string())?
        .to_string();
    Ok((s, off + 4 + n))
}

fn read_i32(buf: &[u8], off: usize) -> Result<i32, String> {
    buf[off..]
        .get(..4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "bson int32 truncated".to_string())
}

fn read_i64(buf: &[u8], off: usize) -> Result<i64, String> {
    buf[off..]
        .get(..8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "bson int64 truncated".to_string())
}

/// Advance past a value of known bson type, returning the new offset. Used to
/// skip unknown fields for forward-compat.
fn skip_value(buf: &[u8], off: usize, ty: u8) -> Result<usize, String> {
    match ty {
        0x01 => Ok(off + 8),                      // double
        T_STRING => Ok(read_string(buf, off)?.1), // string
        T_DOCUMENT | T_ARRAY => {
            // document/array
            let n = read_i32(buf, off)? as usize;
            Ok(off + n)
        }
        T_BINARY => {
            let n = read_i32(buf, off)? as usize;
            Ok(off + 4 + 1 + n)
        }
        T_BOOL => Ok(off + 1),
        0x0A => Ok(off), // null
        T_INT32 => Ok(off + 4),
        0x12 => Ok(off + 8), // int64
        _ => Err(format!("bson: cannot skip unknown element type {ty:#x}")),
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

    fn enc(name: &str) -> &'static EncoderDesc {
        match name {
            "json" => &PLG_ENC_JSON,
            "bson" => &PLG_ENC_BSON,
            _ => unreachable!(),
        }
    }

    #[test]
    fn json_empty_success_matches_v1_shape() {
        let m = machine();
        let e = env(0, true, None);
        assert_eq!(
            String::from_utf8(bytes(|w| (enc("json").write_envelope)(w, &m, &e))).unwrap(),
            "{\"count\":0,\"exhausted\":true,\"solutions\":[]}"
        );
    }

    #[test]
    fn json_output_field_present_when_captured() {
        let m = machine();
        let e = env(0, false, Some("hi\n"));
        assert_eq!(
            String::from_utf8(bytes(|w| (enc("json").write_envelope)(w, &m, &e))).unwrap(),
            "{\"count\":0,\"exhausted\":false,\"output\":\"hi\\n\",\"solutions\":[]}"
        );
    }

    #[test]
    fn json_error_object_is_escaped() {
        assert_eq!(
            String::from_utf8(bytes(|w| {
                (enc("json").write_error)(w, &WireError::Parse("a\"b".into()))
            }))
            .unwrap(),
            "{\"error\":\"a\\\"b\"}"
        );
    }

    #[test]
    fn descriptors_are_named_and_have_distinct_streaming() {
        assert_eq!(PLG_ENC_JSON.name, "json");
        assert_eq!(PLG_ENC_BSON.name, "bson");
        assert!((PLG_ENC_JSON.can_stream)());
        assert!(!(PLG_ENC_BSON.can_stream)());
    }

    #[test]
    fn find_locates_advertised_encoders() {
        let caps: [*const EncoderDesc; 2] = [&PLG_ENC_JSON, &PLG_ENC_BSON];
        let json = unsafe { EncoderDesc::find(caps.as_ptr(), caps.len(), "json") };
        let bson = unsafe { EncoderDesc::find(caps.as_ptr(), caps.len(), "bson") };
        let none = unsafe { EncoderDesc::find(caps.as_ptr(), caps.len(), "csv") };
        assert_eq!(json.unwrap().name, "json");
        assert_eq!(bson.unwrap().name, "bson");
        assert!(none.is_none(), "unadvertised encoder not found");
    }

    #[test]
    fn find_returns_none_for_encoder_omitted_from_table() {
        // A [json]-only capability table must not resolve bson.
        let caps: [*const EncoderDesc; 1] = [&PLG_ENC_JSON];
        let bson = unsafe { EncoderDesc::find(caps.as_ptr(), caps.len(), "bson") };
        assert!(bson.is_none(), "bson resolves despite not being advertised");
    }

    // ── bson structure ──────────────────────────────────────────────────────

    fn bson_doc_len(buf: &[u8]) -> i32 {
        i32::from_le_bytes(buf[0..4].try_into().unwrap())
    }

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
        let buf = bytes(|w| (enc("bson").write_envelope)(w, &m, &e));
        assert_valid_bson_doc(&buf);
        assert!(contains_cstring_key(&buf, b"count"));
        assert!(contains_cstring_key(&buf, b"exhausted"));
        assert!(contains_cstring_key(&buf, b"solutions"));
    }

    #[test]
    fn bson_error_document_is_valid() {
        let buf = bytes(|w| (enc("bson").write_error)(w, &WireError::Runtime("boom".into())));
        assert_valid_bson_doc(&buf);
        assert!(contains_cstring_key(&buf, b"error"));
    }

    fn contains_cstring_key(buf: &[u8], key: &[u8]) -> bool {
        let mut needle = key.to_vec();
        needle.push(0x00);
        buf.windows(needle.len()).any(|w| w == needle.as_slice())
    }

    // ── TermBuf framing round-trip (the losslessness claim) ─────────────────

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

    // ── bson input parsing ────────────────────────────────────────────────

    /// Build a bson request document with hand-written field helpers.
    fn req_doc(fields: &[(u8, &str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        let start = bson_doc_begin(&mut buf);
        for (ty, key, val_bytes) in fields {
            buf.push(*ty);
            bson_cstring(&mut buf, key);
            buf.extend_from_slice(val_bytes);
        }
        bson_doc_end(&mut buf, start);
        buf
    }
    fn bson_int32(n: i32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }
    fn bson_int64(n: i64) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }
    fn bson_str(s: &str) -> Vec<u8> {
        let mut v = (s.len() as i32 + 1).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0x00);
        v
    }

    #[test]
    fn parses_a_query_only_request() {
        let doc = req_doc(&[(T_STRING, "query", &bson_str("parent(tom, X)"))]);
        let r = parse_bson_request(&doc).unwrap();
        assert_eq!(r.query, "parent(tom, X)");
        assert!(r.limit.is_none());
    }

    #[test]
    fn parses_query_and_int32_limit() {
        let doc = req_doc(&[
            (T_STRING, "query", &bson_str("p(X)")),
            (T_INT32, "limit", &bson_int32(5)),
        ]);
        let r = parse_bson_request(&doc).unwrap();
        assert_eq!(r.query, "p(X)");
        assert_eq!(r.limit, Some(5));
    }

    #[test]
    fn parses_query_and_int64_limit() {
        let doc = req_doc(&[
            (T_INT64, "limit", &bson_int64(42)),
            (T_STRING, "query", &bson_str("p(X)")),
        ]);
        let r = parse_bson_request(&doc).unwrap();
        assert_eq!(r.query, "p(X)");
        assert_eq!(r.limit, Some(42));
    }

    #[test]
    fn ignores_unknown_fields_for_forward_compat() {
        let doc = req_doc(&[
            (T_STRING, "caller", &bson_str("an-unknown-field")),
            (T_STRING, "query", &bson_str("ok")),
        ]);
        let r = parse_bson_request(&doc).unwrap();
        assert_eq!(r.query, "ok");
    }

    #[test]
    fn missing_query_is_an_error() {
        let doc = req_doc(&[(T_INT32, "limit", &bson_int32(3))]);
        let err = parse_bson_request(&doc).unwrap_err();
        assert!(err.contains("missing required 'query'"), "{err}");
    }

    #[test]
    fn truncated_document_is_an_error() {
        // declare a length larger than the buffer
        let mut doc = vec![99u8, 0, 0, 0]; // length = 99
        doc.extend_from_slice(b"\x00");
        assert!(parse_bson_request(&doc).is_err());
    }

    #[test]
    fn trailing_bytes_after_document_are_ignored() {
        let mut doc = req_doc(&[(T_STRING, "query", &bson_str("q"))]);
        doc.extend_from_slice(b"garbage trailing bytes"); // stdin may carry these
        let r = parse_bson_request(&doc).unwrap();
        assert_eq!(r.query, "q");
    }
}
