//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Facts, rules, recursion, type-checking, if-then-else, disjunction,
//! cut, negation-as-failure, once/call, and the solution limit.
//!
//! v1 asserted in-process solution *counts* and first-binding values;
//! here we assert the readable text wire output of the compiled plgc
//! binary (value/order), with `count`/`exhausted` checks via the bson
//! envelope. The semantics are v1-validated; only the rendered format
//! differs (no JSON — docs/design/IO.md).
//!
//! Variable placeholders (`_N`) are normalized via `norm()` because plgc
//! numbers fresh variables differently from v1 (known adaptation #1).

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

/// Normalize fresh-variable ids (`_0`, `_13`, …) to a stable `_V` so
/// assertions are insensitive to plgc-vs-v1 variable numbering.
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

#[track_caller]
fn check(c: &Compiled, goal: &str, expected_out: &str, expected_code: i32) {
    let (out, code) = c.query(goal, &[]);
    assert_eq!(
        norm(&out),
        norm(&format!("{expected_out}\n")),
        "goal: {goal}"
    );
    assert_eq!(code, expected_code, "goal: {goal}");
}

// ---- family / recursion fixtures -------------------------------------

const FAMILY: &str = "\
parent(tom, mary).
parent(tom, james).
parent(mary, ann).
parent(mary, bob).
grandparent(X, Z) :- parent(X, Y), parent(Y, Z).
sibling(X, Y) :- parent(P, X), parent(P, Y), X \\= Y.
";

fn family() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(FAMILY))
}

#[test]
fn family_relationships() {
    check(family(), "grandparent(tom, X)", "X = ann\nX = bob", 1);
    check(family(), "sibling(ann, X)", "X = bob", 1);
}

const ANCESTOR: &str = "\
parent(tom, mary).
parent(mary, ann).
parent(ann, alice).
ancestor(X, Y) :- parent(X, Y).
ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).
";

fn ancestor() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(ANCESTOR))
}

#[test]
fn complex_recursive_query() {
    check(
        ancestor(),
        "ancestor(tom, X)",
        "X = mary\nX = ann\nX = alice",
        1,
    );
}

// ---- arithmetic-in-rule recursion ------------------------------------

const FACTORIAL: &str = "\
factorial(0, 1).
factorial(N, F) :- N > 0, N1 is N - 1, factorial(N1, F1), F is N * F1.
";

#[test]
fn arithmetic_pipeline() {
    let c = compile(FACTORIAL);
    check(&c, "factorial(5, X)", "X = 120", 1);
}

// ---- type-checking predicates ----------------------------------------

const TYPECLASS: &str = "\
classify(X, integer) :- integer(X).
classify(X, float) :- float(X).
classify(X, atom) :- atom(X).
";

#[test]
fn type_checking_in_rules() {
    let c = compile(TYPECLASS);
    check(&c, "classify(42, T)", "T = integer", 1);
    check(&c, "classify(3.14, T)", "T = float", 1);
    check(&c, "classify(hello, T)", "T = atom", 1);
}

// ---- if-then-else / disjunction --------------------------------------

const CONTROL: &str = "\
absval(X, Y) :- (X < 0 -> Y is 0 - X ; Y = X).
primary_color(X) :- (X = red ; X = green ; X = blue).
classify2(X, R) :- (X > 0 -> R = positive ; R = non_positive).
test_match(X, R) :- (X = hello -> R = matched ; R = no_match).
";

fn control() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(CONTROL))
}

#[test]
fn if_then_else_in_rule() {
    check(control(), "absval(-5, Y)", "Y = 5", 1);
    check(control(), "absval(3, Y)", "Y = 3", 1);
}

#[test]
fn disjunction_in_rule() {
    check(
        control(),
        "primary_color(X)",
        "X = red\nX = green\nX = blue",
        1,
    );
}

#[test]
fn if_then_else_keeps_bindings() {
    check(control(), "classify2(5, R)", "R = positive", 1);
    check(control(), "classify2(-1, R)", "R = non_positive", 1);
    check(control(), "test_match(hello, R)", "R = matched", 1);
    check(control(), "test_match(world, R)", "R = no_match", 1);
}

// ---- cut / negation / once / call ------------------------------------

const CUT: &str = "\
classify(X, positive) :- X > 0, !.
classify(0, zero) :- !.
classify(_, negative).
foo(a). foo(b). foo(c).
q(a). q(b). q(c).
bird(tweety).
bird(penguin).
can_fly(X) :- bird(X), \\+ penguin_species(X).
penguin_species(penguin).
";

fn cut() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(CUT))
}

#[test]
fn cut_prevents_backtracking() {
    check(cut(), "classify(5, C)", "C = positive", 1);
    check(cut(), "classify(0, C)", "C = zero", 1);
    check(cut(), "classify(-3, C)", "C = negative", 1);
}

#[test]
fn cut_prevents_all_alternatives() {
    check(cut(), "foo(X), !", "X = a", 1);
    check(cut(), "foo(X), !, X = b", "false.", 0);
}

#[test]
fn cut_in_once() {
    // v1 test_cut_in_once: once with cut yields no solution.
    check(cut(), "once((foo(X), !, X = b))", "false.", 0);
}

#[test]
fn cut_in_try_solve_no_leak_after_once() {
    // v1 test_cut_in_try_solve_no_leak_after_once: once(!) must not leak
    // the cut into the following predicate iteration.
    check(cut(), "once(!), q(X)", "X = a\nX = b\nX = c", 1);
}

#[test]
fn negation_as_failure_pipeline() {
    // v1 test_negation_as_failure_pipeline: only tweety can fly.
    check(cut(), "can_fly(X)", "X = tweety", 1);
}

// ---- once / call meta-call -------------------------------------------

const META: &str = "\
color(red). color(green). color(blue).
n(1). n(2). n(3).
first_n(X) :- once(n(X)).
applyg(Goal) :- call(Goal).
apply1(F, X) :- call(F, X).
foo(a, 1, 2). foo(a, 3, 4).
ok. bar :- ok.
weight(apple, 150). weight(banana, 120). weight(cherry, 8).
";

fn meta() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(META))
}

#[test]
fn once_limits_to_first_solution() {
    check(meta(), "once(color(X))", "X = red", 1);
    check(meta(), "first_n(X)", "X = 1", 1);
}

#[test]
fn call_meta_predicate() {
    check(
        meta(),
        "applyg(color(X))",
        "X = red\nX = green\nX = blue",
        1,
    );
    check(meta(), "call(color, X)", "X = red\nX = green\nX = blue", 1);
    check(
        meta(),
        "call(foo(a), X, Y)",
        "X = 1\nY = 2\nX = 3\nY = 4",
        1,
    );
    check(
        meta(),
        "apply1(color, X)",
        "X = red\nX = green\nX = blue",
        1,
    );
    check(meta(), "call(bar)", "true.", 1);
}

#[test]
fn call_n_with_stdlib_member_and_nesting() {
    check(
        meta(),
        "call(member, X, [1, 2, 3])",
        "X = 1\nX = 2\nX = 3",
        1,
    );
    check(
        meta(),
        "call(call(member, X), [a, b, c])",
        "X = a\nX = b\nX = c",
        1,
    );
}

#[test]
fn call_n_operator_atom_and_findall_inner_goal() {
    check(meta(), "call('=', X, foo)", "X = foo", 1);
    check(
        meta(),
        "findall(W, call(weight, _, W), Ws), Ws = [150, 120, 8]",
        "W = _V\nWs = [150, 120, 8]",
        1,
    );
}

#[test]
fn call_n_errors() {
    // v1 test_call_n_unbound_goal_throws_instantiation_error +
    //    test_call_n_non_callable_throws_type_error.
    let (out, code) = meta().query("call(G, X)", &[]);
    assert!(out.contains("instantiation_error"), "{out}");
    assert_eq!(code, 3);
    let (out, code) = meta().query("call(5, X)", &[]);
    assert!(out.contains("type_error(callable"), "{out}");
    assert!(out.contains('5'), "{out}");
    assert_eq!(code, 3);
}

// ---- solution limit --------------------------------------------------

#[test]
fn solution_limit_respected() {
    // v1 test_solution_limit_respected (n(1)..n(10)) using --limit. exhausted
    // semantics live in the bson envelope, so the program advertises both.
    let c = compile(
        ":- io_format([text, bson]).\nn(1). n(2). n(3). n(4). n(5). n(6). n(7). n(8). n(9). n(10).\n",
    );
    let (env, _) = c.query_bson("n(X)", &["--limit", "3"]);
    assert_eq!(env.count, Some(3));
    assert_eq!(env.exhausted, Some(false));
    let (env, _) = c.query_bson("n(X)", &["--limit", "100"]);
    assert_eq!(env.count, Some(10));
    assert_eq!(env.exhausted, Some(true));
}

// Cut is opaque inside \+ (ISO and v1 agree): `foo(X),!,X=b` commits
// to X=a, fails, so \+ succeeds — with X reported unbound.
// Fixed in M4 by the walker cut-barrier mechanics (qbarrier).
#[test]
fn cut_in_negation_succeeds_with_unbound_var() {
    check(cut(), "\\+((foo(X), !, X = b))", "X = _V", 1);
}
