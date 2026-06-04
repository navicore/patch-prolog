//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Control: catch/3 + throw/1 (ISO error handling), ;/-> precedence,
//! and unify_with_occurs_check / no-occurs-check `=`.
//!
//! Fresh-var ids normalized via `norm()` (known adaptation #1).

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
    let (out, code) = prog().query(goal, &[]);
    assert!(
        out.contains("\"count\":1,\"exhausted\":true"),
        "goal {goal}: {out}"
    );
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn fails(goal: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(
        out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n",
        "goal: {goal}"
    );
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
    // v1 test_throw_uncaught_surfaces_as_error_term.
    let (out, code) = prog().query("throw(my_error)", &[]);
    assert_eq!(out, "{\"error\":\"Runtime error: my_error\"}\n");
    assert_eq!(code, 3);
}

#[test]
fn catch_traps_and_binds() {
    // v1 test_catch_traps_matching_thrown_term + binds_catcher_variables +
    //    recovery_can_reference_catcher_var.
    check(
        "catch(throw(e), e, X = recovered)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"recovered\"}]}",
        1,
    );
    check(
        "catch(throw(foo(bar)), foo(X), true)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"bar\"}]}",
        1,
    );
    check(
        "catch(throw(payload(42)), payload(N), Y = N)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"N\":42,\"Y\":42}]}",
        1,
    );
}

#[test]
fn catch_passthrough_non_matching() {
    // v1 test_catch_passes_through_non_matching_thrown_term.
    let (out, code) = prog().query("catch(throw(a), b, true)", &[]);
    assert_eq!(out, "{\"error\":\"Runtime error: a\"}\n");
    assert_eq!(code, 3);
}

#[test]
fn catch_transparent_when_no_throw() {
    // v1 test_catch_with_no_throw_runs_goal_normally.
    check(
        "catch(color(X), _, true)",
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"red\"},{\"X\":\"blue\"}]}",
        1,
    );
}

#[test]
fn catch_traps_builtin_errors() {
    // v1 test_catch_catches_type_error_from_arithmetic + existence_error +
    //    test_type_error_terms_are_inspectable.
    check(
        "catch(X is foo + 1, _, X = trapped)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"trapped\"}]}",
        1,
    );
    check(
        "catch(undefined_predicate(X), error(existence_error(procedure, _), _), Y = trapped)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"_V\",\"Y\":\"trapped\"}]}",
        1,
    );
    check(
        "catch(X is foo, error(type_error(evaluable, _), _), Y = trapped)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"_V\",\"Y\":\"trapped\"}]}",
        1,
    );
}

#[test]
fn catch_nested() {
    // v1 test_catch_nested_inner_caught_first + outer_catches_when_inner_does_not.
    check(
        "catch(catch(throw(e), e, X = inner), e, X = outer)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"inner\"}]}",
        1,
    );
    check(
        "catch(catch(throw(b), a, X = inner), b, X = outer)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"outer\"}]}",
        1,
    );
}

#[test]
fn throw_unbound_is_instantiation_error() {
    // v1 test_throw_unbound_argument_is_instantiation_error.
    check(
        "catch(throw(_), error(instantiation_error, _), X = trapped)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"trapped\"}]}",
        1,
    );
}

#[test]
fn catch_inside_naf() {
    // v1 test_catch_inside_naf_propagates_when_uncaught + handles_thrown.
    err_contains("\\+ throw(my_err)", "my_err");
    check(
        "\\+ catch(throw(e), e, fail)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}",
        1,
    );
}

#[test]
fn existence_and_type_error_shapes() {
    // v1 test_existence_error_indicator_shape.
    let (out, code) = prog().query("frobnicate(X, Y)", &[]);
    assert!(out.contains("existence_error(procedure"), "{out}");
    assert!(out.contains("frobnicate"), "{out}");
    assert_eq!(code, 3);
}

// ---- ; / -> precedence -----------------------------------------------

#[test]
fn semicolon_comma_precedence() {
    // v1 test_semicolon_comma_precedence_in_body + multiple.
    check(
        "sc1(X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"b\"}]}",
        1,
    );
    check(
        "sc2(X, Y)",
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"a\",\"Y\":1},{\"X\":\"b\",\"Y\":2}]}",
        1,
    );
}

// ---- occurs check ----------------------------------------------------

#[test]
fn unify_with_occurs_check() {
    // v1 test_unify_with_occurs_check_success + circular_fails.
    check(
        "unify_with_occurs_check(X, a)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"a\"}]}",
        1,
    );
    fails("unify_with_occurs_check(X, f(X))");
}

#[test]
fn negation_with_member_and_naf_binding() {
    // v1 test_existing_escapes_still_tokenize (the \= and \== goals).
    succeeds_once("X = 1, Y = 2, X \\= Y");
    succeeds_once("X = foo, Y = bar, X \\== Y");
    succeeds_once("\\+ (1 = 2)");
}

// ---- DIVERGENCE (ignored, reported for triage) -----------------------

#[test]
fn no_occurs_check_unify_v1_divergence() {
    // v1 test_no_occurs_check_unify: `=` does NOT occurs-check, so X = f(X)
    // succeeds (creating a cyclic term). plgc actual: empty stdout, exit 139.
    succeeds_once("X = f(X)");
}
