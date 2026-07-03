use super::*;
use crate::parser::Parser;
use plg_shared::StringInterner;

fn parse_term(input: &str) -> (Term, StringInterner) {
    let mut interner = StringInterner::new();
    let goals = Parser::parse_query(input, &mut interner).unwrap();
    assert_eq!(goals.len(), 1);
    (goals.into_iter().next().unwrap(), interner)
}

fn parse_clauses(input: &str) -> (Vec<Clause>, StringInterner) {
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(input, &mut interner).unwrap();
    (clauses, interner)
}

#[test]
fn test_parse_atom() {
    let (term, interner) = parse_term("hello");
    match term {
        Term::Atom(id) => assert_eq!(interner.resolve(id), "hello"),
        _ => panic!("Expected atom"),
    }
}

#[test]
fn test_parse_integer() {
    let (term, _) = parse_term("42");
    assert_eq!(term, Term::Integer(42));
}

#[test]
fn test_parse_float() {
    let (term, _) = parse_term("2.5");
    assert_eq!(term, Term::Float(2.5));
}

#[test]
fn test_parse_variable() {
    let (term, _) = parse_term("X");
    match term {
        Term::Var(_) => {}
        _ => panic!("Expected variable"),
    }
}

#[test]
fn test_parse_compound() {
    let (term, interner) = parse_term("parent(tom, mary)");
    match term {
        Term::Compound { functor, args } => {
            assert_eq!(interner.resolve(functor), "parent");
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_parse_nested_compound() {
    let (term, interner) = parse_term("outer(inner(deep(hello)))");
    match term {
        Term::Compound { functor, ref args } => {
            assert_eq!(interner.resolve(functor), "outer");
            match &args[0] {
                Term::Compound { functor, args } => {
                    assert_eq!(interner.resolve(*functor), "inner");
                    match &args[0] {
                        Term::Compound { functor, args } => {
                            assert_eq!(interner.resolve(*functor), "deep");
                            match &args[0] {
                                Term::Atom(id) => assert_eq!(interner.resolve(*id), "hello"),
                                _ => panic!("Expected atom"),
                            }
                        }
                        _ => panic!("Expected compound"),
                    }
                }
                _ => panic!("Expected compound"),
            }
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_parse_fact() {
    let (clauses, interner) = parse_clauses("likes(mary, food).");
    assert_eq!(clauses.len(), 1);
    assert!(clauses[0].body.is_empty());
    match &clauses[0].head {
        Term::Compound { functor, args } => {
            assert_eq!(interner.resolve(*functor), "likes");
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_parse_rule() {
    let (clauses, interner) = parse_clauses("happy(X) :- likes(X, food).");
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].body.len(), 1);
    match &clauses[0].head {
        Term::Compound { functor, .. } => {
            assert_eq!(interner.resolve(*functor), "happy");
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_variable_scoping() {
    // Same variable name within a clause should get same id
    let (clauses, _) = parse_clauses("foo(X, Y) :- bar(X, Y).");
    let clause = &clauses[0];
    // Extract var ids from head
    if let Term::Compound {
        args: head_args, ..
    } = &clause.head
        && let (Term::Var(hx), Term::Var(hy)) = (&head_args[0], &head_args[1])
        && let Term::Compound {
            args: body_args, ..
        } = &clause.body[0]
        && let (Term::Var(bx), Term::Var(by)) = (&body_args[0], &body_args[1])
    {
        // Same vars in head and body
        assert_eq!(hx, bx, "X in head and body should be same var");
        assert_eq!(hy, by, "Y in head and body should be same var");
        assert_ne!(hx, hy, "X and Y should be different vars");
    }
}

#[test]
fn test_operator_precedence() {
    // 2 + 3 * 4 should parse as 2 + (3 * 4)
    let (term, interner) = parse_term("2 + 3 * 4");
    match term {
        Term::Compound { functor, ref args } => {
            assert_eq!(interner.resolve(functor), "+");
            assert_eq!(args[0], Term::Integer(2));
            match &args[1] {
                Term::Compound { functor, args } => {
                    assert_eq!(interner.resolve(*functor), "*");
                    assert_eq!(args[0], Term::Integer(3));
                    assert_eq!(args[1], Term::Integer(4));
                }
                _ => panic!("Expected compound for 3*4"),
            }
        }
        _ => panic!("Expected compound for addition"),
    }
}

#[test]
fn test_parenthesized_expr() {
    // (2 + 3) * 4 should parse as (2 + 3) * 4
    let (term, interner) = parse_term("(2 + 3) * 4");
    match term {
        Term::Compound { functor, ref args } => {
            assert_eq!(interner.resolve(functor), "*");
            match &args[0] {
                Term::Compound { functor, args } => {
                    assert_eq!(interner.resolve(*functor), "+");
                    assert_eq!(args[0], Term::Integer(2));
                    assert_eq!(args[1], Term::Integer(3));
                }
                _ => panic!("Expected compound for addition"),
            }
            assert_eq!(args[1], Term::Integer(4));
        }
        _ => panic!("Expected compound for multiplication"),
    }
}

#[test]
fn test_is_expression() {
    let (term, interner) = parse_term("X is 2 + 3");
    match term {
        Term::Compound { functor, args } => {
            assert_eq!(interner.resolve(functor), "is");
            assert!(matches!(args[0], Term::Var(_)));
            match &args[1] {
                Term::Compound { functor, .. } => {
                    assert_eq!(interner.resolve(*functor), "+");
                }
                _ => panic!("Expected compound"),
            }
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_unary_minus() {
    let (term, _) = parse_term("- 5");
    assert_eq!(term, Term::Integer(-5));
}

#[test]
fn test_empty_list() {
    let (term, interner) = parse_term("[]");
    match term {
        Term::Atom(id) => assert_eq!(interner.resolve(id), "[]"),
        _ => panic!("Expected empty list atom"),
    }
}

#[test]
fn test_simple_list() {
    let (term, interner) = parse_term("[1, 2, 3]");
    // Should be List(1, List(2, List(3, Atom([]))))
    match term {
        Term::List { head, tail } => {
            assert_eq!(*head, Term::Integer(1));
            match tail.as_ref() {
                Term::List { head, tail } => {
                    assert_eq!(**head, Term::Integer(2));
                    match tail.as_ref() {
                        Term::List { head, tail } => {
                            assert_eq!(**head, Term::Integer(3));
                            match tail.as_ref() {
                                Term::Atom(id) => assert_eq!(interner.resolve(*id), "[]"),
                                _ => panic!("Expected nil"),
                            }
                        }
                        _ => panic!("Expected list"),
                    }
                }
                _ => panic!("Expected list"),
            }
        }
        _ => panic!("Expected list, got {term:?}"),
    }
}

#[test]
fn test_head_tail_list() {
    let (term, _) = parse_term("[H | T]");
    match term {
        Term::List { head, tail } => {
            assert!(matches!(*head, Term::Var(_)));
            assert!(matches!(*tail, Term::Var(_)));
        }
        _ => panic!("Expected list"),
    }
}

#[test]
fn test_multiple_clauses() {
    let (clauses, _) = parse_clauses("a. b. c.");
    assert_eq!(clauses.len(), 3);
}

#[test]
fn test_parse_error() {
    let mut interner = StringInterner::new();
    let result = Parser::parse_program("invalid(((.", &mut interner);
    assert!(result.is_err());
}

#[test]
fn test_comparison_operators() {
    let (term, interner) = parse_term("X > 100");
    match term {
        Term::Compound { functor, .. } => {
            assert_eq!(interner.resolve(functor), ">");
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_cut() {
    let (clauses, interner) = parse_clauses("max(X, Y, X) :- X >= Y, !.");
    // Body is a single conjunction: ','(X >= Y, !)
    assert_eq!(clauses[0].body.len(), 1);
    match &clauses[0].body[0] {
        Term::Compound { functor, args } => {
            assert_eq!(interner.resolve(*functor), ",");
            assert_eq!(args.len(), 2);
            match &args[1] {
                Term::Atom(id) => assert_eq!(interner.resolve(*id), "!"),
                _ => panic!("Expected cut atom"),
            }
        }
        _ => panic!("Expected conjunction"),
    }
}

#[test]
fn test_negation() {
    let (term, interner) = parse_term("\\+ foo(X)");
    match term {
        Term::Compound { functor, args } => {
            assert_eq!(interner.resolve(functor), "\\+");
            assert_eq!(args.len(), 1);
        }
        _ => panic!("Expected compound"),
    }
}

#[test]
fn test_dynamic_directive_single() {
    let mut interner = StringInterner::new();
    let (clauses, dirs) =
        Parser::parse_program_with_directives(":- dynamic(field/1). field(name).", &mut interner)
            .unwrap();
    assert_eq!(clauses.len(), 1, "directive should not be a clause");
    assert_eq!(dirs.dynamic.len(), 1);
    let (f, arity) = dirs.dynamic[0];
    assert_eq!(interner.resolve(f), "field");
    assert_eq!(arity, 1);
}

#[test]
fn test_dynamic_directive_multiple_args() {
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- dynamic(f/1, g/2, h/0).", &mut interner).unwrap();
    assert_eq!(dirs.dynamic.len(), 3);
    assert_eq!(interner.resolve(dirs.dynamic[0].0), "f");
    assert_eq!(dirs.dynamic[0].1, 1);
    assert_eq!(interner.resolve(dirs.dynamic[2].0), "h");
    assert_eq!(dirs.dynamic[2].1, 0);
}

#[test]
fn test_dynamic_directive_comma_chain_arg() {
    // :- dynamic((f/1, g/2)). — comma chain inside a single paren group
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- dynamic((f/1, g/2)).", &mut interner).unwrap();
    assert_eq!(dirs.dynamic.len(), 2);
}

#[test]
fn test_parse_program_ignores_directives() {
    // Back-compat path: parse_program drops directives.
    let mut interner = StringInterner::new();
    let clauses = Parser::parse_program(":- dynamic(field/1). fact(a).", &mut interner).unwrap();
    assert_eq!(clauses.len(), 1);
}

#[test]
fn test_io_format_directive_list() {
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- io_format([json, bson]). f(a).", &mut interner)
            .unwrap();
    assert_eq!(dirs.io_format, vec!["json".to_string(), "bson".to_string()]);
}

#[test]
fn test_io_format_directive_comma_chain() {
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- io_format((json, bson)).", &mut interner)
            .unwrap();
    assert_eq!(dirs.io_format, vec!["json".to_string(), "bson".to_string()]);
}

#[test]
fn test_io_format_directive_single_atom() {
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- io_format(bson).", &mut interner).unwrap();
    assert_eq!(dirs.io_format, vec!["bson".to_string()]);
}

#[test]
fn test_io_format_directive_dedups() {
    let mut interner = StringInterner::new();
    let (_clauses, dirs) =
        Parser::parse_program_with_directives(":- io_format([json, json, bson]).", &mut interner)
            .unwrap();
    assert_eq!(dirs.io_format, vec!["json".to_string(), "bson".to_string()]);
}

#[test]
fn test_unknown_directive_errors() {
    let mut interner = StringInterner::new();
    let result = Parser::parse_program_with_directives(":- unknown_thing(foo).", &mut interner);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.message.contains("Unknown directive"), "got: {err}");
}

#[test]
fn parse_error_span_covers_unexpected_token() {
    let mut interner = StringInterner::new();
    // `]` sits where a term is expected: "go :- bar(" is 10 bytes, `]` at 10.
    let err = Parser::parse_program_with_directives("go :- bar(]).\n", &mut interner).unwrap_err();
    assert_eq!((err.span.lo, err.span.hi), (10, 11), "got: {err}");
    assert!(err.message.contains(']'), "got: {err}");
}

#[test]
fn parse_error_span_points_at_end_of_input() {
    let mut interner = StringInterner::new();
    // Missing clause terminator: parser hits the Eof token at offset 3.
    let err = Parser::parse_program_with_directives("foo", &mut interner).unwrap_err();
    assert_eq!(err.span.lo, 3, "got: {err}");
}

#[test]
fn test_dynamic_directive_bad_arity_errors() {
    let mut interner = StringInterner::new();
    let result = Parser::parse_program_with_directives(":- dynamic(field/(-1)).", &mut interner);
    assert!(result.is_err());
}
