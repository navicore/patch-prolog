//! Integration tests for patch-prolog.
//! Tests the full pipeline: parse rules + query, solve, verify solutions.

use prolog_core::database::CompiledDatabase;
use prolog_core::parser::Parser;
use prolog_core::solver::{term_to_string, SolveResult, Solver};
use prolog_core::term::StringInterner;

/// Helper: parse source, parse query, solve, return solutions as strings.
fn solve_all(source: &str, query_str: &str) -> Vec<Vec<(String, String)>> {
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let solver = Solver::new(&db, goals, vars);
    let solutions = solver.all_solutions().unwrap();
    solutions
        .iter()
        .map(|sol| {
            sol.bindings
                .iter()
                .map(|(name, term)| (name.clone(), term_to_string(term, &db.interner)))
                .collect()
        })
        .collect()
}

/// Helper: parse source, parse query, solve with limit.
fn solve_with_limit(source: &str, query_str: &str, limit: usize) -> usize {
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let solver = Solver::new(&db, goals, vars).with_limit(limit);
    solver.all_solutions().unwrap().len()
}

/// Helper: check that a query produces an error.
fn solve_expect_error(source: &str, query_str: &str) -> String {
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars);
    match solver.next() {
        SolveResult::Error(e) => e,
        SolveResult::Success(_) => panic!("Expected error, got success"),
        SolveResult::Failure => panic!("Expected error, got failure"),
    }
}

/// Helper: get first binding value for a variable.
fn first_binding(source: &str, query_str: &str, var: &str) -> Option<String> {
    let solutions = solve_all(source, query_str);
    solutions.first().and_then(|bindings| {
        bindings
            .iter()
            .find(|(name, _)| name == var)
            .map(|(_, val)| val.clone())
    })
}

// ========================================================================
// Full pipeline tests
// ========================================================================

#[test]
fn test_family_relationships() {
    let source = r#"
        parent(tom, mary).
        parent(tom, james).
        parent(mary, ann).
        parent(mary, bob).
        grandparent(X, Z) :- parent(X, Y), parent(Y, Z).
        sibling(X, Y) :- parent(P, X), parent(P, Y), X \= Y.
    "#;

    // Grandparent
    let solutions = solve_all(source, "grandparent(tom, X)");
    assert_eq!(solutions.len(), 2);

    // Siblings
    let solutions = solve_all(source, "sibling(ann, X)");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "bob");
}

#[test]
fn test_arithmetic_pipeline() {
    let source = r#"
        factorial(0, 1).
        factorial(N, F) :- N > 0, N1 is N - 1, factorial(N1, F1), F is N * F1.
    "#;
    let result = first_binding(source, "factorial(5, X)", "X");
    assert_eq!(result, Some("120".to_string()));
}

#[test]
fn test_list_operations() {
    let source = r#"
        member(X, [X|_]).
        member(X, [_|T]) :- member(X, T).

        append([], L, L).
        append([H|T], L, [H|R]) :- append(T, L, R).

        length([], 0).
        length([_|T], N) :- length(T, N1), N is N1 + 1.
    "#;

    // member
    let solutions = solve_all(source, "member(X, [a, b, c])");
    assert_eq!(solutions.len(), 3);

    // append
    let result = first_binding(source, "append([1, 2], [3, 4], X)", "X");
    assert_eq!(result, Some("[1, 2, 3, 4]".to_string()));

    // length
    let result = first_binding(source, "length([a, b, c, d], N)", "N");
    assert_eq!(result, Some("4".to_string()));
}

#[test]
fn test_negation_as_failure_pipeline() {
    let source = r#"
        bird(tweety).
        bird(penguin).
        can_fly(X) :- bird(X), \+ penguin_species(X).
        penguin_species(penguin).
    "#;
    let solutions = solve_all(source, "can_fly(X)");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "tweety");
}

#[test]
fn test_cut_prevents_backtracking() {
    let source = r#"
        classify(X, positive) :- X > 0, !.
        classify(0, zero) :- !.
        classify(_, negative).
    "#;
    let result = first_binding(source, "classify(5, C)", "C");
    assert_eq!(result, Some("positive".to_string()));

    let result = first_binding(source, "classify(0, C)", "C");
    assert_eq!(result, Some("zero".to_string()));

    let result = first_binding(source, "classify(-3, C)", "C");
    assert_eq!(result, Some("negative".to_string()));
}

// ========================================================================
// Type-checking predicate tests
// ========================================================================

#[test]
fn test_type_checking_in_rules() {
    let source = r#"
        classify(X, integer) :- integer(X).
        classify(X, float) :- float(X).
        classify(X, atom) :- atom(X).
    "#;
    let result = first_binding(source, "classify(42, T)", "T");
    assert_eq!(result, Some("integer".to_string()));

    let result = first_binding(source, "classify(3.14, T)", "T");
    assert_eq!(result, Some("float".to_string()));

    let result = first_binding(source, "classify(hello, T)", "T");
    assert_eq!(result, Some("atom".to_string()));
}

// ========================================================================
// If-then-else tests
// ========================================================================

#[test]
fn test_if_then_else_in_rule() {
    let source = r#"
        abs(X, Y) :- (X < 0 -> Y is 0 - X ; Y = X).
    "#;
    let result = first_binding(source, "abs(-5, Y)", "Y");
    assert_eq!(result, Some("5".to_string()));

    let result = first_binding(source, "abs(3, Y)", "Y");
    assert_eq!(result, Some("3".to_string()));
}

#[test]
fn test_disjunction_in_rule() {
    let source = r#"
        primary_color(X) :- (X = red ; X = green ; X = blue).
    "#;
    let solutions = solve_all(source, "primary_color(X)");
    assert_eq!(solutions.len(), 3);
}

// ========================================================================
// findall tests
// ========================================================================

#[test]
fn test_findall_with_filter() {
    let source = r#"
        score(alice, 85).
        score(bob, 92).
        score(carol, 78).
        score(dave, 95).
    "#;
    let result = first_binding(
        source,
        "findall(Name, (score(Name, S), S > 90), L)",
        "L",
    );
    assert_eq!(result, Some("[bob, dave]".to_string()));
}

#[test]
fn test_findall_empty_result() {
    let source = "fact(a).";
    let result = first_binding(source, "findall(X, missing(X), L)", "L");
    assert_eq!(result, Some("[]".to_string()));
}

// ========================================================================
// Solution limit tests
// ========================================================================

#[test]
fn test_solution_limit_respected() {
    let source = "n(1). n(2). n(3). n(4). n(5). n(6). n(7). n(8). n(9). n(10).";
    assert_eq!(solve_with_limit(source, "n(X)", 3), 3);
    assert_eq!(solve_with_limit(source, "n(X)", 100), 10);
}

// ========================================================================
// Robustness / edge case tests
// ========================================================================

#[test]
fn test_depth_limit_prevents_stack_overflow() {
    let source = "infinite :- infinite.";
    let err = solve_expect_error(source, "infinite");
    assert!(err.contains("depth"));
}

#[test]
fn test_depth_limit_custom() {
    let source = "loop :- loop.";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars("loop", &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(50);
    match solver.next() {
        SolveResult::Error(e) => assert!(e.contains("50")),
        other => panic!("Expected error, got {:?}", other),
    }
}

#[test]
fn test_integer_overflow_detected() {
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let query_str = format!("X is {} + 1", i64::MAX);
    let (goals, vars) = Parser::parse_query_with_vars(&query_str, &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars);
    match solver.next() {
        SolveResult::Error(e) => assert!(e.contains("overflow")),
        other => panic!("Expected error, got {:?}", other),
    }
}

#[test]
fn test_empty_knowledge_base() {
    let solutions = solve_all("", "foo(X)");
    assert!(solutions.is_empty());
}

#[test]
fn test_no_matching_predicate() {
    let source = "color(red). color(blue).";
    let solutions = solve_all(source, "shape(X)");
    assert!(solutions.is_empty());
}

#[test]
fn test_division_by_zero() {
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) =
        Parser::parse_query_with_vars("X is 10 / 0", &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars);
    match solver.next() {
        SolveResult::Error(e) => assert!(e.contains("zero")),
        other => panic!("Expected error, got {:?}", other),
    }
}

#[test]
fn test_unbound_variable_in_arithmetic() {
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars("X is Y + 1", &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars);
    match solver.next() {
        SolveResult::Error(e) => assert!(e.contains("unbound")),
        other => panic!("Expected error, got {:?}", other),
    }
}

// ========================================================================
// Stdlib tests (list predicates compiled from knowledge/stdlib.pl)
// ========================================================================

#[test]
fn test_complex_recursive_query() {
    let source = r#"
        parent(tom, mary).
        parent(mary, ann).
        parent(ann, alice).
        ancestor(X, Y) :- parent(X, Y).
        ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).
    "#;
    let solutions = solve_all(source, "ancestor(tom, X)");
    assert_eq!(solutions.len(), 3); // mary, ann, alice
}

#[test]
fn test_parse_error_detection() {
    let mut interner = StringInterner::new();
    let result = Parser::parse_program("invalid(((.", &mut interner);
    assert!(result.is_err());
}

#[test]
fn test_multiple_queries_sequential() {
    let source = "color(red). color(green). color(blue). shape(circle). shape(square).";

    let colors = solve_all(source, "color(X)");
    assert_eq!(colors.len(), 3);

    let shapes = solve_all(source, "shape(X)");
    assert_eq!(shapes.len(), 2);
}

#[test]
fn test_ground_query_true() {
    let source = "likes(mary, food).";
    let solutions = solve_all(source, "likes(mary, food)");
    assert_eq!(solutions.len(), 1);
    assert!(solutions[0].is_empty()); // no variables to bind
}

#[test]
fn test_ground_query_false() {
    let source = "likes(mary, food).";
    let solutions = solve_all(source, "likes(mary, beer)");
    assert!(solutions.is_empty());
}
