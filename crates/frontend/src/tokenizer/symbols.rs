//! Multi-character symbolic operator scanning. Longest-match disambiguation
//! (`\==` before `\=`, `**` before `*`, etc.) lives here. Ported verbatim
//! from patch-prolog's `tokenizer.rs` `next_token`.

use super::Tokenizer;
use super::token::{Token, TokenKind};

impl Tokenizer<'_> {
    /// Read a symbolic operator starting at byte `ch`. `ch` has been peeked
    /// but not consumed.
    pub(super) fn read_symbol(&mut self, ch: u8, line: usize, col: usize) -> Result<Token, String> {
        let kind = match ch {
            b':' => self.sym_colon(line, col)?,
            b'?' => self.sym_question(line, col)?,
            b'=' => self.sym_equals(line, col)?,
            b'\\' => self.sym_backslash(),
            b'<' => self.sym_lt(),
            b'>' => self.sym_gt(),
            b'@' => return self.sym_at(line, col),
            b'+' => {
                self.advance();
                TokenKind::Plus
            }
            b'*' => self.sym_star(),
            b'^' => {
                self.advance();
                TokenKind::Caret
            }
            b'/' => self.sym_slash(),
            b'-' => self.sym_minus(),
            _ => unreachable!("read_symbol called with non-symbol byte"),
        };
        Ok(Token::new(kind, line, col))
    }

    fn sym_colon(&mut self, _line: usize, _col: usize) -> Result<TokenKind, String> {
        self.advance();
        if self.peek() == Some(b'-') {
            self.advance();
            Ok(TokenKind::Neck)
        } else {
            // Issue #29: bare `:` is the module-qualifier infix (xfy 200).
            Ok(TokenKind::Colon)
        }
    }

    fn sym_question(&mut self, line: usize, col: usize) -> Result<TokenKind, String> {
        self.advance();
        if self.peek() == Some(b'-') {
            self.advance();
            Ok(TokenKind::QueryOp)
        } else {
            Err(format!("Unexpected '?' at line {line} col {col}"))
        }
    }

    fn sym_equals(&mut self, _line: usize, _col: usize) -> Result<TokenKind, String> {
        self.advance();
        let kind = match self.peek() {
            Some(b'=') => {
                self.advance();
                TokenKind::TermEq
            }
            Some(b':') if self.peek_at(1) == Some(b'=') => {
                self.advance();
                self.advance();
                TokenKind::ArithEq
            }
            Some(b'\\') if self.peek_at(1) == Some(b'=') => {
                self.advance();
                self.advance();
                TokenKind::ArithNeq
            }
            Some(b'<') => {
                self.advance();
                TokenKind::Lte
            }
            Some(b'.') if self.peek_at(1) == Some(b'.') => {
                self.advance();
                self.advance();
                TokenKind::Atom("=..".into())
            }
            _ => TokenKind::Equals,
        };
        Ok(kind)
    }

    fn sym_backslash(&mut self) -> TokenKind {
        self.advance();
        match self.peek() {
            // Longest match: \== before \=.
            Some(b'=') if self.peek_at(1) == Some(b'=') => {
                self.advance();
                self.advance();
                TokenKind::TermNeq
            }
            Some(b'=') => {
                self.advance();
                TokenKind::NotEquals
            }
            Some(b'+') => {
                self.advance();
                TokenKind::Not
            }
            // Issue #29: `\/` (bitwise or) before bare `\`.
            Some(b'/') => {
                self.advance();
                TokenKind::BitOr
            }
            // Issue #28: bare `\` is the unary bitwise-complement prefix
            // operator (ISO fy 200). The longer escapes above (\=, \==,
            // \+, \/) are tried first by longest match.
            _ => TokenKind::Backslash,
        }
    }

    fn sym_lt(&mut self) -> TokenKind {
        self.advance();
        // Issue #29: `<<` (shift left) before bare `<`.
        if self.peek() == Some(b'<') {
            self.advance();
            TokenKind::ShiftLeft
        } else {
            TokenKind::Lt
        }
    }

    fn sym_gt(&mut self) -> TokenKind {
        self.advance();
        // Issue #29: `>>` (shift right) before `>=` and bare `>`.
        if self.peek() == Some(b'>') {
            self.advance();
            TokenKind::ShiftRight
        } else if self.peek() == Some(b'=') {
            self.advance();
            TokenKind::Gte
        } else {
            TokenKind::Gt
        }
    }

    fn sym_at(&mut self, line: usize, col: usize) -> Result<Token, String> {
        self.advance();
        let kind = match self.peek() {
            Some(b'<') => {
                self.advance();
                TokenKind::Atom("@<".into())
            }
            Some(b'>') => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    TokenKind::Atom("@>=".into())
                } else {
                    TokenKind::Atom("@>".into())
                }
            }
            Some(b'=') if self.peek_at(1) == Some(b'<') => {
                self.advance();
                self.advance();
                TokenKind::Atom("@=<".into())
            }
            _ => return Err(format!("Unexpected '@' at line {line} col {col}")),
        };
        Ok(Token::new(kind, line, col))
    }

    fn sym_star(&mut self) -> TokenKind {
        self.advance();
        // Issue #29: `**` (float power) before bare `*`.
        if self.peek() == Some(b'*') {
            self.advance();
            TokenKind::Pow
        } else {
            TokenKind::Star
        }
    }

    fn sym_slash(&mut self) -> TokenKind {
        self.advance();
        // Longest match: `//` (int div), then `/\` (bitwise and), else `/`.
        if self.peek() == Some(b'/') {
            self.advance();
            TokenKind::IntDiv
        } else if self.peek() == Some(b'\\') {
            self.advance();
            TokenKind::BitAnd
        } else {
            TokenKind::Slash
        }
    }

    fn sym_minus(&mut self) -> TokenKind {
        self.advance();
        // Check for -> (arrow)
        if self.peek() == Some(b'>') {
            self.advance();
            return TokenKind::Arrow;
        }
        // Negative numbers are handled by the parser's unary-minus folding;
        // the tokenizer always emits a bare `-`.
        TokenKind::Minus
    }
}
