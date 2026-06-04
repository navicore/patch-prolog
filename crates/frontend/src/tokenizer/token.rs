//! Token kinds and the `Token` value (kind + source line/col).
//!
//! Ported from patch-prolog's `tokenizer.rs`, minus the serde derives —
//! tokens never cross a serialization boundary in the compiler.

/// Token types for Edinburgh Prolog.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Identifiers
    Atom(String),     // lowercase-starting or single-quoted
    Variable(String), // uppercase-starting or _
    Integer(i64),
    Float(f64),

    // Operators
    Neck,      // :-
    QueryOp,   // ?-
    Equals,    // =
    NotEquals, // \=
    TermEq,    // ==
    TermNeq,   // \==
    Is,        // is
    Lt,        // <
    Gt,        // >
    Lte,       // =<
    Gte,       // >=
    ArithEq,   // =:=
    ArithNeq,  // =\=
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    IntDiv,    // //
    Mod,       // mod
    Rem,       // rem
    Not,       // \+
    Backslash, // \  (prefix bitwise complement; #28)
    Cut,       // !
    Arrow,     // ->
    Semicolon, // ;
    // Issue #29: missing standard operators
    Pow,        // **  (xfx 200, float power)
    Caret,      // ^   (xfy 200, int power)
    Colon,      // :   (xfy 200, module qualifier — parser only)
    ShiftLeft,  // <<  (yfx 400)
    ShiftRight, // >>  (yfx 400)
    Div,        // div (yfx 400, floor division)
    BitAnd,     // /\  (yfx 500)
    BitOr,      // \/  (yfx 500)
    Xor,        // xor (yfx 500)

    // Punctuation
    Dot,      // .
    Comma,    // ,
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    Pipe,     // |

    // End of input
    Eof,
}

impl std::fmt::Display for TokenKind {
    /// Surface lexeme for error messages (issue #20). Symbolic operators and
    /// punctuation are backticked so they stand out from prose; named tokens
    /// (atoms, variables, numbers) are written as the user would have typed
    /// them; EOF is described in words.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenKind::Atom(s) => write!(f, "atom `{s}`"),
            TokenKind::Variable(s) => write!(f, "variable `{s}`"),
            TokenKind::Integer(n) => write!(f, "integer `{n}`"),
            TokenKind::Float(x) => write!(f, "float `{x}`"),
            TokenKind::Neck => f.write_str("`:-`"),
            TokenKind::QueryOp => f.write_str("`?-`"),
            TokenKind::Equals => f.write_str("`=`"),
            TokenKind::NotEquals => f.write_str("`\\=`"),
            TokenKind::TermEq => f.write_str("`==`"),
            TokenKind::TermNeq => f.write_str("`\\==`"),
            TokenKind::Is => f.write_str("`is`"),
            TokenKind::Lt => f.write_str("`<`"),
            TokenKind::Gt => f.write_str("`>`"),
            TokenKind::Lte => f.write_str("`=<`"),
            TokenKind::Gte => f.write_str("`>=`"),
            TokenKind::ArithEq => f.write_str("`=:=`"),
            TokenKind::ArithNeq => f.write_str("`=\\=`"),
            TokenKind::Plus => f.write_str("`+`"),
            TokenKind::Minus => f.write_str("`-`"),
            TokenKind::Star => f.write_str("`*`"),
            TokenKind::Slash => f.write_str("`/`"),
            TokenKind::IntDiv => f.write_str("`//`"),
            TokenKind::Mod => f.write_str("`mod`"),
            TokenKind::Rem => f.write_str("`rem`"),
            TokenKind::Not => f.write_str("`\\+`"),
            TokenKind::Backslash => f.write_str("`\\`"),
            TokenKind::Pow => f.write_str("`**`"),
            TokenKind::Caret => f.write_str("`^`"),
            TokenKind::Colon => f.write_str("`:`"),
            TokenKind::ShiftLeft => f.write_str("`<<`"),
            TokenKind::ShiftRight => f.write_str("`>>`"),
            TokenKind::Div => f.write_str("`div`"),
            TokenKind::BitAnd => f.write_str("`/\\`"),
            TokenKind::BitOr => f.write_str("`\\/`"),
            TokenKind::Xor => f.write_str("`xor`"),
            TokenKind::Cut => f.write_str("`!`"),
            TokenKind::Arrow => f.write_str("`->`"),
            TokenKind::Semicolon => f.write_str("`;`"),
            TokenKind::Dot => f.write_str("`.`"),
            TokenKind::Comma => f.write_str("`,`"),
            TokenKind::LParen => f.write_str("`(`"),
            TokenKind::RParen => f.write_str("`)`"),
            TokenKind::LBracket => f.write_str("`[`"),
            TokenKind::RBracket => f.write_str("`]`"),
            TokenKind::Pipe => f.write_str("`|`"),
            TokenKind::Eof => f.write_str("end of input"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}
