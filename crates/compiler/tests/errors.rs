//! Error surfaces: existence_error for undefined predicates, the
//! dynamic-predicate silent-fail contract, the uncatchable step limit,
//! query-time parse errors (exit 2), and compile-time (program) parse
//! errors surfaced by `plgc build` (exit 3, surface-lexeme messages).
//!
//! Parse-error wording: plgc's query parser phrases trailing junk as
//! "unexpected input at column N". The behavioral contract (trailing
//! junk => exit-2 parse error) is what's asserted, via exit-2 + "Parse
//! error" checks, not exact wording.

mod harness;
use harness::{Compiled, compile};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

const PROG: &str = "\
:- dynamic(field/1).
color(red). color(blue).
loop :- loop.
go(X) :- missing(X).
ok(yes).
";

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(PROG))
}

/// Shell out to plgc directly to capture a *build* failure's stderr/exit
/// (the harness's `compile` panics on build failure; here we inspect it).
fn try_build(source: &str) -> (String, i32) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("prog.pl");
    std::fs::write(&src, source).expect("write source");
    let bin = dir.path().join("prog");
    let out = Command::new(env!("CARGO_BIN_EXE_plgc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run plgc");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ---- existence errors / dynamic / empty KB ---------------------------

#[test]
fn undefined_predicate_is_existence_error() {
    // no matching predicate + empty knowledge base.
    let (out, code) = prog().query("shape(X)", &[]);
    assert!(out.contains("existence_error"), "{out}");
    assert_eq!(code, 3);
    // Empty-ish KB: querying an undefined predicate still raises.
    let c = compile("just_a_fact.\n");
    let (out, code) = c.query("foo(X)", &[]);
    assert!(out.contains("existence_error"), "{out}");
    assert_eq!(code, 3);
}

#[test]
fn dynamic_predicate_silently_fails() {
    // a dynamic predicate silently fails when undefined.
    let (out, code) = prog().query("field(X)", &[]);
    assert_eq!(out, "false.\n");
    assert_eq!(code, 0);
}

#[test]
fn ground_index_miss_fails() {
    // index ground miss returns empty.
    let (out, code) = prog().query("color(purple)", &[]);
    assert_eq!(out, "false.\n");
    assert_eq!(code, 0);
}

#[test]
fn undefined_in_rule_body_raises_when_reached() {
    // no matching predicate (through a rule body).
    let (out, code) = prog().query("go(X)", &[]);
    assert!(
        out.contains("existence_error(procedure, /(missing, 1))"),
        "{out}"
    );
    assert_eq!(code, 3);
    let (_, code) = prog().query("ok(X)", &[]);
    assert_eq!(code, 1);
}

// ---- step limit ------------------------------------------------------

#[test]
fn step_limit_is_uncatchable() {
    // the step limit prevents stack overflow + is not caught +
    //    naf_step_limit_returns_error_not_success.
    let (out, code) = prog().query("loop", &[]);
    assert!(out.contains("resource_error(steps)"), "{out}");
    assert!(out.contains("Maximum step limit exceeded"), "{out}");
    assert_eq!(code, 3);
    // catch/3 must NOT trap the resource error.
    let (out, code) = prog().query("catch(loop, _, true)", &[]);
    assert!(out.contains("resource_error(steps)"), "{out}");
    assert_eq!(code, 3);
    // \+ must surface the resource error, not treat it as failure.
    let (out, code) = prog().query("\\+(loop)", &[]);
    assert!(out.contains("resource_error(steps)"), "{out}");
    assert_eq!(code, 3);
}

// ---- query-time parse errors (exit 2) --------------------------------

#[test]
fn query_parse_errors_exit_2() {
    // trailing junk after a complete query is an error.
    // plgc phrases these as "Parse error: unexpected input at column N"
    // (see ADAPTATION NOTE); we assert the behavioral contract.
    for q in [
        "member(X,[1,2,3]) zzz",
        "p(X) ]",
        "p(X) . extra",
        "p(X) trailing",
        "X is 1, foo bar",
    ] {
        let (out, code) = prog().query(q, &[]);
        assert!(out.starts_with("error: Parse error:"), "query {q}: {out}");
        assert_eq!(code, 2, "query: {q}");
    }
}

#[test]
fn valid_queries_still_parse() {
    // optional trailing `.`
    // is accepted; the bare goal is accepted.
    // NOTE: the `?- color(X).` form is NOT
    // ported — plgc's `--query` takes a bare goal and rejects the `?-`
    // directive prefix (CLI-surface difference, see top-of-file skip list).
    for q in ["color(X)", "color(X)."] {
        let (out, code) = prog().query(q, &[]);
        assert!(
            out.contains("X = red") && out.contains("X = blue"),
            "query {q}: {out}"
        );
        assert_eq!(code, 1, "query: {q}");
    }
}

// ---- compile-time (program) parse errors — issue #20 -----------------

#[test]
fn program_parse_errors_show_surface_lexemes() {
    // parse errors show the punctuation lexeme / word-operator lexeme /
    //    atom_lexeme / eof_is_phrased_in_words / expected_includes_surface_lexeme
    //    + test_parse_error_detection. Ported as `plgc build` failures.

    // Stray `]` — backtick the lexeme, never the internal RBracket variant.
    let (err, code) = try_build("p(]).\n");
    assert!(err.contains("`]`"), "{err}");
    assert!(!err.contains("RBracket"), "{err}");
    assert_ne!(code, 0);

    // `mod` in primary position names the word-op as `mod`, not `Mod`.
    let (err, _) = try_build("p :- X is mod 3.\n");
    assert!(err.contains("`mod`"), "{err}");

    // `foo bar.` — the offending token is shown as atom `bar`.
    let (err, _) = try_build("foo bar.\n");
    assert!(err.contains("atom `bar`"), "{err}");

    // Unterminated arg list — EOF phrased in words, expected token shown as `)`.
    let (err, _) = try_build("p(x\n");
    assert!(err.contains("end of input"), "{err}");
    assert!(!err.contains("Eof"), "{err}");
    assert!(err.contains("`)`"), "{err}");
    assert!(!err.contains("RParen"), "{err}");

    // Generic malformed program is rejected.
    let (_, code) = try_build("invalid(((.\n");
    assert_ne!(code, 0);
}

// Smoke: try_build succeeds on a valid program (and the binary path exists).
#[test]
fn try_build_accepts_valid_program() {
    let (err, code) = try_build("ok(yes).\n");
    assert_eq!(code, 0, "stderr: {err}");
}

// Reference the harness path env so the binary is always built first
// (compile() already does, but keep this explicit for the shell-out path).
#[test]
fn plgc_binary_exists() {
    assert!(Path::new(env!("CARGO_BIN_EXE_plgc")).exists());
}
