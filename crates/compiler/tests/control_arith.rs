//! M3 integration tests: cut, disjunction, if-then-else, negation,
//! once, unification builtins, comparisons, and arithmetic. Output is the
//! readable text format (semantics originally captured from the v1 oracle
//! on 2026-06-04; the rendered format differs — no JSON, docs/design/IO.md).

mod harness;
use harness::Compiled;
use std::sync::OnceLock;

const M3_PROGRAM: &str = "\
max(X, Y, X) :- X >= Y, !.
max(_, Y, Y).
classify(X, neg) :- X < 0.
classify(0, zero).
classify(X, pos) :- X > 0.
sumlist([], 0).
sumlist([H|T], S) :- sumlist(T, S1), S is S1 + H.
related(X, Y) :- (parent(X, Y) ; parent(Y, X)).
parent(a, b).
parent(b, c).
notparent(X, Y) :- \\+ parent(X, Y).
status(X, S) :- (parent(X, _) -> S = has_kids ; S = childless).
firstkid(X, K) :- once(parent(X, K)).
samesame(X, Y) :- X == Y.
diff(X, Y) :- X \\= Y.
";

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| harness::compile(M3_PROGRAM))
}

#[track_caller]
fn check(goal: &str, expected_out: &str, expected_code: i32) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(out, format!("{expected_out}\n"), "goal: {goal}");
    assert_eq!(code, expected_code, "goal: {goal}");
}

#[test]
fn cut_commits_to_first_clause() {
    check("max(3, 7, M)", "M = 7", 1);
    check("max(9, 2, M)", "M = 9", 1);
}

#[test]
fn arith_comparisons_and_indexing_coexist() {
    check("classify(-5, C)", "C = neg", 1);
    check("classify(0, C)", "C = zero", 1);
    check("classify(9, C)", "C = pos", 1);
}

#[test]
fn is_evaluates_through_recursion() {
    check("sumlist([1,2,3,4], S)", "S = 10", 1);
}

#[test]
fn disjunction_enumerates_both_branches() {
    check("related(b, X)", "X = c\nX = a", 1);
}

#[test]
fn negation_as_failure() {
    check("notparent(c, a)", "true.", 1);
    check("notparent(a, b)", "false.", 0);
}

#[test]
fn if_then_else_both_arms() {
    check("status(a, S)", "S = has_kids", 1);
    check("status(c, S)", "S = childless", 1);
}

#[test]
fn once_commits_to_first_solution() {
    check("firstkid(a, K)", "K = b", 1);
}

#[test]
fn structural_equality_and_not_unify() {
    check("samesame(foo, foo)", "true.", 1);
    check("samesame(foo, bar)", "false.", 0);
    check("diff(foo, bar)", "true.", 1);
    check("diff(foo, foo)", "false.", 0);
}

#[test]
fn top_level_arithmetic_queries() {
    check("X is 2 + 3 * 4", "X = 14", 1);
    check("X is 7 // 2", "X = 3", 1);
    // Floored mod: sign follows the divisor (ISO_COMPLIANCE.md).
    check("X is -7 mod 3", "X = 2", 1);
    check("1 < 2", "true.", 1);
    check("compare(O, foo, bar)", "O = >", 1);
}

#[test]
fn arithmetic_errors_match_v1() {
    check(
        "X is 1 // 0",
        "error: Runtime error: error(evaluation_error(zero_divisor), Division by zero (integer division))",
        3,
    );
    // DELIBERATE ISO-over-v1 divergence (issue #36): the culprit for a
    // non-evaluable atom is the predicate indicator `foo/0` per ISO 8.6,
    // not the bare atom v1 produced. The compound path was already correct.
    check(
        "X is foo + 1",
        "error: Runtime error: error(type_error(evaluable, /(foo, 0)), Cannot evaluate as arithmetic)",
        3,
    );
}

#[test]
fn cut_is_transparent_in_disjunction_iso_rule() {
    // DELIBERATE v1 DIVERGENCE (documented in docs/ISO_COMPLIANCE.md):
    // ISO 7.8.4 — `!` inside `;` cuts the whole clause, including the
    // disjunction's else branch. v1 treated the cut as local to the
    // branch and (non-ISO) leaked `fallback` as a second solution.
    let c = harness::compile("t(X) :- (m(X), X > 1, ! ; X = fallback).\nm(1).\nm(2).\nm(3).\n");
    let (out, code) = c.query("t(X)", &[]);
    assert_eq!(out, "X = 2\n");
    assert_eq!(code, 1);
}

#[test]
fn cut_is_local_in_call_like_contexts() {
    // ISO: `!` inside an if-then-else CONDITION, `\+`, or `once` is
    // opaque — it prunes only within that goal.
    let c = harness::compile(
        "m(1).\nm(2).\nm(3).\n\
         condcut(X, S) :- (m(X), ! -> S = hit ; S = miss).\n\
         nafcut(X) :- \\+ (m(X), !, fail).\n\
         oncecut(X) :- once((m(X), !)).\n",
    );
    let (out, _) = c.query("condcut(X, S)", &[]);
    assert_eq!(out, "S = hit\nX = 1\n");
    let (out, _) = c.query("nafcut(2)", &[]);
    assert_eq!(out, "true.\n");
    let (out, _) = c.query("oncecut(X)", &[]);
    assert_eq!(out, "X = 1\n");
}

#[test]
fn float_literals_in_queries() {
    let c = harness::compile("near(X, Y) :- Z is X - Y, Z < 1, Z > -1.\n");
    let (out, _) = c.query("X is 1.5 + 2.5", &[]);
    assert_eq!(out, "X = 4.0\n");
    // Standard order: Float < Integer at numeric equality.
    let (out, _) = c.query("compare(O, 1, 1.0)", &[]);
    assert_eq!(out, "O = >\n");
}

#[test]
fn deep_backtracking_with_cut_under_small_stack() {
    // Cut inside a recursive predicate must not break constant-stack
    // execution.
    let mut src = String::new();
    for i in 0..1500 {
        src.push_str(&format!("edge(e{i}, e{}).\n", i + 1));
    }
    src.push_str("path(X, X).\npath(X, Z) :- edge(X, Y), !, path(Y, Z).\n");
    let c = harness::compile(&src);
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s 512; PLG_MAX_STEPS=100000000 {} --query 'path(e0, e1500)' --format text",
            c.bin.display()
        ))
        .output()
        .expect("run with ulimit");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "true.\n",
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
