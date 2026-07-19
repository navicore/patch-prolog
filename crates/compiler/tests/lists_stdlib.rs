//! Stdlib list predicates (member/append/length/reverse/last),
//! is_list, copy_term, between, succ/plus, and findall over lists.
//!
//! These rely on plgc's built-in stdlib; the fixture program does NOT
//! redefine member/append (doing so would duplicate the stdlib clauses
//! and inflate solution counts). Output is the readable text format.

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
    check(prog(), "member(X, [a, b, c])", "X = a\nX = b\nX = c", 1);
    check(prog(), "append([1, 2], [3, 4], X)", "X = [1, 2, 3, 4]", 1);
    check(prog(), "length([a, b, c, d], N)", "N = 4", 1);
}

#[test]
fn reverse_and_last() {
    check(prog(), "reverse([1, 2, 3], X)", "X = [3, 2, 1]", 1);
    check(prog(), "last([1, 2, 3], X)", "X = 3", 1);
}

// ---- length/2 robustness (issue #54) --------------------------------
// The naive recursive definition diverged on three reachable inputs:
// build-mode enumeration (`length(L, 3)` with no --limit), negative
// length (`length(L, -1)`), and any generate-and-test using length/2 to
// build a skeleton. The stdlib now fails fast when N is a bound negative
// integer and is deterministic in build mode, so backtracking past the
// unique solution fails cleanly instead of recursing into negative
// counts. These tests assert completion + exit code — pre-fix they hit
// resource_error(steps) (exit 3) after grinding through the step limit.

#[test]
fn length_build_mode_enumerates_once() {
    // Pre-fix: diverged on enumeration. Now: exactly one solution, then
    // no more — the engine exhausts and exits 1.
    check(prog(), "length(L, 3)", "L = [_V, _V, _V]", 1);
    check(prog(), "length(L, 0)", "L = []", 1);
    check(prog(), "length([], N)", "N = 0", 1);
}

#[test]
fn length_negative_bound_fails_fast() {
    // Pre-fix: diverged, building an ever-longer list looking for a
    // negative length. Now: no list has negative length → clean failure.
    check(prog(), "length(L, -1)", "false.", 0);
    check(prog(), "length(L, -5)", "false.", 0);
}

#[test]
fn length_findall_in_build_mode_collects_once() {
    // findall forces enumeration of length/2's choice points — the exact
    // path that diverged pre-fix. Now it collects exactly one list.
    check(
        prog(),
        "findall(L, length(L, 3), Ls)",
        "L = _V\nLs = [[_V, _V, _V]]",
        1,
    );
}

// The textbook generate-and-test idiom (N-queens style): length/2 builds
// the skeleton, then backtracking enumerates fillings. Pre-fix this
// diverged as soon as the engine backtracked into length/2's choice points.
const GENTEST: &str = "\
fill_all([], _).
fill_all([H|T], D) :- member(H, D), fill_all(T, D).
gt(Sol) :- length(Sol, 2), fill_all(Sol, [a, b]).
";

fn gentest() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(GENTEST))
}

#[test]
fn length_generate_and_test_completes() {
    check(
        gentest(),
        "findall(S, gt(S), Ss)",
        "S = _V\nSs = [[a, a], [a, b], [b, a], [b, b]]",
        1,
    );
}

#[test]
fn is_list_predicate() {
    check(prog(), "X = [1,2,3], is_list(X)", "X = [1, 2, 3]", 1);
    check(prog(), "my_list(X), is_list(X)", "X = [1, 2, 3]", 1);
}

// ---- copy_term -------------------------------------------------------

#[test]
fn copy_term_ground_and_lists() {
    check(prog(), "copy_term(hello, Copy)", "Copy = hello", 1);
    check(prog(), "copy_term(42, Copy)", "Copy = 42", 1);
    check(prog(), "copy_term([a, b, c], X)", "X = [a, b, c]", 1);
    check(
        prog(),
        "copy_term([1,2,3,4,5,6,7,8,9,10], X)",
        "X = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]",
        1,
    );
}

#[test]
fn copy_term_fresh_vars_and_aliasing() {
    // Fresh-var ids normalized via norm(). Bindings sorted by name.
    check(
        prog(),
        "copy_term(f(X, Y), Copy)",
        "Copy = f(_V, _V)\nX = _V\nY = _V",
        1,
    );
    check(
        prog(),
        "copy_term(f(X, X), f(A, B)), A = hello",
        "A = hello\nB = hello\nX = _V",
        1,
    );
    // copy_term/2 copies arg1 to a FRESH term, then unifies the copy with
    // [1,2,3]; the originals A,B,C stay unbound (ISO semantics).
    check(
        prog(),
        "copy_term([A, B, C], [1, 2, 3])",
        "A = _V\nB = _V\nC = _V",
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
    check(
        between(),
        "between(1, 5, X)",
        "X = 1\nX = 2\nX = 3\nX = 4\nX = 5",
        1,
    );
    check(between(), "between(3, 3, X)", "X = 3", 1);
    check(between(), "between(5, 3, X)", "false.", 0);
}

#[test]
fn between_in_rule_and_findall() {
    check(
        between(),
        "digit(D)",
        "D = 0\nD = 1\nD = 2\nD = 3\nD = 4\nD = 5\nD = 6\nD = 7\nD = 8\nD = 9",
        1,
    );
    check(
        between(),
        "findall(X, between(1, 5, X), L)",
        "L = [1, 2, 3, 4, 5]\nX = _V",
        1,
    );
}

#[test]
fn between_bound_x_fast_paths() {
    check(between(), "between(1, 1000000, 5)", "true.", 1);
    check(between(), "between(1, 10, 11)", "false.", 0);
    check(between(), "between(5, 10, 3)", "false.", 0);
    check(
        between(),
        "findall(X, (X = 42, between(1, 1000000, X)), L)",
        "L = [42]\nX = _V",
        1,
    );
    check(between(), "X = 50, \\+ between(1, 1000000, X)", "false.", 0);
}

#[test]
fn between_overflow_at_max() {
    // between(MAX, MAX, X) binds X = MAX.
    let g = format!("between({}, {}, X)", i64::MAX, i64::MAX);
    check(between(), &g, "X = 9223372036854775807", 1);
}

// ---- succ / plus -----------------------------------------------------

#[test]
fn succ_and_plus() {
    check(prog(), "succ(3, X)", "X = 4", 1);
    check(prog(), "succ(X, 5)", "X = 4", 1);
    check(prog(), "succ(X, 0)", "false.", 0);
    check(prog(), "plus(3, 4, X)", "X = 7", 1);
    check(prog(), "plus(3, Y, 10)", "Y = 7", 1);
    check(prog(), "plus(X, 4, 10)", "X = 6", 1);
}

// ---- findall over a defined predicate --------------------------------

#[test]
fn findall_filter_and_empty() {
    let c = compile(
        "score(alice, 85).\nscore(bob, 92).\nscore(carol, 78).\nscore(dave, 95).\n\
         missing(_) :- fail.\n",
    );
    check(
        &c,
        "findall(Name, (score(Name, S), S > 90), L)",
        "L = [bob, dave]\nName = _V\nS = _V",
        1,
    );
    check(&c, "findall(X, missing(X), L)", "L = []\nX = _V", 1);
}

#[test]
fn sort_in_rule() {
    check(prog(), "sorted_names(X)", "X = [alice, bob, carol]", 1);
}

#[test]
fn findall_once_collects_one() {
    let c = compile("color(red). color(green). color(blue).\n");
    check(&c, "findall(X, once(color(X)), L)", "L = [red]\nX = _V", 1);
    check(
        &c,
        "findall(X, once(member(X, [a,b,c])), L)",
        "L = [a]\nX = _V",
        1,
    );
}

#[test]
fn cut_in_findall_stops_clause_iteration() {
    let c = compile("p(1). p(2). p(3).\n");
    check(&c, "findall(X, (p(X), !), Xs)", "X = _V\nXs = [1]", 1);
    check(&c, "findall(X, p(X), Xs)", "X = _V\nXs = [1, 2, 3]", 1);
}
