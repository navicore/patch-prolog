//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Operator / parser-surface tests (v1 issues #19, #28, #29).
//!
//! plgc's ISO-subset frontend implements the INFIX operator set (covered
//! in v1_arith: `**`, `^`, `<<`, `>>`, `xor`, `/\`, `\/`, `div`) and
//! accepts SOME parenthesized comparison operators as atoms. Output is the
//! readable text format.

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
    // `(<)`, `(=)`, `(>)` as bare atoms in term position.
    for (q, want) in [("X = (<)", "<"), ("X = (=)", "="), ("X = (>)", ">")] {
        let (out, code) = empty().query(q, &[]);
        assert_eq!(out, format!("X = {want}\n"), "query: {q}");
        assert_eq!(code, 1, "query: {q}");
    }
}

#[test]
fn infix_arithmetic_unaffected() {
    let (out, _) = empty().query("X is 1 + 2", &[]);
    assert_eq!(out, "X = 3\n");
    let (out, _) = empty().query("X is -3 + 5", &[]);
    assert_eq!(out, "X = 2\n");
}

// ---- prefix-op / colon surface (plgc accepts these as compounds) -----

#[test]
fn operator_atom_and_prefix_surface() {
    // functor(+ a, N, A) => N = '+', A = 1.
    let (out, code) = empty().query("functor(+ a, N, A)", &[]);
    assert_eq!(out, "A = 1\nN = +\n");
    assert_eq!(code, 1);
}

#[test]
fn colon_operator() {
    // functor(pkg : expr, N, A) => N = ':', A = 2.
    let (out, code) = empty().query("functor(pkg : expr, N, A)", &[]);
    assert_eq!(out, "A = 2\nN = :\n");
    assert_eq!(code, 1);
}
