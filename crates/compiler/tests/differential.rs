//! Differential tests: the same (program, goal) pairs through the v1
//! interpreter (`../patch-prolog`'s prlg, the semantics oracle) and
//! through plgc-compiled binaries; normalized JSON must match.
//!
//! SKIPPED automatically when the oracle binary is absent (CI runners
//! don't have the old repo). Run locally via `just diff-test`.
//!
//! Known, deliberate divergences are NOT in this corpus — they live as
//! direct assertions in control_arith.rs with ISO_COMPLIANCE.md
//! references (cut transparency in `;`).

mod harness;
use std::path::Path;
use std::process::Command;

const ORACLE: &str = "../../../patch-prolog/target/release/prlg";

const PROGRAM: &str = "\
p(1). p(2). p(3).
q(a). q(b).
data(point(1,2)). data(point(3,4)).
edge(a, b). edge(b, c). edge(c, d).
path(X, X).
path(X, Z) :- edge(X, Y), path(Y, Z).
adult(X) :- age(X, N), N >= 18.
age(alice, 30). age(bob, 12).
classify(X, neg) :- X < 0.
classify(0, zero).
classify(X, pos) :- X > 0.
div_in_body :- _ is 1 // 0.
type_in_body :- atom_length(123, _).
";

const GOALS: &[&str] = &[
    // resolution, backtracking, recursion
    "p(X)",
    "path(a, X)",
    "path(X, d)",
    "adult(X)",
    "classify(-3, C)",
    "data(point(X, Y))",
    // control
    "(p(X), X > 1 ; q(X))",
    "\\+ p(9)",
    "(p(9) -> a = b ; true)",
    "once(p(X))",
    "p(X), !",
    // arithmetic
    "X is 2 + 3 * 4",
    "X is -7 mod 3",
    "X is 2 ^ 10",
    "X is 1 // 0",
    "X is foo",
    "1.5 < 2",
    "X is max(2, 7) + min(1, 0)",
    // unification / comparison
    "f(X, b) = f(a, Y)",
    "a \\= b",
    "compare(O, f(a), g(a))",
    "X @< Y",
    "1.0 == 1.0",
    // builtins
    "functor(point(1,2), F, A)",
    "T =.. [foo, 1]",
    "arg(1, point(a,b), X)",
    "copy_term(f(X, X), T)",
    "atom_length(hello, N)",
    "atom_concat(ab, cd, X)",
    "atom_chars(abc, L)",
    "number_chars(N, ['4','2'])",
    "msort([c,a,b,a], L)",
    "sort([c,a,b,a], L)",
    "succ(4, X)",
    "plus(2, X, 9)",
    "var(X)",
    "is_list([1,2,3])",
    "compound([1])",
    // stdlib (compiled into every plgc binary, embedded in v1)
    "member(X, [1,2,3])",
    "append(X, Y, [1,2])",
    "length([a,b,c], N)",
    "reverse([1,2,3], R)",
    "nth0(1, [a,b,c], X)",
    "last([1,2,3], X)",
    // findall / call / between / catch
    "findall(X, p(X), L)",
    "findall(X-Y, (p(X), q(Y)), L)",
    "findall(X, nosuch(X), L)",
    "call(p, X)",
    "G = q(X), call(G)",
    "between(1, 4, X)",
    "catch(throw(t(9)), t(N), true)",
    "catch(X is 1 // 0, error(evaluation_error(E), _), true)",
    "catch(throw(unmatched), other, true)",
    "throw(boom)",
    // errors
    "nosuch(X)",
    "atom_length(123, N)",
    "succ(X, 0)",
    // compiled-body raises: plgc adds a ` at file:line:col` provenance
    // suffix the oracle lacks; `strip_provenance` must reconcile them.
    "div_in_body",
    "type_in_body",
];

/// Strip a ` at <file>:<line>:<col>` provenance suffix (SPANS.md Layer 3).
/// plgc appends it to a runtime error raised from a *compiled* clause body;
/// the v1 oracle never emits it, so it's a deliberate, expected divergence we
/// remove before comparing. The suffix runs to the end of the error string
/// (the closing `"` of the JSON, or end of line).
fn strip_provenance(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(at) = out.rfind(" at ") {
        let rest = &out[at + 4..];
        let end = rest.find(['"', '\n']).unwrap_or(rest.len());
        if is_file_line_col(&rest[..end]) {
            out.replace_range(at..at + 4 + end, "");
        } else {
            break; // the rightmost ` at ` isn't a provenance suffix
        }
    }
    out
}

/// Does `s` look like `<file>:<line>:<col>` (ends with `:digits:digits`)?
fn is_file_line_col(s: &str) -> bool {
    let mut parts = s.rsplitn(3, ':');
    let (col, line) = (parts.next(), parts.next());
    let digits =
        |x: Option<&str>| x.is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
    parts.next().is_some() && digits(line) && digits(col)
}

/// Normalize variable numbering (`_12` → `_V`) — the only legitimate
/// difference between the two implementations.
fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' && chars.peek().is_some_and(|n| n.is_ascii_digit()) {
            out.push_str("_V");
            while chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn differential_corpus_matches_oracle() {
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join(ORACLE);
    if !oracle.exists() {
        eprintln!(
            "differential: oracle not found at {}; skipping",
            oracle.display()
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let pl = dir.path().join("prog.pl");
    std::fs::write(&pl, PROGRAM).unwrap();
    let compiled = harness::compile(PROGRAM);

    let mut failures = Vec::new();
    for goal in GOALS {
        let old = Command::new(&oracle)
            .args(["run"])
            .arg(&pl)
            .args(["--goal", goal, "--format", "json"])
            .output()
            .expect("run oracle");
        let old_out = String::from_utf8_lossy(&old.stdout).into_owned();
        let old_code = old.status.code().unwrap_or(-1);

        let (new_out, new_code) = compiled.query(goal, &[]);

        // Strip plgc's provenance suffix (the oracle never emits it) before
        // comparing — see `strip_provenance`.
        if norm(&strip_provenance(&old_out)) != norm(&strip_provenance(&new_out))
            || old_code != new_code
        {
            failures.push(format!(
                "GOAL {goal}\n  oracle({old_code}): {old_out}  plgc({new_code}): {new_out}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} goals diverged from the oracle:\n{}",
        failures.len(),
        GOALS.len(),
        failures.join("\n")
    );
}

#[test]
fn strip_provenance_removes_suffix() {
    // The provenance suffix is removed; everything else (including the ISO
    // ball with its own digits/colons) is untouched, so a stripped plgc
    // error equals the oracle's.
    let plgc = "{\"error\":\"Runtime error: error(evaluation_error(zero_divisor), Division by zero) at /tmp/x/prog.pl:2:5\"}";
    let oracle =
        "{\"error\":\"Runtime error: error(evaluation_error(zero_divisor), Division by zero)\"}";
    assert_eq!(strip_provenance(plgc), oracle);
    // No suffix → unchanged (query-side errors, success output).
    assert_eq!(strip_provenance(oracle), oracle);
    let ok = "{\"count\":1,\"exhausted\":true,\"solutions\":[{\"X\":1}]}";
    assert_eq!(strip_provenance(ok), ok);
    // A bare ` at ` that isn't a file:line:col suffix is left alone.
    let prose = "looked at the value";
    assert_eq!(strip_provenance(prose), prose);
}
