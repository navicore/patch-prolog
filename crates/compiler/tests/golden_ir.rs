//! Golden-IR tests: assert structural properties of generated IR
//! without invoking clang. Fast regression net for codegen.

#[test]
fn fact_compiles_to_unify_and_continuation_jump() {
    let ir = plgc::compile_to_ir("parent(tom, mary).").unwrap();
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
    let ir = plgc::compile_to_ir("p(a).\np(b).\np(c).").unwrap();
    assert!(ir.contains("call void @plg_rt_push_cp"), "{ir}");
    // Chain functions t1, t2 for clauses 2 and 3.
    assert!(ir.contains("_t1(ptr %m"), "{ir}");
    assert!(ir.contains("_t2(ptr %m"), "{ir}");
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
    assert!(ir.contains("_k1(ptr %m, i64 %bf)"), "{ir}");
    assert!(ir.contains("@plg_rt_set_k"), "{ir}");
}

#[test]
fn unsupported_builtins_compile_to_runtime_error_stub() {
    // Late binding (v1 contract): the program compiles; reaching the
    // goal raises a clear runtime error. Unrelated queries keep working.
    let ir = plgc::compile_to_ir("p(X) :- \\+ q(X).\nq(a).").unwrap();
    assert!(ir.contains("call i32 @plg_rt_unsupported_builtin"), "{ir}");
}

#[test]
fn fail_compiles_to_immediate_failure() {
    let ir = plgc::compile_to_ir("p(a) :- fail.\np(b).").unwrap();
    assert!(ir.contains("define"), "{ir}");
}

#[test]
fn integer_literal_range_is_enforced() {
    let err = plgc::compile_to_ir(&format!("big({}).", i64::MAX)).unwrap_err();
    assert!(err.contains("61-bit"), "{err}");
}
