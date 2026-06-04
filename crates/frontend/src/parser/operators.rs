//! Operator table DATA — token → operator-name mappings grouped by ISO
//! precedence level, kept separate from the precedence-climbing logic in
//! [`super::term`].
//!
//! This is deliberately pure data (plus trivial lookups over it) so the M2
//! runtime's minimal goal parser can share the same table without dragging in
//! the full parser. Each `&str` here is the atom name the operator interns to.

use crate::tokenizer::TokenKind;

/// Precedence-700 non-associative operators (`is`, `=`, comparisons, term
/// ordering). Maps a fixed-kind token to its operator atom name. The
/// "word" operators that arrive as `Atom(..)` tokens (`@<`, `=..`, ...) are
/// handled by [`word_op_700`].
pub fn op_700(kind: &TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Is => "is",
        TokenKind::Equals => "=",
        TokenKind::NotEquals => "\\=",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::Lte => "=<",
        TokenKind::Gte => ">=",
        TokenKind::ArithEq => "=:=",
        TokenKind::ArithNeq => "=\\=",
        TokenKind::TermEq => "==",
        TokenKind::TermNeq => "\\==",
        _ => return None,
    })
}

/// Precedence-700 operators that the tokenizer emits as bare atoms
/// (`@<`, `@>`, `@=<`, `@>=`, `=..`). Returns the canonical name when `s`
/// is one of them.
pub fn word_op_700(s: &str) -> Option<&'static str> {
    match s {
        "@<" => Some("@<"),
        "@>" => Some("@>"),
        "@=<" => Some("@=<"),
        "@>=" => Some("@>="),
        "=.." => Some("=.."),
        _ => None,
    }
}

/// Precedence-500 left-associative (`yfx`) operators: `+ - /\ \/ xor`.
pub fn op_500(kind: &TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::BitAnd => "/\\",
        TokenKind::BitOr => "\\/",
        TokenKind::Xor => "xor",
        _ => return None,
    })
}

/// Precedence-400 left-associative (`yfx`) operators:
/// `* / // mod rem div << >>`.
pub fn op_400(kind: &TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::IntDiv => "//",
        TokenKind::Mod => "mod",
        TokenKind::Rem => "rem",
        TokenKind::Div => "div",
        TokenKind::ShiftLeft => "<<",
        TokenKind::ShiftRight => ">>",
        _ => return None,
    })
}

/// Operator name for an operator token appearing in *atom* position
/// (issue #19): `p(+)`, `[<, >]`, `X = (mod)`, `=..` round-trips, etc.
/// This is the superset of all infix/prefix operator spellings.
pub fn op_as_atom(kind: &TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::IntDiv => "//",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::Lte => "=<",
        TokenKind::Gte => ">=",
        TokenKind::Equals => "=",
        TokenKind::NotEquals => "\\=",
        TokenKind::TermEq => "==",
        TokenKind::TermNeq => "\\==",
        TokenKind::ArithEq => "=:=",
        TokenKind::ArithNeq => "=\\=",
        TokenKind::Is => "is",
        TokenKind::Mod => "mod",
        TokenKind::Rem => "rem",
        TokenKind::Not => "\\+",
        TokenKind::Backslash => "\\",
        TokenKind::Arrow => "->",
        TokenKind::Semicolon => ";",
        TokenKind::Pow => "**",
        TokenKind::Caret => "^",
        TokenKind::Colon => ":",
        TokenKind::ShiftLeft => "<<",
        TokenKind::ShiftRight => ">>",
        TokenKind::Div => "div",
        TokenKind::BitAnd => "/\\",
        TokenKind::BitOr => "\\/",
        TokenKind::Xor => "xor",
        _ => return None,
    })
}
