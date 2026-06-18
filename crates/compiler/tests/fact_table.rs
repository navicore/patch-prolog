//! Fact-table compilation (FACT_TABLE.md, Stages A–C): a predicate whose
//! clauses are all bodyless facts with ground head args compiles to a `.rodata`
//! table + a generic runtime lookup instead of one function per clause. These
//! tests pin that the observable behavior is identical to the per-clause path —
//! solution order, ground queries, both lookup directions, choice-point
//! backtracking, `--limit`, `findall`/`call` re-entry — across all column kinds
//! (immediate atom/int; serialized compound/list/float/big-int), including the
//! first-arg index (immediate column 0) and the unindexed full scan (compound
//! column 0).
//!
//! Equivalence vs the per-clause/v1 behavior is also pinned by the broader
//! oracle-tested suites (these same fact predicates now route through the
//! table), and `deep_recursion_runs_in_constant_c_stack` in `integration.rs`
//! exercises 2000-deep recursion *through* a fact table under a 512KB stack —
//! validating that delivery to the continuation stays a `musttail`.

mod harness;
use harness::{Compiled, compile};
use std::sync::OnceLock;

/// Atom and int fact predicates, plus a recursive rule over a fact predicate.
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
             path(X, Z) :- edge(X, Y), path(Y, Z).\n",
        )
    })
}

/// Stage C: non-immediate columns (compound, list, float, big-int), a row
/// mixing an indexed immediate column 0 with a compound column 1, and a
/// compound column 0 (which is not indexed → full scan).
fn stage_c() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| {
        compile(
            "data(point(1, 2)).\n\
             data(point(3, 4)).\n\
             tags(a, [x, y, z]).\n\
             pii(p, 3.14).\n\
             big(b, 9223372036854775807).\n\
             m(1, point(2, 3)).\n\
             m(2, point(4, 5)).\n\
             rel(point(1, 2), aa).\n\
             rel(point(3, 4), bb).\n",
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
fn indexed_lookup_gathers_nonadjacent_rows_in_program_order() {
    // The same first-arg key (`tom`) appears at non-adjacent program positions
    // (rows 0, 3, 4). Stage B's first-arg index must binary-search to that key
    // and visit its rows in program order, skipping the interleaved `bob`/`cat`
    // rows — byte-identical to a Stage-A full scan.
    let c = compile("rel(tom, a).\nrel(bob, b).\nrel(cat, c).\nrel(tom, d).\nrel(tom, e).\n");
    let (out, code) = c.query("rel(tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":3,\"exhausted\":true,\"solutions\":[{\"X\":\"a\"},{\"X\":\"d\"},{\"X\":\"e\"}]}\n"
    );
    assert_eq!(code, 1);

    // A bound key absent from the table fast-fails (empty binary-search range).
    let (out, code) = c.query("rel(zzz, X)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
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
fn compound_column_enumerates_and_partially_binds() {
    // Stage C: ground compound columns are serialized into the blob and
    // restored onto the heap before unifying.
    let (out, code) = stage_c().query("data(P)", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"P\":{\"args\":[1,2],\"functor\":\"point\"}},{\"P\":{\"args\":[3,4],\"functor\":\"point\"}}]}\n"
    );
    assert_eq!(code, 1);

    // A partially-bound compound query unifies against the restored term.
    let (out, code) = stage_c().query("data(point(3, X))", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":4}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn list_column_restores() {
    let (out, code) = stage_c().query("tags(a, L)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"x\",\"y\",\"z\"]}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn float_column_resolves_both_directions() {
    let (out, _) = stage_c().query("pii(p, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":3.14}]}\n"
    );
    // Ground float query unifies (bit-identical FLT restore).
    let (out, code) = stage_c().query("pii(p, 3.14)", &[]);
    assert_eq!(out, "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}\n");
    assert_eq!(code, 1);
}

#[test]
fn big_int_column_restores() {
    // Beyond the i61 immediate range: a BIG blob cell, restored on lookup.
    let (out, code) = stage_c().query("big(b, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":9223372036854775807}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn mixed_immediate_and_compound_columns() {
    // Column 0 is an indexed immediate; column 1 is a restored compound.
    let (out, code) = stage_c().query("m(1, P)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"P\":{\"args\":[2,3],\"functor\":\"point\"}}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn compound_column_zero_full_scans() {
    // A compound column 0 has no first-arg index (not u64-sortable); a bound
    // compound key resolves by full scan + restore + unify.
    let (out, code) = stage_c().query("rel(point(3, 4), X)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":\"bb\"}]}\n"
    );
    assert_eq!(code, 1);

    // Partial compound key over the unindexed column.
    let (out, code) = stage_c().query("rel(point(N, 4), Y)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"N\":3,\"Y\":\"bb\"}]}\n"
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
