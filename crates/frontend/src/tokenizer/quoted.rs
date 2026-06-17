//! Single-quoted atom scanning, including `''` doubling and backslash
//! escapes. Ported from patch-prolog's `tokenizer.rs`.

use super::Tokenizer;
use super::token::{Token, TokenKind};
use crate::parse_error::ParseError;

impl Tokenizer<'_> {
    pub(super) fn read_quoted_atom(
        &mut self,
        line: usize,
        col: usize,
    ) -> Result<Token, ParseError> {
        self.advance(); // skip opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.lex_error("Unterminated quoted atom")),
                Some(b'\'') => {
                    self.advance();
                    // Check for escaped quote ''
                    if self.peek() == Some(b'\'') {
                        s.push('\'');
                        self.advance();
                    } else {
                        break;
                    }
                }
                Some(b'\\') => {
                    self.advance();
                    match self.peek() {
                        Some(b'\'') => {
                            s.push('\'');
                            self.advance();
                        }
                        Some(b'\\') => {
                            s.push('\\');
                            self.advance();
                        }
                        Some(b'n') => {
                            s.push('\n');
                            self.advance();
                        }
                        Some(b't') => {
                            s.push('\t');
                            self.advance();
                        }
                        Some(ch) => {
                            s.push(ch as char);
                            self.advance();
                        }
                        None => {
                            return Err(self.lex_error("Unterminated escape"));
                        }
                    }
                }
                Some(ch) => {
                    s.push(ch as char);
                    self.advance();
                }
            }
        }
        Ok(Token::new(TokenKind::Atom(s), line, col))
    }
}
