//! The wire layer: the fixed envelope *shape* as a typed value, plus plural
//! encodings exposed as **descriptors** (vtables of function pointers). Splits
//! what was collapsed in `core.rs` — where byte-emitters implicitly defined the
//! shape — so the shape is owned once (here) and the encodings vary
//! independently. See docs/design/IO.md.
//!
//! Two encodings, **no JSON**: **text** (the readable `X = foo` form — the
//! default, human/shell-facing) and **bson** (binary, dense, typed — the
//! machine-facing format). A host that wants JSON derives it from bson at the
//! host boundary; the engine itself never serializes JSON. Term values inside
//! bson are `BinData(0x00)` wrapping a `copyterm::TermBuf` — the same cell ABI
//! the fact tables and `copy_term/2` use, single-sourced in `plg-shared::cell`,
//! lossless including cyclic terms. Because a binary format can't coexist with
//! streamed text bytes, bson forces capture mode; the encoding dictates the
//! sink.
//!
//! **Why descriptors, not a trait.** Codegen bakes a per-binary *capability
//! table* — pointers to the descriptors the program declared via
//! `io_format/1` — and `entry.rs` dispatches through those pointers, never
//! naming an encoder statically. Link-time `--gc-sections` then strips any
//! encoder whose descriptor isn't in the table.

use crate::copyterm::{self, TermBuf};
use crate::machine::Machine;
use crate::render::RenderedSolution;
use std::io::{self, Write};

/// The fixed engine-output shape — the contract, made a type. Every encoding
/// reads the same fields; only their byte encoding varies.
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],
    /// Captured `write/1` bytes, when the sink is in capture mode. `None` when
    /// output streamed to stdout (the CLI text path). Encodings that can't
    /// stream require the caller to have run in capture mode so this is `Some`.
    pub program_output: Option<&'a str>,
    /// The atom map (id → name, id = array index), when the bson output is
    /// self-describing (`--atoms`). Lets a host decode bson term values (atom-
    /// id-keyed TermBuf) from the single document — no external atom source.
    /// Emitted post-query, so it covers query-introduced atoms too.
    pub atoms: Option<&'a [String]>,
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
            atoms: None,
        }
    }
}

/// The two failure classes the engine distinguishes, carrying the message the
/// wire contract puts on the wire. Callers construct the *correct* variant
/// (`Parse` vs `Runtime`) so the distinction is honest on the wire even though
/// today's encoders collapse both to the same bytes — a future bson `kind`
/// field will surface it. Callers map the class to their surface (exit codes
/// 2/3 for the CLI); the message bytes come from `core::run_query`.
pub enum WireError {
    Parse(String),
    Runtime(String),
}

/// An encoding as a vtable: a name plus the three operations the wire contract
/// needs. `#[repr(C)]` so codegen can reference a descriptor by address and
/// `entry.rs` can read its fields after dereferencing the pointer from the
/// capability table. A `#[no_mangle] static` of this type per encoding
/// (`PLG_ENC_TEXT`, `PLG_ENC_BSON`) is what codegen's `@plg_caps` table points
/// at; encoders not listed there are unreferenced and get dead-stripped.
#[repr(C)]
pub struct EncoderDesc {
    /// The `--format` name this descriptor answers to ("text", "bson").
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

// ── text: the readable wire encoding ────────────────────────────────────────
//
// The human/shell-facing form: `X = foo` per binding, `true.`/`false.` for
// binding-less / empty solutions. Projects the envelope to solutions only —
// `count`/`exhausted` live in the bson envelope; text is the simple, readable
// face. Streaming-safe (line-oriented, coexists with `write/1` on stdout).
// Like all text rendering, cyclic terms are cut (`X = f(X)` → `f(_N)`); bson
// round-trips them.

fn text_write_envelope(w: &mut dyn Write, _m: &Machine, e: &Envelope) -> io::Result<()> {
    if e.solutions.is_empty() {
        return w.write_all(b"false.\n");
    }
    for sol in e.solutions {
        if sol.bindings.is_empty() {
            w.write_all(b"true.\n")?;
            continue;
        }
        for b in &sol.bindings {
            writeln!(w, "{} = {}", b.name, b.text)?;
        }
    }
    Ok(())
}

fn text_write_error(w: &mut dyn Write, err: &WireError) -> io::Result<()> {
    let msg = match err {
        WireError::Parse(m) | WireError::Runtime(m) => m,
    };
    writeln!(w, "error: {msg}")
}

const fn text_can_stream() -> bool {
    true
}

/// The readable text wire encoding (default).
#[unsafe(no_mangle)]
pub static PLG_ENC_TEXT: EncoderDesc = EncoderDesc {
    name: "text",
    write_envelope: text_write_envelope,
    write_error: text_write_error,
    can_stream: text_can_stream,
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
// it):
//     { count: int32, exhausted: bool, output?: string,
//       solutions: [ { <var>: BinData(0x00, <TermBuf bytes>), ... }, ... ] }
//
// Scalars map to native bson types; term values are opaque `BinData(0x00)` (a
// caller speaking bson to a patch-prolog binary has opted into this engine's
// cell ABI). See `serialize_termbuf` for the payload.

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

/// Emit the `atoms` array element (id = index → name string) into `buf`.
/// Shared by the envelope's inline map and the standalone `--atoms` map.
fn bson_atoms_array(buf: &mut Vec<u8>, names: &[String]) {
    buf.push(T_ARRAY);
    bson_cstring(buf, "atoms");
    let arr = bson_doc_begin(buf);
    for (i, name) in names.iter().enumerate() {
        buf.push(T_STRING);
        bson_cstring(buf, &i.to_string());
        let len = i32::try_from(name.len() + 1).expect("atom name < 2GB");
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.push(0x00);
    }
    bson_doc_end(buf, arr);
}

/// Standalone `--atoms` bson output: `{count: N, atoms: [...]}` — just the atom
/// map (program atoms only; no query has run, so no query-introduced atoms).
/// For the case where a host wants the map once and its queries won't introduce
/// new atoms (docs/design/BSON_ATOMS.md).
pub fn write_atom_map_bson<W: Write>(w: &mut W, m: &Machine) -> io::Result<()> {
    let names: Vec<String> = (0..m.atoms.len())
        .map(|i| {
            m.atoms
                .try_resolve(i as u32)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let mut buf = Vec::new();
    let doc = bson_doc_begin(&mut buf);
    buf.push(T_INT32);
    bson_cstring(&mut buf, "count");
    buf.extend_from_slice(&(names.len().min(i32::MAX as usize) as i32).to_le_bytes());
    bson_atoms_array(&mut buf, &names);
    bson_doc_end(&mut buf, doc);
    w.write_all(&buf)
}

/// Standalone `--atoms` text output: `id\tname` per line (program atoms only).
pub fn write_atom_map_text<W: Write>(w: &mut W, m: &Machine) -> io::Result<()> {
    for i in 0..m.atoms.len() {
        let name = m.atoms.try_resolve(i as u32).unwrap_or_default();
        writeln!(w, "{i}\t{name}")?;
    }
    Ok(())
}

fn bson_write_envelope(w: &mut dyn Write, m: &Machine, e: &Envelope) -> io::Result<()> {
    let mut buf = Vec::new();
    let doc = bson_doc_begin(&mut buf);

    buf.push(T_INT32);
    bson_cstring(&mut buf, "count");
    buf.extend_from_slice(&(e.count.min(i32::MAX as usize) as i32).to_le_bytes());

    buf.push(T_BOOL);
    bson_cstring(&mut buf, "exhausted");
    buf.push(if e.exhausted { 0x01 } else { 0x00 });

    if let Some(out) = e.program_output {
        buf.push(T_STRING);
        bson_cstring(&mut buf, "output");
        let len = i32::try_from(out.len() + 1).expect("output string < 2GB");
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(out.as_bytes());
        buf.push(0x00);
    }

    // Self-describing mode (`--atoms`): the atom map (id = index) so a host
    // can resolve bson term atom ids from this same document. Post-query, so
    // it covers query-introduced atoms too.
    if let Some(names) = e.atoms {
        bson_atoms_array(&mut buf, names);
    }

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

fn bson_write_error(w: &mut dyn Write, err: &WireError) -> io::Result<()> {
    let msg = match err {
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
                let n = read_i64(body, off)?;
                limit = Some(n.max(0) as usize);
                off += 8;
            }
            _ => {
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

/// Advance past a value of known bson type, returning the new offset.
fn skip_value(buf: &[u8], off: usize, ty: u8) -> Result<usize, String> {
    match ty {
        0x01 => Ok(off + 8),                      // double
        T_STRING => Ok(read_string(buf, off)?.1), // string
        T_DOCUMENT | T_ARRAY => {
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
        T_INT64 => Ok(off + 8),
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

    fn bytes(f: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> Vec<u8> {
        let mut buf = Vec::new();
        f(&mut buf).unwrap();
        buf
    }

    // ── text ────────────────────────────────────────────────────────────────

    fn env_with(bindings: Vec<(&str, &str)>) -> Vec<RenderedSolution> {
        bindings
            .into_iter()
            .map(|(n, t)| RenderedSolution {
                bindings: vec![crate::render::Binding {
                    name: n.to_string(),
                    text: t.to_string(),
                    word: make_atom(0),
                }],
            })
            .collect()
    }

    #[test]
    fn text_empty_is_false() {
        let e = Envelope {
            count: 0,
            exhausted: true,
            solutions: &[],
            program_output: None,
            atoms: None,
        };
        assert_eq!(
            String::from_utf8(bytes(|w| (PLG_ENC_TEXT.write_envelope)(w, &machine(), &e))).unwrap(),
            "false.\n"
        );
    }

    #[test]
    fn text_renders_bindings_and_true() {
        let sols = env_with(vec![("X", "auth")]);
        let empty_sols = vec![RenderedSolution { bindings: vec![] }];
        // A bound solution renders `X = auth`; a binding-less one renders `true.`.
        let e1 = Envelope {
            count: 1,
            exhausted: false,
            solutions: &sols,
            program_output: None,
            atoms: None,
        };
        assert_eq!(
            String::from_utf8(bytes(|w| (PLG_ENC_TEXT.write_envelope)(w, &machine(), &e1)))
                .unwrap(),
            "X = auth\n"
        );
        let e2 = Envelope {
            count: 1,
            exhausted: true,
            solutions: &empty_sols,
            program_output: None,
            atoms: None,
        };
        assert_eq!(
            String::from_utf8(bytes(|w| (PLG_ENC_TEXT.write_envelope)(w, &machine(), &e2)))
                .unwrap(),
            "true.\n"
        );
    }

    #[test]
    fn descriptors_named_and_streaming() {
        assert_eq!(PLG_ENC_TEXT.name, "text");
        assert_eq!(PLG_ENC_BSON.name, "bson");
        assert!((PLG_ENC_TEXT.can_stream)());
        assert!(!(PLG_ENC_BSON.can_stream)());
    }

    #[test]
    fn find_locates_advertised_encoders() {
        let caps: [*const EncoderDesc; 2] = [&PLG_ENC_TEXT, &PLG_ENC_BSON];
        assert_eq!(
            unsafe { EncoderDesc::find(caps.as_ptr(), 2, "text") }
                .unwrap()
                .name,
            "text"
        );
        assert_eq!(
            unsafe { EncoderDesc::find(caps.as_ptr(), 2, "bson") }
                .unwrap()
                .name,
            "bson"
        );
        assert!(unsafe { EncoderDesc::find(caps.as_ptr(), 2, "json") }.is_none());
    }

    #[test]
    fn find_omitted_encoder_is_none() {
        let caps: [*const EncoderDesc; 1] = [&PLG_ENC_TEXT];
        assert!(unsafe { EncoderDesc::find(caps.as_ptr(), 1, "bson") }.is_none());
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
    fn bson_empty_envelope_self_delimits() {
        let m = machine();
        let e = Envelope {
            count: 0,
            exhausted: true,
            solutions: &[],
            program_output: None,
            atoms: None,
        };
        let buf = bytes(|w| (PLG_ENC_BSON.write_envelope)(w, &m, &e));
        assert_valid_bson_doc(&buf);
        assert!(contains_key(&buf, b"count"));
        assert!(contains_key(&buf, b"exhausted"));
        assert!(contains_key(&buf, b"solutions"));
    }

    #[test]
    fn bson_error_document_valid() {
        let buf = bytes(|w| (PLG_ENC_BSON.write_error)(w, &WireError::Runtime("boom".into())));
        assert_valid_bson_doc(&buf);
        assert!(contains_key(&buf, b"error"));
    }

    fn contains_key(buf: &[u8], key: &[u8]) -> bool {
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

    #[test]
    fn termbuf_framing_roundtrips_scalar_and_cycle() {
        let m = machine();
        let a = make_atom(7);
        let tb = copyterm::copy_to_buf(&m, a);
        assert!(tb.cells.is_empty());
        let rt = deserialize_termbuf(&serialize_termbuf(&tb));
        assert_eq!(rt.root, a);

        // X = f(X): bson must round-trip the cycle (text cuts it to f(_N)).
        let mut m = machine();
        let x = m.new_var();
        let s = {
            let i = m.heap.len();
            m.heap.push(pack_functor(3, 1));
            m.heap.push(x);
            make(TAG_STR, i as u64)
        };
        m.bind(payload(x) as usize, s);
        let tb = copyterm::copy_to_buf(&m, s);
        let rt = deserialize_termbuf(&serialize_termbuf(&tb));
        let restored = copyterm::restore_from_buf(&mut m, &rt);
        assert_eq!(tag_of(restored), TAG_STR);
        let ri = payload(restored) as usize;
        assert_eq!(
            m.deref(m.heap[ri + 1]),
            restored,
            "f(X) arg is the term itself"
        );
    }

    // ── bson input parsing ─────────────────────────────────────────────────

    fn req_doc(fields: &[(u8, &str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        let start = bson_doc_begin(&mut buf);
        for (ty, key, val) in fields {
            buf.push(*ty);
            bson_cstring(&mut buf, key);
            buf.extend_from_slice(val);
        }
        bson_doc_end(&mut buf, start);
        buf
    }
    fn bson_int32(n: i32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }
    fn bson_str(s: &str) -> Vec<u8> {
        let mut v = (s.len() as i32 + 1).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0x00);
        v
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
    fn ignores_unknown_fields() {
        let doc = req_doc(&[
            (T_STRING, "caller", &bson_str("x")),
            (T_STRING, "query", &bson_str("ok")),
        ]);
        assert_eq!(parse_bson_request(&doc).unwrap().query, "ok");
    }

    #[test]
    fn missing_query_is_an_error() {
        let doc = req_doc(&[(T_INT32, "limit", &bson_int32(3))]);
        assert!(
            parse_bson_request(&doc)
                .unwrap_err()
                .contains("missing required 'query'")
        );
    }

    // keep `make_int`/`ATOM_NIL` imports used for future list tests
    #[test]
    fn _keep_imports_used() {
        let _ = make_int(1);
        let _ = ATOM_NIL;
    }
}
