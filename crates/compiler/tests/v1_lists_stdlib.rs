//! Ported from patch-prolog v1 `crates/cli/tests/integration.rs`.
//! Stdlib list predicates (member/append/length/reverse/last),
//! is_list, copy_term, between, succ/plus, and findall over lists.
//!
//! These rely on plgc's built-in stdlib; the fixture program does NOT
//! redefine member/append (doing so would duplicate the stdlib clauses
//! and inflate solution counts).

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

// A fixture that adds no clauses of its own — exercises pure stdlib.
const PROG: &str = "\
score(alice, 85).
score(bob, 92).
score(carol, 78).
sorted_names(Sorted) :- findall(Name, score(Name, _), L), sort(L, Sorted).
my_list([1,2,3]).
";

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(PROG))
}

// ---- member / append / length / reverse / last -----------------------

#[test]
fn list_operations() {
    // v1 test_list_operations.
    check(
        prog(),
        "member(X, [a, b, c])",
        "{\"count\":3,\"exhausted\":true,\"solutions\":[{\"X\":\"a\"},{\"X\":\"b\"},{\"X\":\"c\"}]}",
        1,
    );
    check(
        prog(),
        "append([1, 2], [3, 4], X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[1,2,3,4]}]}",
        1,
    );
    check(
        prog(),
        "length([a, b, c, d], N)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"N\":4}]}",
        1,
    );
}

#[test]
fn reverse_and_last() {
    check(
        prog(),
        "reverse([1, 2, 3], X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[3,2,1]}]}",
        1,
    );
    check(
        prog(),
        "last([1, 2, 3], X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":3}]}",
        1,
    );
}

#[test]
fn is_list_predicate() {
    // v1 test_is_list_with_bound_tail + test_is_list_with_constructed_list.
    check(
        prog(),
        "X = [1,2,3], is_list(X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[1,2,3]}]}",
        1,
    );
    check(
        prog(),
        "my_list(X), is_list(X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[1,2,3]}]}",
        1,
    );
}

// ---- copy_term -------------------------------------------------------

#[test]
fn copy_term_ground_and_lists() {
    // v1 test_copy_term_ground + nested_list + preserves_long_list.
    check(
        prog(),
        "copy_term(hello, Copy)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"Copy\":\"hello\"}]}",
        1,
    );
    check(
        prog(),
        "copy_term(42, Copy)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"Copy\":42}]}",
        1,
    );
    check(
        prog(),
        "copy_term([a, b, c], X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[\"a\",\"b\",\"c\"]}]}",
        1,
    );
    check(
        prog(),
        "copy_term([1,2,3,4,5,6,7,8,9,10], X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[1,2,3,4,5,6,7,8,9,10]}]}",
        1,
    );
}

#[test]
fn copy_term_fresh_vars_and_aliasing() {
    // v1 test_copy_term_basic + test_copy_term_aliasing + test_copy_term_list.
    // Fresh-var ids normalized via norm().
    check(
        prog(),
        "copy_term(f(X, Y), Copy)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"Copy\":{\"args\":[\"_V\",\"_V\"],\"functor\":\"f\"},\"X\":\"_V\",\"Y\":\"_V\"}]}",
        1,
    );
    // Aliasing: the two copies of X must unify together.
    check(
        prog(),
        "copy_term(f(X, X), f(A, B)), A = hello",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"A\":\"hello\",\"B\":\"hello\",\"X\":\"_V\"}]}",
        1,
    );
    // copy_term/2 copies arg1 to a FRESH term, then unifies the copy with
    // [1,2,3]; the originals A,B,C stay unbound (ISO semantics). v1 only
    // asserted success here, not the bindings.
    check(
        prog(),
        "copy_term([A, B, C], [1, 2, 3])",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"A\":\"_V\",\"B\":\"_V\",\"C\":\"_V\"}]}",
        1,
    );
}

// ---- between ---------------------------------------------------------

const BETWEEN: &str = "digit(D) :- between(0, 9, D).\n";

fn between() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(BETWEEN))
}

#[test]
fn between_basic() {
    // v1 test_between_basic/single/empty/low_greater_than_high.
    check(
        between(),
        "between(1, 5, X)",
        "{\"count\":5,\"exhausted\":true,\"solutions\":[{\"X\":1},{\"X\":2},{\"X\":3},{\"X\":4},{\"X\":5}]}",
        1,
    );
    check(
        between(),
        "between(3, 3, X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":3}]}",
        1,
    );
    check(
        between(),
        "between(5, 3, X)",
        "{\"count\":0,\"exhausted\":true,\"solutions\":[]}",
        0,
    );
}

#[test]
fn between_in_rule_and_findall() {
    // v1 test_between_in_rule + test_between_with_findall.
    check(
        between(),
        "digit(D)",
        "{\"count\":10,\"exhausted\":true,\"solutions\":[{\"D\":0},{\"D\":1},{\"D\":2},{\"D\":3},{\"D\":4},{\"D\":5},{\"D\":6},{\"D\":7},{\"D\":8},{\"D\":9}]}",
        1,
    );
    check(
        between(),
        "findall(X, between(1, 5, X), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[1,2,3,4,5],\"X\":\"_V\"}]}",
        1,
    );
}

#[test]
fn between_bound_x_fast_paths() {
    // v1 test_between_bound_x_fast_path / out_of_range / below_range /
    //    bound_x_large_range_findall / bound_x_large_range_naf / unbound_x_naf.
    check(
        between(),
        "between(1, 1000000, 5)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}",
        1,
    );
    check(
        between(),
        "between(1, 10, 11)",
        "{\"count\":0,\"exhausted\":true,\"solutions\":[]}",
        0,
    );
    check(
        between(),
        "between(5, 10, 3)",
        "{\"count\":0,\"exhausted\":true,\"solutions\":[]}",
        0,
    );
    check(
        between(),
        "findall(X, (X = 42, between(1, 1000000, X)), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[42],\"X\":\"_V\"}]}",
        1,
    );
    check(
        between(),
        "X = 50, \\+ between(1, 1000000, X)",
        "{\"count\":0,\"exhausted\":true,\"solutions\":[]}",
        0,
    );
}

#[test]
fn between_overflow_at_max_v1_divergence() {
    // v1 test_between_overflow_at_max: between(MAX, MAX, X) binds X = MAX.
    // plgc actual: {"count":1,...,"solutions":[{"X":-1}]} (exit 1).
    let g = format!("between({}, {}, X)", i64::MAX, i64::MAX);
    check(
        between(),
        &g,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":9223372036854775807}]}",
        1,
    );
}

// ---- succ / plus -----------------------------------------------------

#[test]
fn succ_and_plus() {
    // v1 test_succ_forward/backward/zero_fails + plus_forward/backward.
    check(
        prog(),
        "succ(3, X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":4}]}",
        1,
    );
    check(
        prog(),
        "succ(X, 5)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":4}]}",
        1,
    );
    check(
        prog(),
        "succ(X, 0)",
        "{\"count\":0,\"exhausted\":true,\"solutions\":[]}",
        0,
    );
    check(
        prog(),
        "plus(3, 4, X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":7}]}",
        1,
    );
    check(
        prog(),
        "plus(3, Y, 10)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"Y\":7}]}",
        1,
    );
    check(
        prog(),
        "plus(X, 4, 10)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":6}]}",
        1,
    );
}

// ---- findall over a defined predicate --------------------------------

#[test]
fn findall_filter_and_empty() {
    // v1 test_findall_with_filter + test_findall_empty_result.
    let c = compile(
        "score(alice, 85).\nscore(bob, 92).\nscore(carol, 78).\nscore(dave, 95).\n\
         missing(_) :- fail.\n",
    );
    check(
        &c,
        "findall(Name, (score(Name, S), S > 90), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"bob\",\"dave\"],\"Name\":\"_V\",\"S\":\"_V\"}]}",
        1,
    );
    check(
        &c,
        "findall(X, missing(X), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":\"[]\",\"X\":\"_V\"}]}",
        1,
    );
}

#[test]
fn sort_in_rule() {
    // v1 test_sort_in_rule.
    check(
        prog(),
        "sorted_names(X)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":[\"alice\",\"bob\",\"carol\"]}]}",
        1,
    );
}

#[test]
fn findall_once_collects_one() {
    // v1 test_findall_once_collects_one + test_findall_once_member.
    let c = compile("color(red). color(green). color(blue).\n");
    check(
        &c,
        "findall(X, once(color(X)), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"red\"],\"X\":\"_V\"}]}",
        1,
    );
    check(
        &c,
        "findall(X, once(member(X, [a,b,c])), L)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"a\"],\"X\":\"_V\"}]}",
        1,
    );
}

#[test]
fn cut_in_findall_stops_clause_iteration() {
    // v1 test_cut_in_findall_stops_clause_iteration + without_cut_collects_all.
    let c = compile("p(1). p(2). p(3).\n");
    check(
        &c,
        "findall(X, (p(X), !), Xs)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"_V\",\"Xs\":[1]}]}",
        1,
    );
    check(
        &c,
        "findall(X, p(X), Xs)",
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"_V\",\"Xs\":[1,2,3]}]}",
        1,
    );
}
