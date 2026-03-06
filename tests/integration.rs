//! Integration tests for patch-prolog.
//! Tests the full pipeline: parse rules + query, solve, verify solutions.

use patch_prolog_core::database::CompiledDatabase;
use patch_prolog_core::parser::Parser;
use patch_prolog_core::solver::{term_to_string, SolveResult, Solver};
use patch_prolog_core::term::StringInterner;

/// Helper: parse source, parse query, solve, return solutions as strings.
fn solve_all(source: &str, query_str: &str) -> Vec<Vec<(String, String)>> {
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars(query_str, &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let solver = Solver::new(&db, goals, vars);
    let (solutions, solver_interner) = solver.all_solutions_with_interner().unwrap();
    solutions
        .iter()
        .map(|sol| {
            sol.bindings
                .iter()
                .map(|(name, term)| (name.clone(), term_to_string(term, &solver_interner)))
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
    let result = first_binding(source, "findall(Name, (score(Name, S), S > 90), L)", "L");
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
    assert!(err.contains("step limit"));
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
    let (goals, vars) = Parser::parse_query_with_vars("X is 10 / 0", &mut interner).unwrap();
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

// ========================================================================
// Phase 3: Usability — once/1, call/1, atom predicates, arithmetic
// ========================================================================

#[test]
fn test_once_limits_to_first_solution() {
    let source = "color(red). color(green). color(blue).";
    let solutions = solve_all(source, "once(color(X))");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "red");
}

#[test]
fn test_once_in_rule() {
    let source = r#"
        n(1). n(2). n(3).
        first_n(X) :- once(n(X)).
    "#;
    let result = first_binding(source, "first_n(X)", "X");
    assert_eq!(result, Some("1".to_string()));
}

#[test]
fn test_call_meta_predicate() {
    let source = r#"
        color(red). color(blue). color(green).
        apply(Goal) :- call(Goal).
    "#;
    let solutions = solve_all(source, "apply(color(X))");
    assert_eq!(solutions.len(), 3);
}

#[test]
fn test_atom_length_in_rule() {
    let source = r#"
        long_name(X) :- atom_length(X, N), N > 5.
    "#;
    let solutions = solve_all(source, "long_name(elephant)");
    assert_eq!(solutions.len(), 1);

    let solutions = solve_all(source, "long_name(cat)");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_atom_concat_in_rule() {
    let source = r#"
        greet(Name, Greeting) :- atom_concat(hello, Name, Greeting).
    "#;
    let result = first_binding(source, "greet(world, G)", "G");
    assert_eq!(result, Some("helloworld".to_string()));
}

#[test]
fn test_atom_chars_pipeline() {
    let source = r#"
        starts_with(Atom, Char) :- atom_chars(Atom, [Char|_]).
    "#;
    let result = first_binding(source, "starts_with(hello, C)", "C");
    assert_eq!(result, Some("h".to_string()));
}

#[test]
fn test_arithmetic_abs() {
    let result = first_binding("", "X is abs(-42)", "X");
    assert_eq!(result, Some("42".to_string()));

    let result = first_binding("", "X is abs(42)", "X");
    assert_eq!(result, Some("42".to_string()));
}

#[test]
fn test_arithmetic_max_min() {
    let result = first_binding("", "X is max(10, 20)", "X");
    assert_eq!(result, Some("20".to_string()));

    let result = first_binding("", "X is min(10, 20)", "X");
    assert_eq!(result, Some("10".to_string()));
}

#[test]
fn test_arithmetic_sign() {
    let result = first_binding("", "X is sign(42)", "X");
    assert_eq!(result, Some("1".to_string()));

    let result = first_binding("", "X is sign(0)", "X");
    assert_eq!(result, Some("0".to_string()));

    let result = first_binding("", "X is sign(-7)", "X");
    assert_eq!(result, Some("-1".to_string()));
}

#[test]
fn test_arithmetic_combined() {
    // abs(min(3, -5)) should be 5
    let result = first_binding("", "X is abs(min(3, -5))", "X");
    assert_eq!(result, Some("5".to_string()));
}

// ========================================================================
// Phase 5: Nice-to-have — term ordering, introspection, between, etc.
// ========================================================================

#[test]
fn test_compare_atoms() {
    let result = first_binding("", "compare(Order, apple, banana)", "Order");
    assert_eq!(result, Some("<".to_string()));

    let result = first_binding("", "compare(Order, zebra, apple)", "Order");
    assert_eq!(result, Some(">".to_string()));

    let result = first_binding("", "compare(Order, same, same)", "Order");
    assert_eq!(result, Some("=".to_string()));
}

#[test]
fn test_compare_numbers() {
    let result = first_binding("", "compare(Order, 1, 2)", "Order");
    assert_eq!(result, Some("<".to_string()));

    let result = first_binding("", "compare(Order, 10, 3)", "Order");
    assert_eq!(result, Some(">".to_string()));
}

#[test]
fn test_term_ordering_operators() {
    let source = "";
    // Atoms compared alphabetically
    let solutions = solve_all(source, "apple @< banana");
    assert_eq!(solutions.len(), 1);

    let solutions = solve_all(source, "banana @< apple");
    assert_eq!(solutions.len(), 0);

    let solutions = solve_all(source, "zebra @> apple");
    assert_eq!(solutions.len(), 1);

    let solutions = solve_all(source, "foo @>= foo");
    assert_eq!(solutions.len(), 1);

    let solutions = solve_all(source, "foo @=< foo");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_term_ordering_numbers_before_atoms() {
    // Standard order: numbers < atoms
    let solutions = solve_all("", "1 @< hello");
    assert_eq!(solutions.len(), 1);

    let solutions = solve_all("", "hello @< 1");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_functor_decompose() {
    let result = first_binding("", "functor(foo(a, b, c), Name, Arity)", "Name");
    assert_eq!(result, Some("foo".to_string()));

    let result = first_binding("", "functor(foo(a, b, c), Name, Arity)", "Arity");
    assert_eq!(result, Some("3".to_string()));
}

#[test]
fn test_functor_atom() {
    let result = first_binding("", "functor(hello, Name, Arity)", "Name");
    assert_eq!(result, Some("hello".to_string()));

    let result = first_binding("", "functor(hello, Name, Arity)", "Arity");
    assert_eq!(result, Some("0".to_string()));
}

#[test]
fn test_functor_construct() {
    let result = first_binding("", "functor(T, foo, 2)", "T");
    // Constructed term has fresh variables with numeric IDs
    let val = result.unwrap();
    assert!(val.starts_with("foo("));
    assert!(val.contains(","));
}

#[test]
fn test_functor_number() {
    let result = first_binding("", "functor(42, Name, Arity)", "Name");
    assert_eq!(result, Some("42".to_string()));

    let result = first_binding("", "functor(42, Name, Arity)", "Arity");
    assert_eq!(result, Some("0".to_string()));
}

#[test]
fn test_arg_compound() {
    let result = first_binding("", "arg(1, foo(a, b, c), X)", "X");
    assert_eq!(result, Some("a".to_string()));

    let result = first_binding("", "arg(2, foo(a, b, c), X)", "X");
    assert_eq!(result, Some("b".to_string()));

    let result = first_binding("", "arg(3, foo(a, b, c), X)", "X");
    assert_eq!(result, Some("c".to_string()));
}

#[test]
fn test_arg_out_of_range() {
    let solutions = solve_all("", "arg(4, foo(a, b, c), X)");
    assert_eq!(solutions.len(), 0); // fails, not error
}

#[test]
fn test_univ_decompose() {
    let result = first_binding("", "foo(a, b) =.. L", "L");
    assert_eq!(result, Some("[foo, a, b]".to_string()));
}

#[test]
fn test_univ_atom() {
    let result = first_binding("", "hello =.. L", "L");
    assert_eq!(result, Some("[hello]".to_string()));
}

#[test]
fn test_univ_construct() {
    let result = first_binding("", "T =.. [bar, 1, 2]", "T");
    assert_eq!(result, Some("bar(1, 2)".to_string()));
}

#[test]
fn test_univ_number() {
    let result = first_binding("", "42 =.. L", "L");
    assert_eq!(result, Some("[42]".to_string()));
}

#[test]
fn test_between_basic() {
    let solutions = solve_all("", "between(1, 5, X)");
    assert_eq!(solutions.len(), 5);
    assert_eq!(solutions[0][0].1, "1");
    assert_eq!(solutions[4][0].1, "5");
}

#[test]
fn test_between_single() {
    let solutions = solve_all("", "between(3, 3, X)");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "3");
}

#[test]
fn test_between_empty() {
    let solutions = solve_all("", "between(5, 3, X)");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_between_in_rule() {
    let source = r#"
        digit(D) :- between(0, 9, D).
    "#;
    let solutions = solve_all(source, "digit(D)");
    assert_eq!(solutions.len(), 10);
}

#[test]
fn test_copy_term_basic() {
    let source = "";
    let result = first_binding(source, "copy_term(f(X, Y), Copy)", "Copy");
    // Copy should be f with fresh variables
    assert!(result.is_some());
    let val = result.unwrap();
    assert!(val.starts_with("f("));
}

#[test]
fn test_copy_term_ground() {
    let result = first_binding("", "copy_term(hello, Copy)", "Copy");
    assert_eq!(result, Some("hello".to_string()));

    let result = first_binding("", "copy_term(42, Copy)", "Copy");
    assert_eq!(result, Some("42".to_string()));
}

#[test]
fn test_succ_forward() {
    let result = first_binding("", "succ(3, X)", "X");
    assert_eq!(result, Some("4".to_string()));

    let result = first_binding("", "succ(0, X)", "X");
    assert_eq!(result, Some("1".to_string()));
}

#[test]
fn test_succ_backward() {
    let result = first_binding("", "succ(X, 5)", "X");
    assert_eq!(result, Some("4".to_string()));

    let result = first_binding("", "succ(X, 1)", "X");
    assert_eq!(result, Some("0".to_string()));
}

#[test]
fn test_plus_forward() {
    let result = first_binding("", "plus(3, 4, X)", "X");
    assert_eq!(result, Some("7".to_string()));
}

#[test]
fn test_plus_backward() {
    let result = first_binding("", "plus(3, Y, 10)", "Y");
    assert_eq!(result, Some("7".to_string()));

    let result = first_binding("", "plus(X, 4, 10)", "X");
    assert_eq!(result, Some("6".to_string()));
}

#[test]
fn test_msort_basic() {
    let result = first_binding("", "msort([c, a, b], X)", "X");
    assert_eq!(result, Some("[a, b, c]".to_string()));
}

#[test]
fn test_msort_preserves_duplicates() {
    let result = first_binding("", "msort([b, a, b, a], X)", "X");
    assert_eq!(result, Some("[a, a, b, b]".to_string()));
}

#[test]
fn test_msort_numbers() {
    let result = first_binding("", "msort([3, 1, 2], X)", "X");
    assert_eq!(result, Some("[1, 2, 3]".to_string()));
}

#[test]
fn test_sort_removes_duplicates() {
    let result = first_binding("", "sort([b, a, b, a], X)", "X");
    assert_eq!(result, Some("[a, b]".to_string()));
}

#[test]
fn test_sort_empty() {
    let result = first_binding("", "sort([], X)", "X");
    assert_eq!(result, Some("[]".to_string()));
}

#[test]
fn test_number_chars_integer() {
    let result = first_binding("", "number_chars(123, X)", "X");
    assert_eq!(result, Some("[1, 2, 3]".to_string()));

    // Verify elements are atoms (not integers) by checking atom/1 on first element
    let solutions = solve_all("", "number_chars(123, [H|_]), atom(H)");
    assert_eq!(
        solutions.len(),
        1,
        "number_chars should produce atom elements"
    );
}

#[test]
fn test_number_chars_reverse() {
    let result = first_binding("", "number_chars(X, ['4', '5', '6'])", "X");
    assert_eq!(result, Some("456".to_string()));
}

#[test]
fn test_number_codes_integer() {
    let result = first_binding("", "number_codes(65, X)", "X");
    // Character codes for '6' and '5': 54, 53
    assert!(result.is_some());
}

#[test]
fn test_number_codes_reverse() {
    // ASCII codes for '1', '2', '3' are 49, 50, 51
    let result = first_binding("", "number_codes(X, [49, 50, 51])", "X");
    assert_eq!(result, Some("123".to_string()));
}

#[test]
fn test_between_with_findall() {
    let result = first_binding("", "findall(X, between(1, 5, X), L)", "L");
    assert_eq!(result, Some("[1, 2, 3, 4, 5]".to_string()));
}

#[test]
fn test_sort_in_rule() {
    let source = r#"
        score(alice, 85).
        score(bob, 92).
        score(carol, 78).
        sorted_names(Sorted) :- findall(Name, score(Name, _), L), sort(L, Sorted).
    "#;
    let result = first_binding(source, "sorted_names(X)", "X");
    assert_eq!(result, Some("[alice, bob, carol]".to_string()));
}

#[test]
fn test_functor_in_rule() {
    let source = r#"
        arity_of(Term, A) :- functor(Term, _, A).
    "#;
    let result = first_binding(source, "arity_of(foo(a, b), A)", "A");
    assert_eq!(result, Some("2".to_string()));
}

#[test]
fn test_univ_in_rule() {
    // Test =.. decompose into head/tail pattern inside a rule
    let source = r#"
        get_functor(Term, F) :- Term =.. [F|_].
    "#;
    let result = first_binding(source, "get_functor(foo(a, b), F)", "F");
    assert_eq!(result, Some("foo".to_string()));
}

#[test]
fn test_univ_reconstruct_in_rule() {
    // Test =.. reconstruction from a list
    let source = r#"
        rebuild(Term, R) :-
            Term =.. L,
            R =.. L.
    "#;
    let result = first_binding(source, "rebuild(foo(a, b), R)", "R");
    assert_eq!(result, Some("foo(a, b)".to_string()));
}

// ========================================================================
// PR review regression tests
// ========================================================================

#[test]
fn test_negation_between_bound() {
    // \+ between(1,5,3) should fail (3 IS between 1 and 5)
    let solutions = solve_all("", "\\+ between(1, 5, 3)");
    assert_eq!(solutions.len(), 0); // negation should fail

    // \+ between(1,5,10) should succeed (10 is NOT between 1 and 5)
    let solutions = solve_all("", "\\+ between(1, 5, 10)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_compare_compound_with_bound_vars() {
    // compare/3 should deeply resolve variables inside compounds
    let source = r#"
        test(Order) :- X = 1, Y = 2, compare(Order, f(X), f(Y)).
    "#;
    let result = first_binding(source, "test(O)", "O");
    assert_eq!(result, Some("<".to_string()));
}

#[test]
fn test_term_ordering_compound_with_bound_vars() {
    let source = r#"
        test :- X = apple, Y = banana, f(X) @< f(Y).
    "#;
    let solutions = solve_all(source, "test");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_if_then_else_keeps_bindings() {
    // The condition bindings should be kept for the then branch
    let source = r#"
        classify(X, Result) :- (X > 0 -> Result = positive ; Result = non_positive).
    "#;
    let result = first_binding(source, "classify(5, R)", "R");
    assert_eq!(result, Some("positive".to_string()));

    let result = first_binding(source, "classify(-1, R)", "R");
    assert_eq!(result, Some("non_positive".to_string()));
}

#[test]
fn test_if_then_else_condition_bindings_propagate() {
    // Bindings from condition should be available in then branch
    let source = r#"
        test(X, R) :- (X = hello -> R = matched ; R = no_match).
    "#;
    let result = first_binding(source, "test(hello, R)", "R");
    assert_eq!(result, Some("matched".to_string()));

    let result = first_binding(source, "test(world, R)", "R");
    assert_eq!(result, Some("no_match".to_string()));
}

#[test]
fn test_atom_chars_reverse() {
    // atom_chars/2 reverse: char list -> atom
    let result = first_binding("", "atom_chars(X, [h, e, l, l, o])", "X");
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn test_atom_chars_roundtrip() {
    let source = r#"
        roundtrip(Atom, Result) :- atom_chars(Atom, Chars), atom_chars(Result, Chars).
    "#;
    let result = first_binding(source, "roundtrip(hello, R)", "R");
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn test_succ_overflow() {
    let source = "";
    let err = solve_expect_error(source, &format!("succ({}, X)", i64::MAX));
    assert!(err.contains("overflow"));
}

#[test]
fn test_plus_overflow() {
    let source = "";
    let err = solve_expect_error(source, &format!("plus({}, 1, X)", i64::MAX));
    assert!(err.contains("overflow"));
}

// ========================================================================
// PR Review Round 2 — Regression tests
// ========================================================================

#[test]
fn test_number_chars_reverse_in_once() {
    // Bug: number_chars/2 reverse mode was missing from try_exec_misc
    // once(number_chars(X, ['1','2','3'])) should bind X = 123
    let source = "";
    let val = first_binding(source, "once(number_chars(X, ['1','2','3']))", "X");
    assert_eq!(val.as_deref(), Some("123"));
}

#[test]
fn test_number_codes_reverse_in_once() {
    // number_codes/2 reverse mode was also missing from try_exec_misc
    let source = "";
    let val = first_binding(source, "once(number_codes(X, [52, 50]))", "X");
    assert_eq!(val.as_deref(), Some("42"));
}

#[test]
fn test_negation_number_chars_reverse() {
    // \+ number_chars(_, ['9']) should fail (because 9 is a valid number)
    let source = "";
    let solutions = solve_all(source, "\\+ number_chars(_, ['9'])");
    assert!(
        solutions.is_empty(),
        "\\+ number_chars(_, ['9']) should fail since 9 is valid"
    );
}

#[test]
fn test_iso_float_integer_ordering() {
    // ISO 8.4.2.1: float < integer when same arithmetic value
    // compare(Order, 1.0, 1) should give Order = <
    let source = "";
    let val = first_binding(source, "compare(Order, 1.0, 1)", "Order");
    assert_eq!(val.as_deref(), Some("<"));
}

#[test]
fn test_iso_integer_float_ordering() {
    // compare(Order, 1, 1.0) should give Order = >
    let source = "";
    let val = first_binding(source, "compare(Order, 1, 1.0)", "Order");
    assert_eq!(val.as_deref(), Some(">"));
}

#[test]
fn test_between_conjunction_in_negation() {
    // \+ (between(1, 5, X), X > 3) should fail because X=4 satisfies the conjunction
    let source = "";
    let solutions = solve_all(source, "\\+ (between(1, 5, X), X > 3)");
    assert!(
        solutions.is_empty(),
        "\\+ (between(1,5,X), X > 3) should fail because X=4 or X=5 satisfy the conjunction"
    );
}

#[test]
fn test_once_between_conjunction() {
    // once((between(1, 5, X), X > 3)) should bind X = 4
    let source = "";
    let val = first_binding(source, "once((between(1, 5, X), X > 3))", "X");
    assert_eq!(val.as_deref(), Some("4"));
}

// ========================================================================
// PR Review Round 3 — Regression tests
// ========================================================================

#[test]
fn test_functor_list_in_once() {
    // Bug: functor/3 in try_exec_misc was missing Term::List case
    let source = "";
    let val = first_binding(source, "once(functor([a,b], F, A)), F = '.', A = 2", "F");
    assert_eq!(val.as_deref(), Some("."));
}

#[test]
fn test_functor_construct_in_once() {
    // Bug: functor/3 in try_exec_misc was missing Term::Var construction case
    let source = "";
    let solutions = solve_all(source, "once(functor(T, foo, 2)), nonvar(T)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_univ_list_in_once() {
    // Bug: =../2 in try_exec_misc was missing Term::List case
    let source = "";
    let val = first_binding(source, "once([a,b] =.. L)", "L");
    // Should decompose to ['.', a, [b]]
    assert!(val.is_some(), "once([a,b] =.. L) should succeed");
}

#[test]
fn test_functor_arity_too_large() {
    // Bug: large arity caused silent wraparound or OOM
    let source = "";
    let err = solve_expect_error(source, "functor(T, f, 9999999)");
    assert!(err.contains("arity too large"));
}

#[test]
fn test_findall_functor_list() {
    // functor/3 with list inside findall should work
    let source = "";
    let val = first_binding(source, "findall(A, functor([1,2,3], _, A), As)", "As");
    assert_eq!(val.as_deref(), Some("[2]"));
}

// ========================================================================
// PR Review Round 4 — Regression tests
// ========================================================================

#[test]
fn test_findall_once_collects_one() {
    // Bug: once/1 inside findall was collecting ALL solutions instead of one
    let source = "color(red). color(green). color(blue).";
    let val = first_binding(source, "findall(X, once(color(X)), L)", "L");
    assert_eq!(val.as_deref(), Some("[red]"));
}

#[test]
fn test_findall_once_member() {
    // findall(X, once(member(X, [a,b,c])), L) should give [a] not [a,b,c]
    let source = include_str!("../knowledge/stdlib.pl");
    let val = first_binding(source, "findall(X, once(member(X, [a,b,c])), L)", "L");
    assert_eq!(val.as_deref(), Some("[a]"));
}

#[test]
fn test_between_overflow_at_max() {
    // between/3 with low = i64::MAX should not panic from unchecked low + 1
    let source = "";
    let val = first_binding(
        source,
        &format!("between({}, {}, X)", i64::MAX, i64::MAX),
        "X",
    );
    let expected = i64::MAX.to_string();
    assert_eq!(val.as_deref(), Some(expected.as_str()));
}

// ========================================================================
// Review Round 5 regression tests
// ========================================================================

#[test]
fn test_mod_floored_semantics() {
    // ISO Prolog mod uses floored division (rem_euclid), not truncated remainder
    // -7 mod 3 should be 2 (floored), not -1 (truncated)
    let result = first_binding("", "X is -7 mod 3", "X");
    assert_eq!(result, Some("2".to_string()));

    let result = first_binding("", "X is 7 mod -3", "X");
    assert_eq!(result, Some("-2".to_string()));
}

#[test]
fn test_float_formatting_in_number_chars() {
    // Float 1.0 should format as "1.0" not "1"
    let result = first_binding("", "number_chars(1.0, X)", "X");
    assert_eq!(result, Some("[1, ., 0]".to_string()));
}

#[test]
fn test_float_formatting_in_number_codes() {
    // Float 2.0 should format as "2.0" not "2"
    let result = first_binding("", "number_codes(2.0, X)", "X");
    // Character codes for '2', '.', '0': 50, 46, 48
    assert_eq!(result, Some("[50, 46, 48]".to_string()));
}

#[test]
fn test_univ_type_error_non_atom_functor() {
    // =../2 should error when constructing with non-atom functor and arity > 0
    let err = solve_expect_error("", "X =.. [3, a, b]");
    assert!(err.contains("=../2"));
}

#[test]
fn test_functor_negative_arity() {
    // functor/3 with negative arity should give a clear error
    let err = solve_expect_error("", "functor(X, foo, -1)");
    assert!(err.contains("non-negative"));
}

#[test]
fn test_step_limit_in_try_solve_once() {
    // Step limit should be enforced in try_solve_once (via \+)
    // An infinite loop inside \+ should not hang forever
    let source = "loop :- loop.";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars("\\+ loop", &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(100);
    // Should terminate (not hang) — either success (because \+ of a non-terminating
    // goal that hits step limit is treated as failure of the inner goal, so \+ succeeds)
    // or failure or error
    let result = solver.next();
    // The important thing is it terminates; \+ returns success because inner goal fails
    // due to step limit exhaustion
    assert!(matches!(
        result,
        SolveResult::Success(_) | SolveResult::Error(_)
    ));
}

// ========================================================================
// Review Round 6 regression tests
// ========================================================================

#[test]
fn test_succ_zero_fails() {
    // succ(X, 0) should fail (no non-negative predecessor of 0), not error
    let solutions = solve_all("", "succ(X, 0)");
    assert!(solutions.is_empty());
}

#[test]
fn test_number_chars_rejects_bad_elements() {
    // number_chars(X, ['1', bad_atom, '2']) should error, not silently skip bad_atom
    let err = solve_expect_error("", "number_chars(X, ['1', bad_atom, '2'])");
    assert!(err.contains("single-character"));
}

#[test]
fn test_number_codes_rejects_bad_elements() {
    // number_codes(X, [49, foo, 50]) should error (foo is not an integer)
    let err = solve_expect_error("", "number_codes(X, [49, foo, 50])");
    assert!(err.contains("character codes"));
}

#[test]
fn test_copy_term_aliasing() {
    // copy_term(f(X, X), Y): two occurrences of X should map to the same fresh variable
    let result = first_binding("", "copy_term(f(X, X), f(A, B)), A = hello", "B");
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn test_between_low_greater_than_high() {
    // between(5, 3, X) should fail
    let solutions = solve_all("", "between(5, 3, X)");
    assert!(solutions.is_empty());
}

// ========================================================================
// Review Round 7 regression tests
// ========================================================================

#[test]
fn test_mod_large_negative_divisor() {
    // Negative divisor should work correctly (ISO: result has sign of divisor)
    let result = first_binding("", "X is 5 mod -3", "X");
    assert_eq!(result, Some("-1".to_string()));
}

#[test]
fn test_atom_chars_rejects_multi_char_atom() {
    // atom_chars(X, [a, bc, d]) should fail (bc is not a single character)
    let solutions = solve_all("", "atom_chars(X, [a, bc, d])");
    assert!(solutions.is_empty());
}

#[test]
fn test_atom_chars_rejects_non_atom_element() {
    // atom_chars(X, [a, 1, b]) should fail (1 is an integer, not an atom)
    let solutions = solve_all("", "atom_chars(X, [a, 1, b])");
    assert!(solutions.is_empty());
}

// ========================================================================
// Review Round 8 regression tests
// ========================================================================

#[test]
fn test_between_bound_x_large_range_naf() {
    // between/3 with bound X under \+ should be O(1), not iterate the full range
    let solutions = solve_all("", "X = 50, \\+ between(1, 1000000, X)");
    // X=50 is in range, so between succeeds, \+ fails => no solutions
    assert!(solutions.is_empty());
}

#[test]
fn test_between_bound_x_large_range_findall() {
    // between/3 with bound X in findall context should be O(1)
    let l_val = first_binding("", "findall(X, (X = 42, between(1, 1000000, X)), L)", "L");
    assert_eq!(l_val, Some("[42]".to_string()));
}

#[test]
fn test_between_unbound_x_naf() {
    // between/3 with unbound X under \+ should still work (step limit protects)
    let solutions = solve_all("", "\\+ between(1, 5, X)");
    // between(1,5,X) succeeds with X=1, so \+ fails
    assert!(solutions.is_empty());
}

#[test]
fn test_copy_term_list() {
    // copy_term with a list should produce fresh variables
    let solutions = solve_all("", "copy_term([A, B, C], [1, 2, 3])");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_copy_term_nested_list() {
    // copy_term preserves list structure
    let result = first_binding("", "copy_term([a, b, c], X)", "X");
    assert_eq!(result, Some("[a, b, c]".to_string()));
}

// ========================================================================
// Review Round 9 regression tests
// ========================================================================

#[test]
fn test_no_occurs_check_unify() {
    // ISO: X = f(X) should succeed (=/2 does not occurs-check)
    let solutions = solve_all("", "X = f(X)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_number_chars_integer_elements_error() {
    // number_chars(X, [1, 2, 3]) with integer elements should error (not silently fail)
    let err = solve_expect_error("", "number_chars(X, [1, 2, 3])");
    assert!(err.contains("single-character"));
}

#[test]
fn test_number_codes_atom_elements_error() {
    // number_codes(X, [a, b, c]) with atom elements should error
    let err = solve_expect_error("", "number_codes(X, [a, b, c])");
    assert!(err.contains("character codes"));
}

#[test]
fn test_copy_term_preserves_long_list() {
    // copy_term with a longer list to exercise iterative spine traversal
    let result = first_binding("", "copy_term([1,2,3,4,5,6,7,8,9,10], X)", "X");
    assert_eq!(result, Some("[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]".to_string()));
}

#[test]
fn test_cut_prevents_all_alternatives() {
    // Verify cut actually prevents ALL alternatives, not just first-solution check
    let src = "foo(a). foo(b). foo(c).";

    // foo(X), ! should give exactly 1 solution (cut prevents trying b,c)
    let solutions = solve_all(src, "foo(X), !");
    assert_eq!(solutions.len(), 1, "cut should prevent foo alternatives");
    assert_eq!(first_binding(src, "foo(X), !", "X"), Some("a".to_string()));

    // foo(X), !, X = b should fail (X=a from first clause, cut prevents alternatives)
    let solutions = solve_all(src, "foo(X), !, X = b");
    assert_eq!(
        solutions.len(),
        0,
        "cut should prevent trying X=b alternative"
    );
}

#[test]
fn test_cut_in_negation() {
    let src = "foo(a). foo(b). foo(c).";

    // \+((foo(X), !, X = b)) — inner fails (cut prevents X=b), so \+ succeeds
    let solutions = solve_all(src, "\\+((foo(X), !, X = b))");
    assert_eq!(
        solutions.len(),
        1,
        "\\+ should succeed when inner fails due to cut"
    );
}

#[test]
fn test_cut_in_once() {
    let src = "foo(a). foo(b). foo(c).";

    // once((foo(X), !, X = b)) — cut prevents alternatives, X=a fails X=b, overall fails
    let solutions = solve_all(src, "once((foo(X), !, X = b))");
    assert_eq!(solutions.len(), 0, "once with cut should fail");
}

#[test]
fn test_float_div_by_int_zero() {
    // 1.0 / 0 should report "Division by zero", not "Infinity"
    let err = solve_expect_error("", "X is 1.0 / 0");
    assert!(err.contains("Division by zero"), "got: {}", err);
}

// ---- Round 11 regression tests ----

#[test]
fn test_between_step_limit_in_findall() {
    // between/3 inside findall with a huge range should be stopped by step limit
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) =
        Parser::parse_query_with_vars("findall(X, between(1, 1000000000, X), L)", &mut interner)
            .unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(100);
    // Should terminate (not loop 10^9 times); step limit caps iteration
    let result = solver.next();
    // The result should be success (findall collects what it can before step limit)
    // or failure, but importantly it terminates quickly
    match result {
        SolveResult::Success(_) | SolveResult::Failure | SolveResult::Error(_) => {} // all acceptable
    }
}

#[test]
fn test_between_step_limit_in_negation() {
    // between/3 inside \+ with a huge range should be stopped by step limit
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    // \+ between(1, 1000000000, X) -- X is unbound, first val succeeds, NAF fails
    // But the step limit should prevent runaway if semantics change
    let (goals, vars) = Parser::parse_query_with_vars(
        "\\+ (between(1, 1000000000, X), X > 1000000000)",
        &mut interner,
    )
    .unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(200);
    let result = solver.next();
    // Should terminate quickly due to step limit
    match result {
        SolveResult::Success(_) | SolveResult::Failure | SolveResult::Error(_) => {}
    }
}

#[test]
fn test_univ_unbound_functor_errors() {
    // T =.. [F] where F is unbound should error (ISO instantiation_error)
    let err = solve_expect_error("", "T =.. [F]");
    assert!(
        err.contains("instantiation") || err.contains("must be bound"),
        "got: {}",
        err
    );
}

#[test]
fn test_univ_number_construct_still_works() {
    // T =.. [42] should still succeed with T = 42
    let solutions = solve_all("", "T =.. [42]");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "42");
}

#[test]
fn test_number_chars_invalid_syntax_errors() {
    // number_chars(X, [a,b,c]) should be a syntax error, not silent failure
    let err = solve_expect_error("", "number_chars(X, [a,b,c])");
    assert!(
        err.contains("invalid number syntax") || err.contains("syntax"),
        "got: {}",
        err
    );
}

#[test]
fn test_number_codes_invalid_syntax_errors() {
    // number_codes(X, [97,98,99]) -> "abc" is not a valid number -> syntax error
    let err = solve_expect_error("", "number_codes(X, [97,98,99])");
    assert!(
        err.contains("invalid number syntax") || err.contains("syntax"),
        "got: {}",
        err
    );
}

#[test]
fn test_number_chars_valid_still_works() {
    // number_chars(X, ['1','2','3']) should still parse as 123
    let solutions = solve_all("", "number_chars(X, ['1','2','3'])");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "123");
}

#[test]
fn test_number_chars_unify_failure_backtracks() {
    // number_chars(42, ['1','2','3']) -> parses to 123, unify with 42 fails -> backtrack (not error)
    let solutions = solve_all("", "number_chars(42, ['1','2','3'])");
    assert_eq!(solutions.len(), 0);
}

// ---- Round 12 regression tests ----

#[test]
fn test_cut_in_findall_stops_clause_iteration() {
    // findall(X, (p(X), !), Xs) should return [1], not [1,2,3]
    let source = "p(1). p(2). p(3).";
    let result = first_binding(source, "findall(X, (p(X), !), Xs)", "Xs");
    assert_eq!(result, Some("[1]".to_string()));
}

#[test]
fn test_findall_without_cut_collects_all() {
    // Without cut, findall should still collect all solutions
    let source = "p(1). p(2). p(3).";
    let result = first_binding(source, "findall(X, p(X), Xs)", "Xs");
    assert_eq!(result, Some("[1, 2, 3]".to_string()));
}

#[test]
fn test_findall_step_limit_returns_error() {
    // findall with huge between should error, not return partial results
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) =
        Parser::parse_query_with_vars("findall(X, between(1, 100000, X), L)", &mut interner)
            .unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(100);
    match solver.next() {
        SolveResult::Error(e) => assert!(e.contains("step limit"), "got: {}", e),
        other => panic!("Expected error, got {:?}", other),
    }
}

#[test]
fn test_is_list_with_bound_tail() {
    // is_list should work when tail variable is bound to []
    let source = "";
    let solutions = solve_all(source, "X = [1,2,3], is_list(X)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_is_list_with_constructed_list() {
    // is_list on a list built via append
    let source = "my_list([1,2,3]).";
    let solutions = solve_all(source, "my_list(X), is_list(X)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_naf_parses_with_operator() {
    // \+ has ISO precedence 900fy — should parse \+ X = Y as \+(X = Y)
    let source = "";
    let solutions = solve_all(source, "X = hello, \\+ X = goodbye");
    assert_eq!(solutions.len(), 1);
    assert_eq!(solutions[0][0].1, "hello");
}

#[test]
fn test_naf_precedence_with_is() {
    // \+ X is 2+3 should parse as \+(X is 2+3), not (\+ X) is 2+3
    let source = "";
    let solutions = solve_all(source, "\\+ 1 =:= 2");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_float_overflow_literal_rejected() {
    // A float literal that overflows f64 should be a parse error
    let mut interner = StringInterner::new();
    let huge = format!("X is {}0.0", "9".repeat(310));
    let result = Parser::parse_query_with_vars(&huge, &mut interner);
    assert!(
        result.is_err(),
        "Expected parse error for overflowing float literal"
    );
}

#[test]
fn test_index_ground_miss_returns_empty() {
    // Querying with a ground first arg that doesn't match any clause should fail immediately
    let source = "color(red). color(blue).";
    let solutions = solve_all(source, "color(purple)");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_number_chars_rejects_nan() {
    // number_chars(X, ['N','a','N']) should error, not bind X to NaN
    let err = solve_expect_error("", "number_chars(X, ['N','a','N'])");
    assert!(
        err.contains("invalid number syntax") || err.contains("syntax"),
        "got: {}",
        err
    );
}

#[test]
fn test_number_chars_rejects_infinity() {
    // number_chars(X, [i,n,f]) should error, not bind X to Infinity
    let err = solve_expect_error("", "number_chars(X, [i,n,f])");
    assert!(
        err.contains("invalid number syntax") || err.contains("syntax"),
        "got: {}",
        err
    );
}

#[test]
fn test_cut_in_try_solve_no_leak_after_once() {
    // once(!) followed by predicate iteration should not be affected by dirty cut flag
    let source = "q(a). q(b). q(c).";
    let solutions = solve_all(source, "once(!), q(X)");
    assert_eq!(solutions.len(), 3); // Should find all q solutions
}

// ---- Round 14 regression tests ----

#[test]
fn test_naf_step_limit_returns_error_not_success() {
    // \+(loop) should error when step limit fires, not succeed
    let source = "loop :- loop.";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars("\\+(loop)", &mut interner).unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(50);
    match solver.next() {
        SolveResult::Error(e) => assert!(
            e.contains("step limit") || e.contains("exceeded"),
            "got: {}",
            e
        ),
        SolveResult::Success(_) => {
            panic!("Expected error, got success (NAF incorrectly succeeded)")
        }
        SolveResult::Failure => panic!("Expected error, got failure"),
    }
}

#[test]
fn test_between_bound_x_fast_path() {
    // between(1, 1000000, 5) should succeed in O(1), not O(N)
    let solutions = solve_all("", "between(1, 1000000, 5)");
    assert_eq!(solutions.len(), 1);
}

#[test]
fn test_between_bound_x_out_of_range() {
    // between(1, 10, 11) should fail
    let solutions = solve_all("", "between(1, 10, 11)");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_between_bound_x_below_range() {
    // between(5, 10, 3) should fail
    let solutions = solve_all("", "between(5, 10, 3)");
    assert_eq!(solutions.len(), 0);
}

#[test]
fn test_findall_steps_accumulate_globally() {
    // Nested findalls should share a single step budget, not get independent budgets
    // With max_depth=200, two findalls of 150 items each should exceed the limit
    let source = "";
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(source, &mut interner).unwrap();
    let (goals, vars) = Parser::parse_query_with_vars(
        "findall(X, between(1, 150, X), L1), findall(Y, between(1, 150, Y), L2)",
        &mut interner,
    )
    .unwrap();
    let db = CompiledDatabase::new(interner, clauses);
    let mut solver = Solver::new(&db, goals, vars).with_max_depth(200);
    // First findall uses ~150 steps. Second should hit the limit since only ~50 remain.
    match solver.next() {
        SolveResult::Error(e) => assert!(
            e.contains("step limit") || e.contains("exceeded"),
            "got: {}",
            e
        ),
        SolveResult::Success(_) => {
            panic!("Expected error — two findalls of 150 should exceed max_depth=200")
        }
        SolveResult::Failure => panic!("Expected error, got failure"),
    }
}
