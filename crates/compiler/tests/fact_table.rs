//! Fact-table compilation (FACT_TABLE.md, Stage A): a predicate whose clauses
//! are all bodyless facts with immediate (atom/int) columns compiles to a
//! `.rodata` table + a generic runtime lookup instead of one function per
//! clause. These tests pin that the observable behavior is identical to the
//! per-clause path — solution order, ground queries, both lookup directions,
//! choice-point backtracking, and `findall`/`call` re-entry — and that a
//! mixed program (a compound-column fact predicate stays per-clause) works.
//!
//! Equivalence vs the per-clause/v1 behavior is also pinned by the broader
//! oracle-tested suites (these same fact predicates now route through the
//! table), and `deep_recursion_runs_in_constant_c_stack` in `integration.rs`
//! exercises 2000-deep recursion *through* a fact table under a 512KB stack —
//! validating that delivery to the continuation stays a `musttail`.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

/// Atom and int fact predicates, a recursive rule over a fact predicate, and
/// one compound-column fact predicate (which must stay per-clause).
fn facts() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| {
        compile(
            "parent(tom, bob).\n\
             parent(tom, liz).\n\
             parent(bob, ann).\n\
             age(alice, 30).\n\
             age(bob, 25).\n\
             edge(a, b).\n\
             edge(b, c).\n\
             edge(c, d).\n\
             path(X, X).\n\
             path(X, Z) :- edge(X, Y), path(Y, Z).\n\
             coords(origin, point(0, 0)).\n",
        )
    })
}

#[test]
fn enumerates_rows_in_program_order() {
    let (out, code) = facts().query("parent(tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"bob\"},{\"X\":\"liz\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn ground_query_success_and_failure() {
    let (out, code) = facts().query("parent(bob, ann)", &[]);
    assert_eq!(out, "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}\n");
    assert_eq!(code, 1);

    let (out, code) = facts().query("parent(bob, tom)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
}

#[test]
fn int_column_resolves_both_directions() {
    let (out, _) = facts().query("age(bob, A)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"A\":25}]}\n"
    );

    let (out, _) = facts().query("age(N, 30)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"N\":\"alice\"}]}\n"
    );
}

#[test]
fn findall_re_enters_the_table() {
    let (out, code) = facts().query("findall(X, parent(tom, X), L)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"bob\",\"liz\"],\"X\":\"_0\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn call_re_enters_the_table() {
    let (out, code) = facts().query("call(parent, tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"bob\"},{\"X\":\"liz\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn recursive_rule_over_a_fact_table() {
    // `path/2` recurses through the `edge` fact table; the base case plus the
    // three edges give a→{a,b,c,d}.
    let (out, code) = facts().query("path(a, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":4,\"exhausted\":true,\"solutions\":[{\"X\":\"a\"},{\"X\":\"b\"},{\"X\":\"c\"},{\"X\":\"d\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn limit_caps_table_enumeration() {
    // The choice-point shape honors `--limit` exactly like per-clause facts.
    let (out, code) = facts().query("parent(tom, X)", &["--limit", "1"]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":false,\"solutions\":[{\"X\":\"bob\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn compound_column_predicate_stays_per_clause() {
    // A ground fact with a compound column does NOT qualify for Stage A; it
    // compiles per-clause and coexists in the same binary.
    let (out, code) = facts().query("coords(origin, P)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"P\":{\"args\":[0,0],\"functor\":\"point\"}}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn undefined_predicate_still_raises_existence_error() {
    let (out, code) = facts().query("nosuch(X)", &[]);
    assert!(
        out.contains("existence_error(procedure, /(nosuch, 1))"),
        "{out}"
    );
    assert_eq!(code, 3);
}
