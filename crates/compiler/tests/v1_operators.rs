//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Operator / parser-surface tests (v1 issues #19, #28, #29).
//!
//! plgc's ISO-subset frontend implements the INFIX operator set (covered
//! in v1_arith: `**`, `^`, `<<`, `>>`, `xor`, `/\`, `\/`, `div`) and
//! accepts SOME parenthesized comparison operators as atoms, but does NOT
//! implement v1's operator-as-atom-in-term-position and prefix-`+`/`\`/`:`
//! surface features. The unsupported cases are recorded as `#[ignore]`d
//! divergence tests (see top-of-file SKIP LIST in v1_errors.rs, group B,
//! and the final report).

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

fn empty() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile("just_a_fact.\n"))
}

// ---- supported: comparison operator as a parenthesized atom ----------

#[test]
fn comparison_operator_as_atom() {
    // v1 test_op_atom_in_parens_after_equals (the `<` case) — plgc accepts
    // `(<)`, `(=)`, `(>)` as bare atoms in term position.
    for (q, want) in [("X = (<)", "<"), ("X = (=)", "="), ("X = (>)", ">")] {
        let (out, code) = empty().query(q, &[]);
        assert_eq!(
            out,
            format!("{{\"count\":1,\"exhausted\":true,\"solutions\":[{{\"X\":\"{want}\"}}]}}\n"),
            "query: {q}"
        );
        assert_eq!(code, 1, "query: {q}");
    }
}

#[test]
fn infix_arithmetic_unaffected() {
    // v1 test_op_atom_does_not_break_infix_arithmetic / prefix_minus_still_works.
    let (out, _) = empty().query("X is 1 + 2", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":3}]}\n"
    );
    let (out, _) = empty().query("X is -3 + 5", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":2}]}\n"
    );
}

// ---- DIVERGENCES (ignored, reported for triage) ----------------------
// Each ignored test documents the v1-expected behavior vs plgc's actual.

#[test]
fn operator_atom_and_prefix_surface_v1_divergence() {
    // v1 test_prefix_plus_on_atom_builds_compound expected:
    //   functor(+ a, N, A) => N = '+', A = 1.
    // plgc actual: {"error":"Parse error: expected `,` or `)` at column 11"} (exit 2).
    let (out, code) = empty().query("functor(+ a, N, A)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"A\":1,\"N\":\"+\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn colon_operator_v1_divergence() {
    // v1 test_op_colon_parses_as_term expected:
    //   functor(pkg : expr, N, A) => N = ':', A = 2.
    // plgc actual: {"error":"Parse error: expected `,` or `)` at column 13"} (exit 2).
    let (out, code) = empty().query("functor(pkg : expr, N, A)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"A\":2,\"N\":\":\"}]}\n"
    );
    assert_eq!(code, 1);
}
