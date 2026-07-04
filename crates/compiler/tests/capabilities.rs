//! Capability-table integration (docs/design/IO.md): a binary advertises a
//! declared set of wire encodings via `:- io_format([...])` (default `[text]`),
//! `--format`/`--input-format` are validated against it, encoders not declared
//! are dead-stripped, and the engine speaks **text + bson, no JSON**.

mod harness;
use harness::compile;
use std::process::Command;
use std::sync::OnceLock;

/// Default binary (no `io_format` directive) — advertises the default
/// `[text, bson]` (bson is a core format, available without a directive).
fn default_prog() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile("parent(tom, bob).\nparent(tom, liz).\n"))
}

/// Declares `[text, bson]` explicitly (same as the default).
fn both() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([text, bson]).\nparent(tom, bob).\nparent(tom, liz).\n"))
}

/// Declares `[bson]` only — restriction forces bson, sheds text.
fn bson_only() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([bson]).\nf(a).\n"))
}

/// Declares `[text]` only — restriction forces text, sheds bson.
fn text_only() -> &'static harness::Compiled {
    static C: OnceLock<harness::Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([text]).\nf(a).\n"))
}

#[test]
fn default_binary_serves_text_and_bson() {
    // bson is a core format: a freshly-built binary speaks it without a
    // directive. text stays the default OUTPUT (human-readable).
    let (out, code) = default_prog().query("parent(tom, X)", &[]);
    assert_eq!(out, "X = bob\nX = liz\n");
    assert_eq!(code, 1);
    let (env, code) = default_prog().query_bson("parent(tom, X)", &[]);
    assert_eq!(code, 1);
    assert_eq!(env.count, Some(2));
}

#[test]
fn json_is_not_a_format() {
    // The engine speaks text + bson; json is not a wire format.
    let (out, code) = both().query("parent(tom, X)", &["--format", "json"]);
    assert_eq!(code, 2);
    assert!(out.is_empty());
}

#[test]
fn both_serves_text_and_bson() {
    let (text, code) = both().query("parent(tom, X)", &["--format", "text"]);
    assert_eq!(code, 1);
    assert_eq!(text, "X = bob\nX = liz\n");
    let (env, code) = both().query_bson("parent(tom, X)", &[]);
    assert_eq!(code, 1);
    assert_eq!(env.count, Some(2));
    assert_eq!(env.exhausted, Some(true));
}

#[test]
fn bson_limit_is_honored() {
    let (env, _) = both().query_bson("parent(tom, X)", &["--limit", "1"]);
    assert_eq!(env.count, Some(1));
    assert_eq!(env.exhausted, Some(false), "limit hit ⇒ not exhausted");
}

#[test]
fn bson_only_serves_bson_and_rejects_text() {
    let (_, code) = bson_only().query("f(X)", &["--format", "text"]);
    assert_eq!(code, 2, "text on a [bson]-only binary ⇒ exit 2");
    let (env, code) = bson_only().query_bson("f(X)", &[]);
    assert_eq!(code, 1);
    assert_eq!(env.count, Some(1));
}

#[test]
fn text_only_rejects_bson() {
    // A program can RESTRICT to text-only via the directive; bson is then
    // undeclared and rejected (and dead-stripped — see below).
    let (out, code) = text_only().query("f(X)", &["--format", "bson"]);
    assert_eq!(code, 2, "bson on a text-only binary ⇒ exit 2");
    assert!(out.is_empty());
}

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
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("io_format: unknown encoder `csv`"),
        "error names the bad encoder"
    );
}

/// bson input: the one-field `{query, limit?}` request document.
fn bson_request(query: &str, limit: Option<i64>) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x02);
    body.extend_from_slice(b"query\0");
    let qb = query.as_bytes();
    body.extend_from_slice(&(qb.len() as i32 + 1).to_le_bytes());
    body.extend_from_slice(qb);
    body.push(0x00);
    if let Some(n) = limit {
        body.push(0x10);
        body.extend_from_slice(b"limit\0");
        body.extend_from_slice(&(n as i32).to_le_bytes());
    }
    let total = body.len() + 5;
    let mut doc = (total as i32).to_le_bytes().to_vec();
    doc.extend_from_slice(&body);
    doc.push(0x00);
    doc
}

#[test]
fn bson_input_drives_query_with_text_output() {
    let req = bson_request("parent(tom, X)", None);
    let (out, code) = both().run_with_stdin(&["--input-format", "bson", "--format", "text"], &req);
    assert_eq!(code, 1);
    assert_eq!(out, b"X = bob\nX = liz\n");
}

#[test]
fn bson_input_limit_honored() {
    let req = bson_request("parent(tom, X)", Some(1));
    let (env, _) =
        both().run_with_stdin_bson(&["--input-format", "bson", "--format", "bson"], &req);
    assert_eq!(env.count, Some(1));
    assert_eq!(env.exhausted, Some(false));
}

#[test]
fn default_accepts_bson_input() {
    // bson input works out of the box (default advertises bson).
    let req = bson_request("parent(tom, X)", None);
    let (out, code) =
        default_prog().run_with_stdin(&["--input-format", "bson", "--format", "text"], &req);
    assert_eq!(code, 1);
    assert_eq!(out, b"X = bob\nX = liz\n");
}

#[test]
fn text_only_rejects_bson_input() {
    let req = bson_request("f(a)", None);
    let (_out, code) =
        text_only().run_with_stdin(&["--input-format", "bson", "--format", "text"], &req);
    assert_eq!(code, 2, "bson input on a text-only binary ⇒ exit 2");
}

#[test]
fn argv_query_still_works_in_both_binary() {
    let (out, code) = both().query("parent(tom, X)", &["--format", "text"]);
    assert_eq!(code, 1);
    assert_eq!(out, "X = bob\nX = liz\n");
}

/// Dead-stripping: a default binary links BOTH core encoders (bson is core,
/// not opt-in); a RESTRICTED binary sheds what it doesn't advertise. No JSON
/// encoder exists anywhere.
#[test]
fn dead_stripping_follows_the_directive() {
    let has = |bin: &std::path::Path, sym: &str| -> bool {
        let o = Command::new("nm").arg(bin).output().unwrap();
        String::from_utf8_lossy(&o.stdout).contains(sym)
    };
    // Default: both linked (bson is available without a directive).
    assert!(has(&default_prog().bin, "PLG_ENC_TEXT"));
    assert!(has(&default_prog().bin, "PLG_ENC_BSON"));
    // Restrict to text-only ⇒ bson dead-stripped.
    assert!(has(&text_only().bin, "PLG_ENC_TEXT"));
    assert!(
        !has(&text_only().bin, "PLG_ENC_BSON"),
        "bson stripped from a text-only binary"
    );
    // Restrict to bson-only ⇒ text dead-stripped.
    assert!(has(&bson_only().bin, "PLG_ENC_BSON"));
    assert!(
        !has(&bson_only().bin, "PLG_ENC_TEXT"),
        "text stripped from a bson-only binary"
    );
    // No JSON encoder exists anywhere.
    assert!(!has(&default_prog().bin, "PLG_ENC_JSON"));
}

/// bson error path: a runtime error under `--format bson` emits a valid bson
/// error document on stdout (not plaintext stderr).
#[test]
fn bson_error_path_on_runtime_error() {
    let (env, code) = both().query_bson("no_such_pred(X)", &["--format", "bson"]);
    assert_eq!(code, 3, "undefined predicate ⇒ runtime error ⇒ exit 3");
    assert!(env.error.is_some(), "error encoded as bson, not stderr");
}
