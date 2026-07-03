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
