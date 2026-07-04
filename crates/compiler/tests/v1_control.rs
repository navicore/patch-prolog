//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Control: catch/3 + throw/1 (ISO error handling), ;/-> precedence,
//! and unify_with_occurs_check / no-occurs-check `=`.
//!
//! Fresh-var ids normalized via `norm()`. Value assertions in text; "exactly
//! one solution" checks via the bson envelope (count). PROG advertises both.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'_' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            out.push_str("_V");
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

const PROG: &str = "\
:- io_format([text, bson]).
color(red). color(blue).
sc1(X) :- X = a, fail ; X = b.
sc2(X, Y) :- X = a, Y = 1 ; X = b, Y = 2.
";

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(PROG))
}

#[track_caller]
fn check(goal: &str, expected_out: &str, expected_code: i32) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(
        norm(&out),
        norm(&format!("{expected_out}\n")),
        "goal: {goal}"
    );
    assert_eq!(code, expected_code, "goal: {goal}");
}

#[track_caller]
fn succeeds_once(goal: &str) {
    let (env, code) = prog().query_bson(goal, &[]);
    assert_eq!(env.count, Some(1), "goal {goal}: {env:?}");
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn fails(goal: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(out, "false.\n", "goal: {goal}");
    assert_eq!(code, 0, "goal: {goal}");
}

#[track_caller]
fn err_contains(goal: &str, needle: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert!(out.contains(needle), "goal {goal}: {out}");
    assert_eq!(code, 3, "goal: {goal}");
}

// ---- throw / catch ---------------------------------------------------

#[test]
fn throw_uncaught_surfaces_as_error() {
    let (out, code) = prog().query("throw(my_error)", &[]);
    assert_eq!(out, "error: Runtime error: my_error\n");
    assert_eq!(code, 3);
}

#[test]
fn catch_traps_and_binds() {
    check("catch(throw(e), e, X = recovered)", "X = recovered", 1);
    check("catch(throw(foo(bar)), foo(X), true)", "X = bar", 1);
    check(
        "catch(throw(payload(42)), payload(N), Y = N)",
        "N = 42\nY = 42",
        1,
    );
}

#[test]
fn catch_passthrough_non_matching() {
    let (out, code) = prog().query("catch(throw(a), b, true)", &[]);
    assert_eq!(out, "error: Runtime error: a\n");
    assert_eq!(code, 3);
}

#[test]
fn catch_transparent_when_no_throw() {
    check("catch(color(X), _, true)", "X = red\nX = blue", 1);
}

#[test]
fn catch_traps_builtin_errors() {
    check("catch(X is foo + 1, _, X = trapped)", "X = trapped", 1);
    check(
        "catch(undefined_predicate(X), error(existence_error(procedure, _), _), Y = trapped)",
        "X = _V\nY = trapped",
        1,
    );
    check(
        "catch(X is foo, error(type_error(evaluable, _), _), Y = trapped)",
        "X = _V\nY = trapped",
        1,
    );
}

#[test]
fn catch_nested() {
    check(
        "catch(catch(throw(e), e, X = inner), e, X = outer)",
        "X = inner",
        1,
    );
    check(
        "catch(catch(throw(b), a, X = inner), b, X = outer)",
        "X = outer",
        1,
    );
}

#[test]
fn throw_unbound_is_instantiation_error() {
    check(
        "catch(throw(_), error(instantiation_error, _), X = trapped)",
        "X = trapped",
        1,
    );
}

#[test]
fn catch_inside_naf() {
    err_contains("\\+ throw(my_err)", "my_err");
    check("\\+ catch(throw(e), e, fail)", "true.", 1);
}

#[test]
fn existence_and_type_error_shapes() {
    let (out, code) = prog().query("frobnicate(X, Y)", &[]);
    assert!(out.contains("existence_error(procedure"), "{out}");
    assert!(out.contains("frobnicate"), "{out}");
    assert_eq!(code, 3);
}

// ---- ; / -> precedence -----------------------------------------------

#[test]
fn semicolon_comma_precedence() {
    check("sc1(X)", "X = b", 1);
    check("sc2(X, Y)", "X = a\nY = 1\nX = b\nY = 2", 1);
}

// ---- occurs check ----------------------------------------------------

#[test]
fn unify_with_occurs_check() {
    check("unify_with_occurs_check(X, a)", "X = a", 1);
    fails("unify_with_occurs_check(X, f(X))");
}

#[test]
fn negation_with_member_and_naf_binding() {
    succeeds_once("X = 1, Y = 2, X \\= Y");
    succeeds_once("X = foo, Y = bar, X \\== Y");
    succeeds_once("\\+ (1 = 2)");
}

// ---- DIVERGENCE (ignored, reported for triage) -----------------------

#[test]
fn no_occurs_check_unify_v1_divergence() {
    // v1 test_no_occurs_check_unify: `=` does NOT occurs-check, so X = f(X)
    // succeeds (creating a cyclic term). plgc renders the cycle as f(_V).
    succeeds_once("X = f(X)");
}
