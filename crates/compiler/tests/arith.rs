//! Arithmetic evaluation: `is/2`, function evaluators, the extended
//! operator set (** ^ >> << xor div /\ \/), mod/rem/div semantics,
//! float division, and arithmetic error terms.
//!
//! Every goal here is variable-free in its output (no `_N`), so no
//! normalization is needed. Value assertions use the readable text format;
//! "exactly one solution" checks use the bson envelope (count). All queries
//! run against a one-fact program that advertises both formats.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

fn empty() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(":- io_format([text, bson]).\ndummy_fact.\n"))
}

#[track_caller]
fn ok(goal: &str, expected_x: &str) {
    let (out, code) = empty().query(goal, &[]);
    assert_eq!(out, format!("X = {expected_x}\n"), "goal: {goal}");
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn solves(goal: &str) {
    // Succeeds with a single, binding-free solution.
    let (out, code) = empty().query(goal, &[]);
    assert_eq!(out, "true.\n", "goal: {goal}");
    assert_eq!(code, 1, "goal: {goal}");
}

/// Succeeds with exactly one solution (bindings unchecked) — mirrors the
/// tests that only asserted `solutions.len() == 1`. count lives in bson.
#[track_caller]
fn succeeds_once(goal: &str) {
    let (env, code) = empty().query_bson(goal, &[]);
    assert_eq!(env.count, Some(1), "goal {goal}: {env:?}");
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn err_contains(goal: &str, needle: &str) {
    let (out, code) = empty().query(goal, &[]);
    assert!(out.contains(needle), "goal {goal}: {out}");
    assert_eq!(code, 3, "goal: {goal}");
}

// ---- evaluator functions ---------------------------------------------

#[test]
fn arithmetic_functions() {
    ok("X is abs(-42)", "42");
    ok("X is abs(42)", "42");
    ok("X is max(10, 20)", "20");
    ok("X is min(10, 20)", "10");
    ok("X is sign(42)", "1");
    ok("X is sign(0)", "0");
    ok("X is sign(-7)", "-1");
    ok("X is abs(min(3, -5))", "5");
}

// ---- extended operators (issue #29) ----------------------------------

#[test]
fn extended_operators() {
    ok("X is 2 ^ 10", "1024");
    ok("X is 2 ^ 3 ^ 2", "512"); // xfy right-assoc
    ok("X is 1 << 4", "16");
    ok("X is 32 >> 2", "8");
    ok("X is 6 /\\ 3", "2");
    ok("X is 5 \\/ 2", "7");
    ok("X is 6 xor 3", "5");
}

#[test]
fn pow_is_always_float() {
    ok("X is 2 ** 3", "8.0");
    succeeds_once("X is 2 ** 3, float(X)");
    succeeds_once("X is 2 ** 3, X =:= 8");
    succeeds_once("X is 2 ^ 10, integer(X)");
}

#[test]
fn operator_precedence() {
    succeeds_once("X is 2 * 3 ** 2, X =:= 18");
    ok("X is 1 + 2 << 1", "5");
    ok("X is 6 /\\ 3 + 1", "3");
}

// ---- div / mod / rem -------------------------------------------------

#[test]
fn div_floor_semantics() {
    ok("X is -7 div 2", "-4");
    ok("X is 7 div -2", "-4");
    ok("X is 7 div 2", "3");
}

#[test]
fn integer_division_and_rem() {
    ok("X is 7 // 2", "3");
    ok("X is -7 // 2", "-3");
    ok("X is 7 rem 3", "1");
    ok("X is -7 rem 2", "-1");
}

#[test]
fn mod_floored_semantics() {
    // mod follows the sign of the divisor.
    ok("X is -7 mod 3", "2");
    ok("X is 7 mod -3", "-2");
    ok("X is 5 mod -3", "-1");
}

#[test]
fn mod_vs_rem_difference() {
    // -7 mod 2 = 1, -7 rem 2 = -1.
    let (out, code) = empty().query("X is -7 mod 2, Y is -7 rem 2", &[]);
    assert_eq!(out, "X = 1\nY = -1\n");
    assert_eq!(code, 1);
}

// ---- float division (ISO 9.1.4) --------------------------------------

#[test]
fn iso_div_yields_float() {
    let (out, _) = empty().query("X is 10 / 3", &[]);
    assert!(out.contains("3.333"), "{out}");
    let (out, _) = empty().query("X is -10 / 3", &[]);
    assert!(out.contains("-3.333"), "{out}");
    ok("X is 6 / 2", "3.0");
    succeeds_once("X is 6 / 2, float(X)");
    // 6 / 2 must NOT be an integer.
    let (out, code) = empty().query("X is 6 / 2, integer(X)", &[]);
    assert_eq!(out, "false.\n");
    assert_eq!(code, 0);
    ok("X is 10 // 3", "3");
}

#[test]
fn double_minus_and_infix() {
    ok("X is - - 3", "3");
    ok("X is -3 + 5", "2");
    ok("X is 1 + 2", "3");
}

// ---- arithmetic errors -----------------------------------------------

#[test]
fn arithmetic_error_terms() {
    err_contains("X is 10 / 0", "zero");
    err_contains("X is 1.0 / 0", "Division by zero");
    err_contains("X is 1 / 0", "zero");
    err_contains("X is 5 div 0", "zero");
    err_contains("X is Y + 1", "instantiation");
    err_contains(&format!("X is {} + 1", i64::MAX), "overflow");
    err_contains("X is 1 << -1", "Shift");
    err_contains("X is foo + 1", "type_error(evaluable");
}

#[test]
fn succ_plus_overflow() {
    err_contains(&format!("succ({}, X)", i64::MAX), "overflow");
    err_contains(&format!("plus({}, 1, X)", i64::MAX), "overflow");
}

// ---- naf precedence around arithmetic --------------------------------

#[test]
fn naf_precedence() {
    // `\+ 1 =:= 2` parses as `\+(1 =:= 2)`.
    solves("\\+ 1 =:= 2");
    // `\+ X = goodbye` parses as `\+(X = goodbye)`.
    let (out, code) = empty().query("X = hello, \\+ X = goodbye", &[]);
    assert_eq!(out, "X = hello\n");
    assert_eq!(code, 1);
}
