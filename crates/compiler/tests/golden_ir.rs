//! Golden-IR tests: assert structural properties of generated IR
//! without invoking clang. Fast regression net for codegen.

#[test]
fn fact_compiles_to_unify_and_continuation_jump() {
    // A ground fact with a non-immediate (compound) column does NOT qualify
    // for fact-table compilation, so it exercises the per-clause path: head
    // unification + a tail call to the continuation. (All-immediate facts
    // take the table path — see `fact_predicate_compiles_to_rodata_table`.)
    let ir = plgc::compile_to_ir("parent(tom, point(1, 2)).").unwrap();
    // Entry exists and is registered.
    assert!(ir.contains("define i32 @plg_pred_"), "{ir}");
    assert!(ir.contains("@plg_registry"), "{ir}");
    assert!(ir.contains("@plg_atom_strs"), "{ir}");
    // Head constants are immediate tagged words (no runtime atom lookup).
    assert!(ir.contains("call i32 @plg_rt_unify(ptr %m"), "{ir}");
    // Solution delivery is a guaranteed tail call.
    assert!(ir.contains("musttail call i32"), "{ir}");
}

#[test]
fn multi_clause_predicate_pushes_choice_points() {
    // Compound columns keep this on the per-clause path; multiple clauses
    // then lazily link choice points with chain retry functions. (The
    // all-immediate variant compiles to a table instead.)
    let ir = plgc::compile_to_ir("p(f(a)).\np(f(b)).\np(f(c)).").unwrap();
    assert!(ir.contains("call void @plg_rt_push_cp"), "{ir}");
    // Chain functions t1, t2 for clauses 2 and 3.
    assert!(ir.contains("_t1(ptr %m"), "{ir}");
    assert!(ir.contains("_t2(ptr %m"), "{ir}");
}

#[test]
fn fact_predicate_compiles_to_rodata_table() {
    // All clauses are bodyless facts with immediate (atom/int) columns →
    // one `.rodata` table of words + a generic runtime lookup, NOT one
    // function per clause (FACT_TABLE.md Stage A).
    let ir =
        plgc::compile_to_ir("parent(tom, bob).\nparent(tom, liz).\nparent(bob, ann).").unwrap();
    // A private constant table: 3 facts × 2 columns = 6 immediate words.
    assert!(ir.contains("@plg_facts_"), "{ir}");
    assert!(
        ir.contains("private unnamed_addr constant [6 x i64]"),
        "{ir}"
    );
    // Entry finds the first matching row; the retry continuation resumes the
    // scan on backtracking (the choice point itself lives in the runtime).
    assert!(ir.contains("call i32 @plg_rt_fact_first(ptr %m"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_fact_next(ptr %m"), "{ir}");
    assert!(ir.contains("_ftr(ptr %m, i64 %f)"), "{ir}");
    // Delivery is still a guaranteed tail call to the continuation.
    assert!(ir.contains("musttail call i32"), "{ir}");
    // `parent` took the fact-table path, not per-clause (whose header would
    // read "(N clauses)"). The whole-IR no-unify check is confounded by the
    // embedded stdlib rules, so we assert the predicate header instead.
    assert!(ir.contains("parent/2 (3 facts \u{2192} table)"), "{ir}");
}

#[test]
fn single_clause_predicate_pushes_no_choice_point() {
    let ir = plgc::compile_to_ir("only(x).").unwrap();
    assert!(!ir.contains("call void @plg_rt_push_cp"), "{ir}");
}

#[test]
fn every_musttail_is_followed_by_ret() {
    // The musttail/ret pairing is what guarantees constant-stack
    // recursion; a regression here is a stack overflow in the field.
    let ir = plgc::compile_to_ir("p(a).\np(b).\nq(X) :- p(X), p(X).\nr(X) :- q(X).").unwrap();
    let lines: Vec<&str> = ir.lines().collect();
    let mut count = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.contains("musttail call") {
            count += 1;
            let next = lines.get(i + 1).copied().unwrap_or("");
            assert!(
                next.trim_start().starts_with("ret i32"),
                "musttail not followed by ret at line {i}: {line} / {next}"
            );
        }
    }
    assert!(count >= 4, "expected several musttail sites, got {count}");
}

#[test]
fn dynamic_only_predicates_register_fail_stub() {
    let ir = plgc::compile_to_ir(":- dynamic(extra/2).\np(a).").unwrap();
    assert!(
        ir.contains("ptr @plg_rt_pred_fail"),
        "dynamic registry row should point at the fail stub: {ir}"
    );
}

#[test]
fn last_body_goal_restores_caller_continuation() {
    let ir = plgc::compile_to_ir("a(X) :- b(X), c(X).\nb(1).\nc(1).").unwrap();
    // The continuation for the last goal loads k from the body frame
    // (slots 0/1) and reinstalls it — last-call optimization.
    assert!(ir.contains("_a1(ptr %m, i64 %bf)"), "{ir}");
    assert!(ir.contains("@plg_rt_set_k"), "{ir}");
}

#[test]
fn m4_control_builtins_emit_runtime_calls() {
    let ir = plgc::compile_to_ir(
        "p(X) :- findall(Y, q(Y), X).\n\
         s(X) :- catch(q(X), _, fail).\n\
         t :- throw(boom).\n\
         c(X) :- call(q, X).\n\
         m(G) :- G.\n\
         b(X) :- between(1, 3, X).\n\
         q(a).",
    )
    .unwrap();
    assert!(ir.contains("call i32 @plg_rt_b_findall_3"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_catch_3"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_throw_1"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_metacall"), "{ir}");
    // between/3 dispatches like a predicate (uniform signature).
    assert!(
        ir.contains("musttail call i32 @plg_rt_pred_between_3"),
        "{ir}"
    );
}

#[test]
fn m4_det_builtins_emit_inline_calls() {
    let ir = plgc::compile_to_ir(
        "p(X, L) :- atom(X), atom_chars(X, L).\n\
         w(X) :- write(X), nl.",
    )
    .unwrap();
    assert!(ir.contains("call i32 @plg_rt_b_atom_1"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_atom_chars_2"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_write_1"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_nl_0"), "{ir}");
}

#[test]
fn m3_control_compiles_natively() {
    // NAF, if-then-else, and cut are compiled control flow now — no
    // unsupported stubs, real choice points and commit heights.
    let ir = plgc::compile_to_ir(
        "p(X) :- \\+ q(X).\n\
         r(X, S) :- (q(X) -> S = yes ; S = no).\n\
         m(X, Y, X) :- X >= Y, !.\n\
         m(_, Y, Y).\n\
         q(a).",
    )
    .unwrap();
    assert!(!ir.contains("call i32 @plg_rt_unsupported_builtin"), "{ir}");
    assert!(ir.contains("call void @plg_rt_cut"), "{ir}");
    assert!(ir.contains("call i64 @plg_rt_cp_top"), "{ir}");
    assert!(ir.contains("call i32 @plg_rt_b_arith_cmp"), "{ir}");
}

#[test]
fn first_arg_indexing_emits_switch() {
    // Compound second columns keep these on the per-clause path; distinct
    // atom first arguments then drive first-argument indexing as an IR
    // `switch`. (All-immediate facts compile to a table, which Stage A does
    // not index yet.)
    let ir =
        plgc::compile_to_ir("color(red, c(warm)).\ncolor(blue, c(cool)).\ncolor(green, c(cool)).")
            .unwrap();
    assert!(ir.contains("switch i64"), "{ir}");
    assert!(ir.contains(", indexed"), "{ir}");
    // Distinct keys + no var-keyed clauses: each key chain is a single
    // candidate ⇒ deterministic dispatch pushes NO choice point. The
    // only push_cp in the entry belongs to the unbound-argument (REF)
    // path, which must try all three clauses.
    let entry = ir.split("define i32 @plg_pred_").nth(1).unwrap();
    let entry_fn = &entry[..entry.find("\n}").unwrap()];
    assert_eq!(
        entry_fn.matches("call void @plg_rt_push_cp").count(),
        1,
        "keyed chains should be deterministic (only the REF/all chain pushes):\n{entry_fn}"
    );
}

#[test]
fn fail_compiles_to_immediate_failure() {
    let ir = plgc::compile_to_ir("p(a) :- fail.\np(b).").unwrap();
    assert!(ir.contains("define"), "{ir}");
}

#[test]
fn big_integer_literals_box_at_runtime() {
    // M4: beyond-immediate integers compile to a runtime BIG box
    // (full i64 range, v1 parity).
    let ir = plgc::compile_to_ir(&format!("big({}).", i64::MAX)).unwrap();
    assert!(
        ir.contains(&format!(
            "call i64 @plg_rt_put_big(ptr %m, i64 {})",
            i64::MAX
        )),
        "{ir}"
    );
}
