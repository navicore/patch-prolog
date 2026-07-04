//! Term introspection (functor/3, arg/3, =../2, copy_term in once),
//! atom predicates (atom_length, atom_concat, atom_chars), number_chars /
//! number_codes (both directions + error terms), term ordering (@<, @>,
//! compare/3) and structural identity (==, \==).
//!
//! Fresh-var ids normalized via `norm()`. Value assertions in text; "exactly
//! one solution" via bson count. PROG advertises both.

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

fn prog() -> &'static Compiled {
    static C: OnceLock<Compiled> = OnceLock::new();
    C.get_or_init(|| {
        compile(
            ":- io_format([text, bson]).\n\
             arity_of(Term, A) :- functor(Term, _, A).\n\
             get_functor(Term, F) :- Term =.. [F|_].\n\
             rebuild(Term, R) :- Term =.. L, R =.. L.\n\
             long_name(X) :- atom_length(X, N), N > 5.\n\
             greet(Name, Greeting) :- atom_concat(hello, Name, Greeting).\n\
             starts_with(Atom, Char) :- atom_chars(Atom, [Char|_]).\n\
             roundtrip(Atom, Result) :- atom_chars(Atom, Cs), atom_chars(Result, Cs).\n",
        )
    })
}

#[track_caller]
fn check(goal: &str, expected_out: &str, expected_code: i32) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(
        norm(&out),
        norm(&format!("{expected_out}\n")),
        "goal: {goal}"
    );
    assert_eq!(code, expected_code, "goal: {goal}");
}

#[track_caller]
fn succeeds_once(goal: &str) {
    let (env, code) = prog().query_bson(goal, &[]);
    assert_eq!(env.count, Some(1), "goal {goal}: {env:?}");
    assert_eq!(code, 1, "goal: {goal}");
}

#[track_caller]
fn fails(goal: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert_eq!(out, "false.\n", "goal: {goal}");
    assert_eq!(code, 0, "goal: {goal}");
}

#[track_caller]
fn err_contains(goal: &str, needle: &str) {
    let (out, code) = prog().query(goal, &[]);
    assert!(out.contains(needle), "goal {goal}: {out}");
    assert_eq!(code, 3, "goal: {goal}");
}

// ---- functor / arg ---------------------------------------------------

#[test]
fn functor_decompose_and_construct() {
    check(
        "functor(foo(a, b, c), Name, Arity)",
        "Arity = 3\nName = foo",
        1,
    );
    check("functor(hello, Name, Arity)", "Arity = 0\nName = hello", 1);
    check("functor(42, Name, Arity)", "Arity = 0\nName = 42", 1);
    check("functor(T, foo, 2)", "T = foo(_V, _V)", 1);
    check("arity_of(foo(a, b), A)", "A = 2", 1);
    // functor/3 on a list: functor name is '.', arity 2.
    check("functor([a,b], F, A)", "A = 2\nF = .", 1);
}

#[test]
fn functor_errors() {
    err_contains("functor(T, f, 9999999)", "arity too large");
    err_contains("functor(X, foo, -1)", "non-negative");
}

#[test]
fn arg_in_range_and_out() {
    check("arg(1, foo(a, b, c), X)", "X = a", 1);
    check("arg(2, foo(a, b, c), X)", "X = b", 1);
    fails("arg(4, foo(a, b, c), X)");
}

// ---- univ (=..) ------------------------------------------------------

#[test]
fn univ_decompose_and_construct() {
    check("foo(a, b) =.. L", "L = [foo, a, b]", 1);
    check("hello =.. L", "L = [hello]", 1);
    check("T =.. [bar, 1, 2]", "T = bar(1, 2)", 1);
    check("42 =.. L", "L = [42]", 1);
    check("get_functor(foo(a, b), F)", "F = foo", 1);
    check("rebuild(foo(a, b), R)", "R = foo(a, b)", 1);
}

#[test]
fn univ_number_construct_and_errors() {
    check("T =.. [42]", "T = 42", 1);
    err_contains("T =.. [F]", "instantiation");
    err_contains("X =.. [3, a, b]", "type_error(atom");
}

#[test]
fn univ_list_in_once() {
    succeeds_once("once([a,b] =.. L)");
    check(
        "findall(A, functor([1,2,3], _, A), As)",
        "A = _V\nAs = [2]",
        1,
    );
}

// ---- atom predicates -------------------------------------------------

#[test]
fn atom_predicates() {
    succeeds_once("long_name(elephant)");
    fails("long_name(cat)");
    check("greet(world, G)", "G = helloworld", 1);
    check("starts_with(hello, C)", "C = h", 1);
    check("atom_chars(X, [h, e, l, l, o])", "X = hello", 1);
    check("roundtrip(hello, R)", "R = hello", 1);
}

#[test]
fn atom_chars_rejections() {
    fails("atom_chars(X, [a, bc, d])");
    fails("atom_chars(X, [a, 1, b])");
}

// ---- numberChars / numberCodes ---------------------------------------

#[test]
fn number_chars_and_codes() {
    check("number_chars(123, X)", "X = [1, 2, 3]", 1);
    check("number_chars(X, ['4', '5', '6'])", "X = 456", 1);
    check("number_codes(65, X)", "X = [54, 53]", 1);
    check("number_codes(X, [49, 50, 51])", "X = 123", 1);
    check("number_chars(1.0, X)", "X = [1, ., 0]", 1);
    check("number_codes(2.0, X)", "X = [50, 46, 48]", 1);
}

#[test]
fn number_chars_codes_in_once_and_atomicity() {
    succeeds_once("once(number_chars(X, ['1','2','3']))");
    succeeds_once("once(number_codes(X, [52, 50]))");
    succeeds_once("number_chars(X, ['1','2','3'])");
    fails("number_chars(42, ['1','2','3'])");
    succeeds_once("number_chars(123, [H|_]), atom(H)");
}

#[test]
fn number_chars_codes_error_terms() {
    err_contains("number_chars(X, ['1', bad_atom, '2'])", "single-character");
    err_contains("number_codes(X, [49, foo, 50])", "character codes");
    err_contains("number_chars(X, [1, 2, 3])", "single-character");
    err_contains("number_codes(X, [a, b, c])", "character codes");
    err_contains("number_chars(X, [a,b,c])", "syntax");
    err_contains("number_codes(X, [97,98,99])", "syntax");
    err_contains("number_chars(X, ['N','a','N'])", "syntax");
    err_contains("number_chars(X, [i,n,f])", "syntax");
}

// ---- ordering / compare / identity -----------------------------------

#[test]
fn compare_3() {
    check("compare(Order, apple, banana)", "Order = <", 1);
    check("compare(Order, zebra, apple)", "Order = >", 1);
    check("compare(Order, same, same)", "Order = =", 1);
    check("compare(Order, 1, 2)", "Order = <", 1);
    // ISO: float < integer when arithmetically equal.
    check("compare(Order, 1.0, 1)", "Order = <", 1);
    check("compare(Order, 1, 1.0)", "Order = >", 1);
}

#[test]
fn term_ordering_operators() {
    succeeds_once("apple @< banana");
    fails("banana @< apple");
    succeeds_once("zebra @> apple");
    succeeds_once("foo @>= foo");
    succeeds_once("foo @=< foo");
    succeeds_once("1 @< hello");
    fails("hello @< 1");
}

#[test]
fn structural_identity() {
    succeeds_once("foo == foo");
    fails("foo \\== foo");
    fails("foo == bar");
    succeeds_once("foo \\== bar");
    succeeds_once("X = Y");
    fails("X == Y");
    succeeds_once("X \\== Y");
    succeeds_once("X == X");
    fails("X \\== X");
    succeeds_once("foo(a, b) == foo(a, b)");
    fails("foo(a, b) == foo(a, c)");
    succeeds_once("foo(a, b) \\== foo(a, c)");
    fails("1 == 1.0");
    succeeds_once("1 =:= 1.0");
}

#[test]
fn compare_compound_with_bound_vars() {
    let c = compile(
        ":- io_format([text, bson]).\n\
         tcmp(Order) :- X = 1, Y = 2, compare(Order, f(X), f(Y)).\n\
         tlt :- X = apple, Y = banana, f(X) @< f(Y).\n",
    );
    let (out, code) = c.query("tcmp(O)", &[]);
    assert_eq!(out, "O = <\n");
    assert_eq!(code, 1);
    let (out, code) = c.query("tlt", &[]);
    assert_eq!(out, "true.\n");
    assert_eq!(code, 1);
}

// ---- copy_term in once (term file's share) ---------------------------

#[test]
fn functor_and_univ_in_once() {
    check(
        "once(functor([a,b], F, A)), F = '.', A = 2",
        "A = 2\nF = .",
        1,
    );
    succeeds_once("once(functor(T, foo, 2)), nonvar(T)");
}
