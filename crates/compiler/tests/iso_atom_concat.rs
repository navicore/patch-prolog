//! ISO 8.16.2 `atom_concat/3` — forward assembly AND the relational split
//! modes (issue #35).
//!
//! `atom_concat/3` is multi-mode, like `append/3` for lists:
//! - forward: A and B atoms → concatenate, unify with C;
//! - known prefix/suffix: one of A/B bound + C bound → the matching split;
//! - both unbound + C bound → enumerate every decomposition on backtracking;
//! - insufficiently instantiated (A or B var, C var) → `instantiation_error`
//!   (NOT `type_error`, which is reserved for a bound term of the wrong type).
//!
//! These pin ISO behaviour directly; the old forward-only `type_error`-on-split
//! contract was a bug.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile("p.\n"))
}

#[track_caller]
fn check(goal: &str, expected: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(out.trim_end(), expected, "goal: {goal}");
    assert_eq!(code, 1, "goal {goal} should succeed: {out}");
}

#[track_caller]
fn fails(goal: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(
        out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n",
        "goal {goal} should have no solutions"
    );
    assert_eq!(code, 0, "goal: {goal}");
}

#[track_caller]
fn errors(goal: &str, needle: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert!(
        out.contains(needle),
        "goal {goal}: expected {needle}, got {out}"
    );
    assert_eq!(code, 3, "goal {goal} should error: {out}");
}

#[test]
fn forward_assembly() {
    check(
        "atom_concat(foo, bar, X)",
        r#"{"count":1,"exhausted":true,"solutions":[{"X":"foobar"}]}"#,
    );
    // Both bound: a verification, binding nothing.
    check(
        "atom_concat(foo, bar, foobar)",
        r#"{"count":1,"exhausted":true,"solutions":[{}]}"#,
    );
    fails("atom_concat(foo, bar, nope)");
}

#[test]
fn known_prefix_or_suffix_selects_the_split() {
    check(
        "atom_concat(X, bar, foobar)",
        r#"{"count":1,"exhausted":true,"solutions":[{"X":"foo"}]}"#,
    );
    check(
        "atom_concat(foo, Y, foobar)",
        r#"{"count":1,"exhausted":true,"solutions":[{"Y":"bar"}]}"#,
    );
    // A prefix/suffix that doesn't line up has no solution (it does not error).
    fails("atom_concat(X, xyz, foobar)");
    fails("atom_concat(zzz, Y, foobar)");
}

#[test]
fn both_unbound_enumerates_every_decomposition() {
    check(
        "atom_concat(A, B, abc)",
        r#"{"count":4,"exhausted":true,"solutions":[{"A":"","B":"abc"},{"A":"a","B":"bc"},{"A":"ab","B":"c"},{"A":"abc","B":""}]}"#,
    );
    // The empty atom decomposes exactly one way: into two empty atoms.
    check(
        "atom_concat(A, B, '')",
        r#"{"count":1,"exhausted":true,"solutions":[{"A":"","B":""}]}"#,
    );
}

#[test]
fn shared_variable_keeps_only_equal_halves() {
    // atom_concat(X, X, C) succeeds only at the split whose halves match.
    check(
        "atom_concat(X, X, abab)",
        r#"{"count":1,"exhausted":true,"solutions":[{"X":"ab"}]}"#,
    );
    fails("atom_concat(X, X, abc)");
}

#[test]
fn unicode_splits_on_char_boundaries() {
    // Multi-byte atoms must split between characters, never inside a code point.
    check(
        "atom_concat(A, B, héllo)",
        r#"{"count":6,"exhausted":true,"solutions":[{"A":"","B":"héllo"},{"A":"h","B":"éllo"},{"A":"hé","B":"llo"},{"A":"hél","B":"lo"},{"A":"héll","B":"o"},{"A":"héllo","B":""}]}"#,
    );
}

#[test]
fn unbound_with_unbound_c_is_instantiation_error() {
    // ISO: instantiation_error, not type_error, when a needed argument is a var.
    errors("atom_concat(X, Y, Z)", "instantiation_error");
    errors("atom_concat(foo, Y, Z)", "instantiation_error");
    errors("atom_concat(X, bar, Z)", "instantiation_error");
}

#[test]
fn bound_non_atom_is_type_error() {
    errors("atom_concat(123, foo, _)", "type_error(atom, 123)");
    errors("atom_concat(foo, 456, _)", "type_error(atom, 456)");
}

#[test]
fn split_mode_works_inside_a_compiled_clause_body() {
    // Exercises the compiled predicate-dispatch path (clause.rs), distinct from
    // the top-level query/metacall path: a nondeterministic atom_concat mid-body
    // must enumerate and feed each solution to the continuation.
    let c = compile("prefix(P, W) :- atom_concat(P, _, W).\n");
    let (out, code) = c.query("prefix(P, abc)", &[]);
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        out.trim_end(),
        r#"{"count":4,"exhausted":true,"solutions":[{"P":""},{"P":"a"},{"P":"ab"},{"P":"abc"}]}"#,
    );
}

#[test]
fn split_mode_reachable_via_metacall() {
    // call/1 routes through the runtime's query-side dispatch (try_builtin),
    // which must also enumerate splits.
    check(
        "call(atom_concat(A, B, ab))",
        r#"{"count":3,"exhausted":true,"solutions":[{"A":"","B":"ab"},{"A":"a","B":"b"},{"A":"ab","B":""}]}"#,
    );
}
