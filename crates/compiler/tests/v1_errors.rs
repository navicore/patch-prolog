//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Error surfaces: existence_error for undefined predicates, the
//! dynamic-predicate silent-fail contract, the uncatchable step limit,
//! query-time parse errors (exit 2), and compile-time (program) parse
//! errors surfaced by `plgc build` (exit 3, surface-lexeme messages —
//! v1 issue #20).
//!
//! ADAPTATION NOTE (instruction #3): v1's query-parser produced phrased
//! messages like "after query" and named the offending token; plgc's
//! query parser phrases trailing junk as "unexpected input at column N".
//! The BEHAVIORAL contract (trailing junk => exit-2 parse error) is
//! identical, so the issue-#30 tests are ported as exit-2 + "Parse error"
//! assertions rather than asserting v1's exact wording.
//!
//! =====================================================================
//! SKIP LIST (v1 tests NOT ported — central record for all v1_*.rs files)
//! =====================================================================
//!
//! A. In-process library-API unit tests with NO CLI/wire analog. These
//!    drive `Parser`/`Solver`/`CompiledDatabase` directly and assert on
//!    internal types; plgc exposes only the compiled-binary wire surface:
//!      - test_parse_error_detection (Parser::parse_program is_err) —
//!        COVERED behaviorally by program_parse_errors_show_surface_lexemes.
//!      - test_depth_limit_custom (.with_max_depth(50) API)
//!      - test_integer_overflow_detected (constructs i64::MAX query via API;
//!        the wire path IS covered in v1_arith::arithmetic_error_terms)
//!      - test_division_by_zero / test_unbound_variable_in_arithmetic (API
//!        form; wire form covered in v1_arith)
//!      - test_step_limit_in_try_solve_once / test_naf_step_limit_returns_error_not_success
//!        / test_findall_step_limit_returns_error / test_findall_steps_accumulate_globally
//!        / test_between_step_limit_in_findall / test_between_step_limit_in_negation
//!        / test_catch_does_not_catch_step_limit (all .with_max_depth(N) API;
//!        the default-limit wire path is covered by step_limit_is_uncatchable)
//!      - test_float_overflow_literal_rejected (Parser API; plgc rejects the
//!        same literal at build/query time but the unit test is API-shaped)
//!
//! B. v1 parser-SURFACE features plgc's ISO-subset frontend does NOT
//!    implement — these are genuine surface divergences, reported for
//!    triage (see final report), not ported as passing tests:
//!      - Issue #19/#28 operator-as-atom & prefix +/\ :
//!        test_op_atom_in_list, test_op_atom_in_parens_after_equals,
//!        test_op_atom_not_operator_as_atom, test_op_atom_minus_alone_in_parens,
//!        test_prefix_plus_on_atom_builds_compound, _folds_integer_literal,
//!        _folds_float_literal, test_prefix_backslash_on_atom_builds_compound,
//!        _on_integer_does_not_fold, test_backslash_as_atom_in_closing_context,
//!        test_prefix_plus_minus_chain, test_op_atom_univ_roundtrip,
//!        test_op_atom_pow_in_arg_position, test_op_atom_xor_in_arg_position,
//!        test_op_atom_new_ops_in_list, test_op_atom_word_op_in_arg_position,
//!        test_op_atom_plus_in_arg_position.
//!        (`p(+).` etc. → "Parse error", `X is + 3` → "Parse error".)
//!      - Issue #29 `:` and `^`-as-bare-op term tests:
//!        test_op_colon_parses_as_term, test_op_colon_right_associative.
//!        (`a:b:c` and `pkg:expr` → "Parse error".)
//!    Ported variants that DO work on plgc (e.g. infix `**`, `^`, `<<`,
//!    `xor`, `/\`, `\/`, `(<)` as atom) live in v1_arith / elsewhere.
//!
//! C. v1-runner CLI-surface details plgc phrases/handles differently:
//!      - test_query_valid_with_query_op_still_works (`?- Goal.` form):
//!        plgc's --query takes a bare goal and rejects the `?-` prefix.
//!      - The issue-#30 named-token wording (see ADAPTATION NOTE above).

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
    // v1 test_no_matching_predicate + test_empty_knowledge_base.
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
    // v1 test_dynamic_predicate_silently_fails_when_undefined.
    let (out, code) = prog().query("field(X)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
}

#[test]
fn ground_index_miss_fails() {
    // v1 test_index_ground_miss_returns_empty.
    let (out, code) = prog().query("color(purple)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
}

#[test]
fn undefined_in_rule_body_raises_when_reached() {
    // v1 test_no_matching_predicate (through a rule body).
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
    // v1 test_depth_limit_prevents_stack_overflow + does_not_catch_step_limit +
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
    // v1 issue #30 family: trailing junk after a complete query is an error.
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
        assert!(
            out.starts_with("{\"error\":\"Parse error:"),
            "query {q}: {out}"
        );
        assert_eq!(code, 2, "query: {q}");
    }
}

#[test]
fn valid_queries_still_parse() {
    // v1 test_query_valid_with_no_dot / with_dot. The optional trailing `.`
    // is accepted; the bare goal is accepted.
    // NOTE: v1's `?- color(X).` form (test_query_valid_with_query_op) is NOT
    // ported — plgc's `--query` takes a bare goal and rejects the `?-`
    // directive prefix (CLI-surface difference, see top-of-file skip list).
    for q in ["color(X)", "color(X)."] {
        let (out, code) = prog().query(q, &[]);
        assert!(out.contains("\"count\":2"), "query {q}: {out}");
        assert_eq!(code, 1, "query: {q}");
    }
}

// ---- compile-time (program) parse errors — issue #20 -----------------

#[test]
fn program_parse_errors_show_surface_lexemes() {
    // v1 test_parse_err_shows_punctuation_lexeme / word_operator_lexeme /
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
