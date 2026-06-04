//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Arithmetic evaluation: `is/2`, function evaluators, the extended
//! operator set (** ^ >> << xor div /\ \/), mod/rem/div semantics,
//! float division, and arithmetic error terms.
//!
//! Every goal here is variable-free in its output (no `_N`), so no
//! normalization is needed. All queries run against a one-fact program.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

fn empty() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    // Needs at least one clause to be a valid program.
    C.get_or_init(|| compile("dummy_fact.\n"))
}

#[track_caller]
fn ok(goal: &str, expected_x: &str) {
    let (out, code) = empty().query(goal, &[]);
    assert_eq!(
        out,
        format!("{{\"count\":1,\"exhausted\":true,\"solutions\":[{{\"X\":{expected_x}}}]}}\n"),
        "goal: {goal}"
    );
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn solves(goal: &str) {
    // Succeeds with a single, binding-free solution.
    let (out, code) = empty().query(goal, &[]);
    assert_eq!(
        out, "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}\n",
        "goal: {goal}"
    );
    assert_eq!(code, 1, "goal: {goal}");
}

/// Succeeds with exactly one solution (bindings unchecked) — mirrors v1
/// tests that only asserted `solutions.len() == 1`.
#[track_caller]
fn succeeds_once(goal: &str) {
    let (out, code) = empty().query(goal, &[]);
    assert!(
        out.contains("\"count\":1,\"exhausted\":true"),
        "goal {goal}: {out}"
    );
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
    // v1 test_arithmetic_abs/max_min/sign/combined.
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
    // v1 test_op_caret_int_power / shift_left / shift_right / bit_and /
    //    bit_or / bit_xor.
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
    // v1 test_op_pow_float_yields_float / value.
    ok("X is 2 ** 3", "8.0");
    succeeds_once("X is 2 ** 3, float(X)");
    succeeds_once("X is 2 ** 3, X =:= 8");
    succeeds_once("X is 2 ^ 10, integer(X)");
}

#[test]
fn operator_precedence() {
    // v1 test_op_precedence_pow_tighter_than_mul / shift_tighter_than_plus /
    //    bit_and_left_to_right_with_plus.
    succeeds_once("X is 2 * 3 ** 2, X =:= 18");
    ok("X is 1 + 2 << 1", "5");
    ok("X is 6 /\\ 3 + 1", "3");
}

// ---- div / mod / rem -------------------------------------------------

#[test]
fn div_floor_semantics() {
    // v1 test_op_div_floor / div_floor_negative_divisor.
    ok("X is -7 div 2", "-4");
    ok("X is 7 div -2", "-4");
    ok("X is 7 div 2", "3");
}

#[test]
fn integer_division_and_rem() {
    // v1 test_integer_division_operator / negative + rem_operator +
    //    rem_negative_dividend.
    ok("X is 7 // 2", "3");
    ok("X is -7 // 2", "-3");
    ok("X is 7 rem 3", "1");
    ok("X is -7 rem 2", "-1");
}

#[test]
fn mod_floored_semantics() {
    // v1 test_mod_floored_semantics / mod_large_negative_divisor.
    // mod follows the sign of the divisor.
    ok("X is -7 mod 3", "2");
    ok("X is 7 mod -3", "-2");
    ok("X is 5 mod -3", "-1");
}

#[test]
fn mod_vs_rem_difference() {
    // v1 test_mod_vs_rem_difference: -7 mod 2 = 1, -7 rem 2 = -1.
    let (out, code) = empty().query("X is -7 mod 2, Y is -7 rem 2", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":1,\"Y\":-1}]}\n"
    );
    assert_eq!(code, 1);
}

// ---- float division (ISO 9.1.4) --------------------------------------

#[test]
fn iso_div_yields_float() {
    // v1 test_iso_div_int_int_is_float / negative / exact_quotient / still_truncates.
    let (out, _) = empty().query("X is 10 / 3", &[]);
    assert!(out.contains("3.333"), "{out}");
    let (out, _) = empty().query("X is -10 / 3", &[]);
    assert!(out.contains("-3.333"), "{out}");
    ok("X is 6 / 2", "3.0");
    succeeds_once("X is 6 / 2, float(X)");
    // 6 / 2 must NOT be an integer.
    let (out, code) = empty().query("X is 6 / 2, integer(X)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
    ok("X is 10 // 3", "3");
}

#[test]
fn double_minus_and_infix() {
    // v1 test_prefix_double_minus_folds + does_not_break_infix + prefix_minus.
    ok("X is - - 3", "3");
    ok("X is -3 + 5", "2");
    ok("X is 1 + 2", "3");
}

// ---- arithmetic errors -----------------------------------------------

#[test]
fn arithmetic_error_terms() {
    // v1 test_division_by_zero / unbound_variable_in_arithmetic /
    //    integer_overflow_detected / float_div_by_int_zero / div_int_by_int_zero /
    //    op_div_zero_errors / op_shift_error_on_negative_count.
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
    // v1 test_succ_overflow + test_plus_overflow.
    err_contains(&format!("succ({}, X)", i64::MAX), "overflow");
    err_contains(&format!("plus({}, 1, X)", i64::MAX), "overflow");
}

// ---- naf precedence around arithmetic --------------------------------

#[test]
fn naf_precedence() {
    // v1 test_naf_precedence_with_is: `\+ 1 =:= 2` parses as `\+(1 =:= 2)`.
    solves("\\+ 1 =:= 2");
    // v1 test_naf_parses_with_operator: `\+ X = goodbye` parses as `\+(X = goodbye)`.
    let (out, code) = empty().query("X = hello, \\+ X = goodbye", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"hello\"}]}\n"
    );
    assert_eq!(code, 1);
}
