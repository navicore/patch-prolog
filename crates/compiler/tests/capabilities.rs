//! Capability-table integration (docs/design/IO.md): a binary advertises a
//! declared set of wire encodings via `:- io_format([...])` (default `[json]`),
//! `--format` is validated against it, and encoders not declared are
//! dead-stripped from the binary.

mod harness;
use harness::compile;
use std::process::Command;
use std::sync::OnceLock;

/// Default binary (no `io_format` directive) — advertises `[json]` only.
fn default_prog() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile("parent(tom, bob).\n"))
}

/// Declares `[json, bson]` via a list.
fn both_list() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([json, bson]).\nparent(tom, bob).\n"))
}

/// Declares `[json, bson]` via a comma-chain.
fn both_comma() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format((json, bson)).\nparent(tom, bob).\n"))
}

/// Declares `[bson]` only.
fn bson_only() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([bson]).\nf(a).\n"))
}

/// The default `[json]` binary still answers json byte-identically (the wire
/// contract is preserved for the default capability set).
#[test]
fn default_binary_answers_json() {
    let (out, code) = default_prog().query("parent(tom, X)", &["--format", "json"]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"bob\"}]}\n"
    );
    assert_eq!(code, 1);
}

/// A format the binary doesn't advertise is rejected at the CLI (exit 2),
/// whether the encoder doesn't exist or just isn't declared.
#[test]
fn default_binary_rejects_undeclared_bson() {
    let (out, code) = default_prog().query("parent(tom, X)", &["--format", "bson"]);
    assert_eq!(code, 2, "undeclared format → exit 2");
    assert!(out.is_empty(), "usage error emits nothing on stdout");
}

#[test]
fn unknown_format_rejected() {
    let (out, code) = default_prog().query("parent(tom, X)", &["--format", "nonsense"]);
    assert_eq!(code, 2);
    assert!(out.is_empty());
}

/// The human `text` form is always available (display rendering, not a wire
/// encoding gated by the capability table).
#[test]
fn human_text_always_available() {
    let (out, code) = default_prog().query("parent(tom, X)", &["--format", "text"]);
    assert_eq!(code, 1);
    assert!(out.contains("X = bob"), "human form renders: {out}");
}

/// A `[json, bson]` binary answers both.
#[test]
fn both_list_serves_json_and_bson() {
    let (json, _) = both_list().query("parent(tom, X)", &["--format", "json"]);
    assert!(json.contains("\"X\":\"bob\""));
    let (bson, code) = both_list().query_bytes("parent(tom, X)", &["--format", "bson"]);
    assert_eq!(code, 1, "bson on a [json,bson] binary succeeds");
    // bson document is self-delimiting: leading int32 length == buffer length.
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len(), "bson self-delimits");
}

/// The comma-chain directive form is equivalent to the list form.
#[test]
fn comma_chain_directive_works() {
    let (bson, code) = both_comma().query_bytes("parent(tom, X)", &["--format", "bson"]);
    assert_eq!(code, 1);
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len());
}

/// A `[bson]`-only binary serves bson but rejects the undeclared json format.
#[test]
fn bson_only_rejects_json() {
    let (_, code) = bson_only().query("f(X)", &["--format", "json"]);
    assert_eq!(code, 2, "json on a [bson]-only binary → exit 2");
    let (bson, code) = bson_only().query_bytes("f(X)", &["--format", "bson"]);
    assert_eq!(code, 1);
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len());
}

/// An unknown encoder NAME in the directive is a build-time error.
#[test]
fn unknown_encoder_name_is_build_error() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("bad.pl");
    std::fs::write(&src, ":- io_format([csv]).\nf(a).\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_plgc"))
        .args(["build"])
        .arg(&src)
        .arg("-o")
        .arg(dir.path().join("bad"))
        .output()
        .unwrap();
    assert!(!out.status.success(), "build must fail on unknown encoder");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("io_format: unknown encoder `csv`"),
        "error names the bad encoder: {stderr}"
    );
}

/// Dead-stripping: a binary advertising only json must not link the bson
/// encoder descriptor; a bson-only binary must not link json. (IO.md
/// checkpoint — a [text]-only binary pays zero bson cost.)
#[test]
fn undeclared_encoders_are_dead_stripped() {
    use std::process::Command;
    let has = |bin: &str, sym: &str| -> bool {
        let o = Command::new("nm").arg(bin).output().unwrap();
        String::from_utf8_lossy(&o.stdout).contains(sym)
    };
    // default [json] binary: PLG_ENC_JSON present, PLG_ENC_BSON stripped.
    assert!(has(&default_prog().bin.to_string_lossy(), "PLG_ENC_JSON"));
    assert!(
        !has(&default_prog().bin.to_string_lossy(), "PLG_ENC_BSON"),
        "bson encoder dead-stripped from a json-only binary"
    );
    // [bson]-only binary: PLG_ENC_BSON present, PLG_ENC_JSON stripped.
    assert!(has(&bson_only().bin.to_string_lossy(), "PLG_ENC_BSON"));
    assert!(
        !has(&bson_only().bin.to_string_lossy(), "PLG_ENC_JSON"),
        "json encoder dead-stripped from a bson-only binary"
    );
}

// ── bson input (IO.md: the one-field request document) ─────────────────────

/// Build a bson request `{query, limit?}` from raw bytes (no bson dep).
fn bson_request(query: &str, limit: Option<i64>) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x02);
    body.extend_from_slice(b"query\0");
    let qb = query.as_bytes();
    body.extend_from_slice(&(qb.len() as i32 + 1).to_le_bytes());
    body.extend_from_slice(qb);
    body.push(0x00);
    if let Some(n) = limit {
        body.push(0x10); // int32
        body.extend_from_slice(b"limit\0");
        body.extend_from_slice(&(n as i32).to_le_bytes());
    }
    let total = body.len() + 5; // length prefix (4) + trailing null (1)
    let mut doc = (total as i32).to_le_bytes().to_vec();
    doc.extend_from_slice(&body);
    doc.push(0x00);
    doc
}

/// bson-in / json-out: a request document drives the query; argv selects the
/// output format. (IO.md orthogonality — input and output encodings are
/// independent.)
#[test]
fn bson_input_drives_query_with_json_output() {
    let req = bson_request("parent(tom, X)", None);
    let (out, code) =
        both_list().run_with_stdin(&["--input-format", "bson", "--format", "json"], &req);
    assert_eq!(code, 1);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("\"X\":\"bob\""),
        "query ran from bson request: {out}"
    );
}

/// A `limit` in the bson request is honored (surfaces as `exhausted:false`
/// when the limit is hit).
#[test]
fn bson_input_limit_is_honored() {
    let req = bson_request("parent(tom, X)", Some(1));
    let (out, _) =
        both_list().run_with_stdin(&["--input-format", "bson", "--format", "json"], &req);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("\"exhausted\":false"),
        "limit hit ⇒ not exhausted: {out}"
    );
    assert!(out.contains("\"count\":1"), "limit 1 ⇒ one solution: {out}");
}

/// bson-in / bson-out (both directions binary).
#[test]
fn bson_in_bson_out() {
    let req = bson_request("parent(tom, X)", None);
    let (bson, code) =
        both_list().run_with_stdin(&["--input-format", "bson", "--format", "bson"], &req);
    assert_eq!(code, 1);
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len(), "bson-out self-delimits");
}

/// A [json]-only binary rejects bson input (capability gates both directions).
#[test]
fn json_only_binary_rejects_bson_input() {
    let req = bson_request("parent(tom, X)", None);
    let (_out, code) =
        default_prog().run_with_stdin(&["--input-format", "bson", "--format", "json"], &req);
    assert_eq!(code, 2, "bson input on a json-only binary ⇒ exit 2");
}

/// A request missing the required `query` string is a usage error (exit 2).
#[test]
fn bson_request_missing_query_is_rejected() {
    let mut body = vec![0x10];
    body.extend_from_slice(b"limit\0");
    body.extend_from_slice(&3i32.to_le_bytes());
    let mut doc = ((body.len() + 5) as i32).to_le_bytes().to_vec();
    doc.extend_from_slice(&body);
    doc.push(0x00);
    let (_out, code) =
        both_list().run_with_stdin(&["--input-format", "bson", "--format", "json"], &doc);
    assert_eq!(code, 2, "missing query ⇒ exit 2");
}

/// argv `--query` still works (text-input mode is the default and untouched).
#[test]
fn argv_query_still_works_in_bson_binary() {
    let (out, code) = both_list().query("parent(tom, X)", &["--format", "json"]);
    assert_eq!(code, 1);
    assert!(out.contains("\"X\":\"bob\""));
}

// ── bson error path (review #1 + #2: errors route through the output encoder) ─

/// A bson cstring-key scan (sufficient for these fixtures, not a parser).
fn bson_has_key(buf: &[u8], key: &[u8]) -> bool {
    let mut needle = key.to_vec();
    needle.push(0x00);
    buf.windows(needle.len()).any(|w| w == needle.as_slice())
}

/// A runtime error (undefined non-dynamic predicate ⇒ existence_error, exit 3)
/// is emitted as a valid bson error document on stdout — not plaintext stderr.
#[test]
fn bson_error_path_on_runtime_error() {
    let (bson, code) = both_list().query_bytes("no_such_pred(X)", &["--format", "bson"]);
    assert_eq!(code, 3, "undefined predicate ⇒ runtime error ⇒ exit 3");
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len(), "bson error doc self-delimits");
    assert!(bson_has_key(&bson, b"error"), "carries an error field");
}

/// A query-parse error (exit 2) is also emitted as a bson error document.
#[test]
fn bson_error_path_on_parse_error() {
    let (bson, code) = both_list().query_bytes("bad(", &["--format", "bson"]);
    assert_eq!(code, 2, "malformed query ⇒ parse error ⇒ exit 2");
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(len as usize, bson.len(), "bson error doc self-delimits");
    assert!(bson_has_key(&bson, b"error"));
}

/// A malformed bson *request* (exit 2) is encoded in the chosen output format,
/// not dropped to stderr — the "I only speak bson" contract at the framing
/// layer. (Review #2.)
#[test]
fn bson_request_parse_error_is_encoded_not_stderr() {
    let (bson, code) =
        both_list().run_with_stdin(&["--input-format", "bson", "--format", "bson"], b"not bson");
    assert_eq!(code, 2);
    // stdout carries a valid bson error document (the plaintext did NOT go to
    // stderr): self-delimiting length matches.
    let len = i32::from_le_bytes(bson[..4].try_into().unwrap());
    assert_eq!(
        len as usize,
        bson.len(),
        "malformed-request error encoded as bson"
    );
    assert!(bson_has_key(&bson, b"error"));
}
