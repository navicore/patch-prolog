//! Identifier scanning: unquoted atoms (and the keyword operators they may
//! resolve to) and variables. Ported from patch-prolog's `tokenizer.rs`.

use super::Tokenizer;
use super::token::{Token, TokenKind};

impl Tokenizer<'_> {
    pub(super) fn read_atom(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                s.push(self.advance() as char);
            } else {
                break;
            }
        }
        // Check for keyword operators
        let kind = match s.as_str() {
            "is" => TokenKind::Is,
            "mod" => TokenKind::Mod,
            "rem" => TokenKind::Rem,
            // Issue #29
            "div" => TokenKind::Div,
            "xor" => TokenKind::Xor,
            _ => TokenKind::Atom(s),
        };
        Ok(Token::new(kind, line, col))
    }

    pub(super) fn read_variable(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                s.push(self.advance() as char);
            } else {
                break;
            }
        }
        Ok(Token::new(TokenKind::Variable(s), line, col))
    }
}
