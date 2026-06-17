use super::*;

fn tok(input: &str) -> Vec<TokenKind> {
    Tokenizer::tokenize(input)
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| *k != TokenKind::Eof)
        .collect()
}

#[test]
fn test_atoms() {
    assert_eq!(tok("hello"), vec![TokenKind::Atom("hello".into())]);
    assert_eq!(tok("foo_bar"), vec![TokenKind::Atom("foo_bar".into())]);
    assert_eq!(tok("a123"), vec![TokenKind::Atom("a123".into())]);
}

#[test]
fn lexer_error_carries_span_not_a_trailer() {
    // Invalid byte at offset 4 ("abc ~"): the error pins the offending byte
    // via a Span, and the message has no "at line N col M" trailer.
    let err = Tokenizer::tokenize("abc ~").unwrap_err();
    assert_eq!((err.span.lo, err.span.hi), (4, 5), "got: {err}");
    assert!(!err.message.contains("at line"), "got: {}", err.message);
    assert!(err.message.contains('~'), "got: {}", err.message);
}

#[test]
fn unterminated_quote_points_at_end_of_input() {
    let err = Tokenizer::tokenize("'oops").unwrap_err();
    // Scanner stalls at EOF (offset 5).
    assert_eq!(err.span.lo, 5, "got: {err}");
    assert!(!err.message.contains("at line"), "got: {}", err.message);
}

#[test]
fn test_quoted_atoms() {
    assert_eq!(
        tok("'hello world'"),
        vec![TokenKind::Atom("hello world".into())]
    );
    assert_eq!(tok("'it''s'"), vec![TokenKind::Atom("it's".into())]);
}

#[test]
fn test_variables() {
    assert_eq!(tok("X"), vec![TokenKind::Variable("X".into())]);
    assert_eq!(tok("_foo"), vec![TokenKind::Variable("_foo".into())]);
    assert_eq!(tok("_"), vec![TokenKind::Variable("_".into())]);
    assert_eq!(tok("MyVar"), vec![TokenKind::Variable("MyVar".into())]);
}

#[test]
fn test_numbers() {
    assert_eq!(tok("42"), vec![TokenKind::Integer(42)]);
    assert_eq!(tok("2.5"), vec![TokenKind::Float(2.5)]);
    assert_eq!(tok("0"), vec![TokenKind::Integer(0)]);
}

#[test]
fn test_operators() {
    assert_eq!(tok(":-"), vec![TokenKind::Neck]);
    assert_eq!(tok("?-"), vec![TokenKind::QueryOp]);
    assert_eq!(tok("="), vec![TokenKind::Equals]);
    assert_eq!(tok("\\="), vec![TokenKind::NotEquals]);
    assert_eq!(tok("is"), vec![TokenKind::Is]);
    assert_eq!(tok("<"), vec![TokenKind::Lt]);
    assert_eq!(tok(">"), vec![TokenKind::Gt]);
    assert_eq!(tok("=<"), vec![TokenKind::Lte]);
    assert_eq!(tok(">="), vec![TokenKind::Gte]);
    assert_eq!(tok("=:="), vec![TokenKind::ArithEq]);
    assert_eq!(tok("=\\="), vec![TokenKind::ArithNeq]);
    assert_eq!(tok("=="), vec![TokenKind::TermEq]);
    assert_eq!(tok("\\=="), vec![TokenKind::TermNeq]);
    assert_eq!(tok("\\+"), vec![TokenKind::Not]);
    // Issue #28: bare `\` is the bitwise-complement prefix.
    // Longest match still wins for \=, \==, \+.
    assert_eq!(tok("\\"), vec![TokenKind::Backslash]);
    assert_eq!(
        tok("\\ 3"),
        vec![TokenKind::Backslash, TokenKind::Integer(3)]
    );
    // Issue #29: the rest of the standard operator table.
    assert_eq!(tok("**"), vec![TokenKind::Pow]);
    assert_eq!(tok("^"), vec![TokenKind::Caret]);
    assert_eq!(tok(":"), vec![TokenKind::Colon]);
    assert_eq!(tok("<<"), vec![TokenKind::ShiftLeft]);
    assert_eq!(tok(">>"), vec![TokenKind::ShiftRight]);
    assert_eq!(tok("/\\"), vec![TokenKind::BitAnd]);
    assert_eq!(tok("\\/"), vec![TokenKind::BitOr]);
    assert_eq!(tok("div"), vec![TokenKind::Div]);
    assert_eq!(tok("xor"), vec![TokenKind::Xor]);
    // Longest-match regressions: no false positives.
    assert_eq!(tok("*"), vec![TokenKind::Star]); // not **
    assert_eq!(tok(">="), vec![TokenKind::Gte]); // > then =, not >>
    assert_eq!(tok("//"), vec![TokenKind::IntDiv]); // // not /\
    assert_eq!(tok(":-"), vec![TokenKind::Neck]); // :- still wins over :
}

#[test]
fn test_punctuation() {
    assert_eq!(
        tok("( ) | , ."),
        vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::Pipe,
            TokenKind::Comma,
            TokenKind::Dot,
        ]
    );
    // [ ] with space is separate tokens, not []
    assert_eq!(tok("[ ]"), vec![TokenKind::LBracket, TokenKind::RBracket,]);
}

#[test]
fn test_cut() {
    assert_eq!(tok("!"), vec![TokenKind::Cut]);
}

#[test]
fn test_clause() {
    let tokens = tok("parent(tom, mary).");
    assert_eq!(
        tokens,
        vec![
            TokenKind::Atom("parent".into()),
            TokenKind::LParen,
            TokenKind::Atom("tom".into()),
            TokenKind::Comma,
            TokenKind::Atom("mary".into()),
            TokenKind::RParen,
            TokenKind::Dot,
        ]
    );
}

#[test]
fn test_rule() {
    let tokens = tok("happy(X) :- likes(X, food).");
    assert_eq!(
        tokens,
        vec![
            TokenKind::Atom("happy".into()),
            TokenKind::LParen,
            TokenKind::Variable("X".into()),
            TokenKind::RParen,
            TokenKind::Neck,
            TokenKind::Atom("likes".into()),
            TokenKind::LParen,
            TokenKind::Variable("X".into()),
            TokenKind::Comma,
            TokenKind::Atom("food".into()),
            TokenKind::RParen,
            TokenKind::Dot,
        ]
    );
}

#[test]
fn test_arithmetic() {
    let tokens = tok("X is 2 + 3 * 4.");
    assert_eq!(
        tokens,
        vec![
            TokenKind::Variable("X".into()),
            TokenKind::Is,
            TokenKind::Integer(2),
            TokenKind::Plus,
            TokenKind::Integer(3),
            TokenKind::Star,
            TokenKind::Integer(4),
            TokenKind::Dot,
        ]
    );
}

#[test]
fn test_line_comment() {
    assert_eq!(
        tok("foo % this is a comment\nbar"),
        vec![TokenKind::Atom("foo".into()), TokenKind::Atom("bar".into()),]
    );
}

#[test]
fn test_block_comment() {
    assert_eq!(
        tok("foo /* block */ bar"),
        vec![TokenKind::Atom("foo".into()), TokenKind::Atom("bar".into()),]
    );
}

#[test]
fn test_empty_list() {
    assert_eq!(tok("[]"), vec![TokenKind::Atom("[]".into())]);
}

#[test]
fn test_list_syntax() {
    let tokens = tok("[1, 2, 3]");
    assert_eq!(
        tokens,
        vec![
            TokenKind::LBracket,
            TokenKind::Integer(1),
            TokenKind::Comma,
            TokenKind::Integer(2),
            TokenKind::Comma,
            TokenKind::Integer(3),
            TokenKind::RBracket,
        ]
    );
}

#[test]
fn test_minus_operator() {
    assert_eq!(
        tok("5 - 3"),
        vec![
            TokenKind::Integer(5),
            TokenKind::Minus,
            TokenKind::Integer(3),
        ]
    );
}
