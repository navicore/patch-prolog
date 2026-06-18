//! Compiled-binary integration tests: compile fixture programs ONCE
//! per program (clang dominates test time), then assert stdout bytes
//! and exit codes for many queries — the v1 wire contract.

mod harness;
use harness::{Compiled, compile};
use std::process::Command;
use std::sync::OnceLock;

fn family() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| compile(include_str!("fixtures/family.pl")))
}

#[test]
fn facts_enumerate_in_program_order() {
    let (out, code) = family().query("parent(tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":3,\"exhausted\":true,\"solutions\":[{\"X\":\"mary\"},{\"X\":\"james\"},{\"X\":\"ann\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn ground_query_success_and_failure() {
    let (out, code) = family().query("parent(tom, mary)", &[]);
    assert_eq!(out, "{\"count\":1,\"exhausted\":true,\"solutions\":[{}]}\n");
    assert_eq!(code, 1);

    let (out, code) = family().query("parent(mary, tom)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0, "no solutions => exit 0 (linter-clean)");
}

#[test]
fn rule_with_conjunction() {
    let (out, code) = family().query("grandparent(tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"bob\"},{\"X\":\"carol\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn recursive_predicate_backtracks_through_both_clauses() {
    let (out, code) = family().query("ancestor(tom, X)", &[]);
    assert_eq!(
        out,
        "{\"count\":5,\"exhausted\":true,\"solutions\":[{\"X\":\"mary\"},{\"X\":\"james\"},{\"X\":\"ann\"},{\"X\":\"bob\"},{\"X\":\"carol\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn conjunctive_query_shares_bindings() {
    let (out, code) = family().query("parent(tom, X), parent(X, Y)", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":\"mary\",\"Y\":\"bob\"},{\"X\":\"james\",\"Y\":\"carol\"}]}\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn limit_and_exhausted_flag() {
    // limit < solutions: exhausted=false
    let (out, _) = family().query("parent(tom, X)", &["--limit", "2"]);
    assert!(out.contains("\"count\":2,\"exhausted\":false"), "{out}");
    // limit == solutions: also exhausted=false (v1 formula: count < limit)
    let (out, _) = family().query("parent(tom, X)", &["--limit", "3"]);
    assert!(out.contains("\"count\":3,\"exhausted\":false"), "{out}");
    // limit > solutions: exhausted=true
    let (out, _) = family().query("parent(tom, X)", &["--limit", "5"]);
    assert!(out.contains("\"count\":3,\"exhausted\":true"), "{out}");
}

#[test]
fn text_format() {
    let (out, code) = family().query("grandparent(tom, X)", &["--format", "text"]);
    assert_eq!(out, "X = bob\nX = carol\n");
    assert_eq!(code, 1);

    let (out, code) = family().query("parent(mary, tom)", &["--format", "text"]);
    assert_eq!(out, "false.\n");
    assert_eq!(code, 0);

    let (out, _) = family().query("parent(tom, mary)", &["--format", "text"]);
    assert_eq!(out, "true.\n");
}

#[test]
fn unknown_predicate_is_runtime_error_exit_3() {
    let (out, code) = family().query("nosuch(X)", &[]);
    assert_eq!(
        out,
        "{\"error\":\"Runtime error: error(existence_error(procedure, /(nosuch, 1)), Undefined procedure: nosuch/1)\"}\n"
    );
    assert_eq!(code, 3);
}

#[test]
fn query_parse_error_exit_2() {
    let (out, code) = family().query("parent(tom", &[]);
    assert!(out.starts_with("{\"error\":\"Parse error:"), "{out}");
    assert_eq!(code, 2);
}

#[test]
fn dynamic_predicate_fails_silently() {
    let c = compile(
        ":- dynamic(extra_data/2).\n\
         violation(X) :- extra_data(X, bad).\n\
         ok(yes).\n",
    );
    // Querying the dynamic predicate directly: no solutions, exit 0.
    let (out, code) = c.query("extra_data(X, Y)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
    // Through a rule body: also clean failure, not existence_error.
    let (out, code) = c.query("violation(X)", &[]);
    assert_eq!(out, "{\"count\":0,\"exhausted\":true,\"solutions\":[]}\n");
    assert_eq!(code, 0);
}

#[test]
fn undefined_in_rule_body_raises_when_reached() {
    let c = compile("go(X) :- missing(X).\nok(yes).\n");
    let (out, code) = c.query("go(X)", &[]);
    assert!(
        out.contains("existence_error(procedure, /(missing, 1))"),
        "{out}"
    );
    assert_eq!(code, 3);
    // Unreached undefined predicates don't error.
    let (_, code) = c.query("ok(X)", &[]);
    assert_eq!(code, 1);
}

#[test]
fn existence_error_carries_source_location() {
    // SPANS.md Layer 3 checkpoint 3: an undefined call in a clause body
    // names file:line:col. `missing(X)` is on line 2, indented 4 spaces, so
    // the call site is col 5.
    let c = compile("go(X) :-\n    missing(X).\n");
    let (out, code) = c.query("go(a)", &[]);
    assert_eq!(code, 3);
    // ISO term shape unchanged, plus the provenance suffix.
    assert!(out.contains("Undefined procedure: missing/1) at "), "{out}");
    assert!(out.contains("prog.pl:2:5"), "{out}");
}

#[test]
fn existence_error_in_disjunctive_body_carries_coarse_span() {
    // A top-level `;` collapses the whole body to one span
    // (parse_body_conjuncts), so an undefined call inside a disjunction
    // branch reports the BODY's start column, not the call's. Here the body
    // `fail ; missing` starts at line 2 col 5 (4-space indent), and that — not
    // `missing`'s own column — is what the suffix names. Pinned so a future
    // granularity refactor knows it's changing this behavior.
    let c = compile("go :-\n    fail ; missing.\n");
    let (out, code) = c.query("go", &[]);
    assert_eq!(code, 3);
    assert!(out.contains("Undefined procedure: missing/0) at "), "{out}");
    assert!(
        out.contains("prog.pl:2:5"),
        "coarse: body start, not `missing`: {out}"
    );
}

#[test]
fn query_side_existence_error_has_no_location_suffix() {
    // A directly-queried undefined predicate has no compiled call site;
    // its message must stay byte-identical to v1 (no ` at ...` suffix).
    let c = compile("ok(yes).\n");
    let (out, code) = c.query("nope(X)", &[]);
    assert_eq!(code, 3);
    assert!(
        !out.contains(" at "),
        "no provenance suffix expected: {out}"
    );
}

#[test]
fn arithmetic_evaluation_error_carries_source_location() {
    // SPANS.md Layer 3, Stage 2: an is/2 zero-divisor in a compiled body
    // names file:line:col. Conjunct `_ is 1 // 0` is line 2, col 5.
    let c = compile("go :-\n    _ is 1 // 0.\n");
    let (out, code) = c.query("go", &[]);
    assert_eq!(code, 3);
    assert!(out.contains("evaluation_error(zero_divisor)"), "{out}");
    assert!(out.contains("prog.pl:2:5"), "{out}");
}

#[test]
fn arithmetic_type_error_carries_source_location() {
    // is/2 evaluating a non-number atom → type_error, with provenance.
    let c = compile("go :-\n    _ is foo + 1.\n");
    let (out, code) = c.query("go", &[]);
    assert_eq!(code, 3);
    assert!(out.contains("type_error(evaluable, foo)"), "{out}");
    assert!(out.contains("prog.pl:2:5"), "{out}");
}

#[test]
fn arithmetic_comparison_error_carries_source_location() {
    // A comparison (`<`) evaluating a non-number also names the call site.
    let c = compile("go :-\n    1 < foo.\n");
    let (out, code) = c.query("go", &[]);
    assert_eq!(code, 3);
    assert!(out.contains("type_error(evaluable, foo)"), "{out}");
    assert!(out.contains("prog.pl:2:5"), "{out}");
}

#[test]
fn query_side_arith_error_has_no_location_suffix() {
    // Runtime-walked arithmetic (a query) has no compiled call site; the
    // message stays byte-identical to v1.
    let c = compile("ok(yes).\n");
    let (out, code) = c.query("_ is 1 // 0", &[]);
    assert_eq!(code, 3);
    assert!(out.contains("evaluation_error(zero_divisor)"), "{out}");
    assert!(!out.contains(" at "), "no suffix expected: {out}");
}

#[test]
fn step_limit_is_uncatchable_resource_error() {
    let c = compile("loop :- loop.\n");
    let (out, code) = c.query("loop", &[]);
    assert_eq!(
        out,
        "{\"error\":\"Runtime error: error(resource_error(steps), Maximum step limit exceeded (10000))\"}\n"
    );
    assert_eq!(code, 3);
}

#[test]
fn structured_terms_in_facts_and_queries() {
    let c = compile(
        "point(coord(1, 2)).\n\
         point(coord(3, 4)).\n\
         shape(box(coord(0, 0), coord(10, 10))).\n\
         items([a, b, c]).\n",
    );
    let (out, _) = c.query("point(coord(X, Y))", &[]);
    assert_eq!(
        out,
        "{\"count\":2,\"exhausted\":true,\"solutions\":[{\"X\":1,\"Y\":2},{\"X\":3,\"Y\":4}]}\n"
    );
    let (out, _) = c.query("point(P)", &["--limit", "1"]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":false,\"solutions\":[{\"P\":{\"args\":[1,2],\"functor\":\"coord\"}}]}\n"
    );
    let (out, _) = c.query("items(L)", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"L\":[\"a\",\"b\",\"c\"]}]}\n"
    );
    let (out, _) = c.query("items([H|T])", &[]);
    assert_eq!(
        out,
        "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"H\":\"a\",\"T\":[\"b\",\"c\"]}]}\n"
    );
}

#[test]
fn deep_recursion_runs_in_constant_c_stack() {
    // 2k-deep determinate recursion; would overflow a small stack if
    // musttail ever regressed. The binary inherits our rlimit, so
    // constrain the child via sh.
    let mut src = String::new();
    for i in 0..2000 {
        src.push_str(&format!("next(n{i}, n{}).\n", i + 1));
    }
    src.push_str("reach(X, X).\nreach(X, Z) :- next(X, Y), reach(Y, Z).\n");
    let c = compile(&src);
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s 512; PLG_MAX_STEPS=100000000 {} --query 'reach(n0, n2000)' --format text",
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
    assert_eq!(out.status.code(), Some(1));
}
