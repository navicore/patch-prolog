//! Numeric literal scanning: integers and floats. The dot disambiguation
//! (float fraction vs. clause terminator) lives here. Ported from
//! patch-prolog's `tokenizer.rs`.

use super::Tokenizer;
use super::token::{Token, TokenKind};

impl Tokenizer<'_> {
    pub(super) fn read_number(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        let mut is_float = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                s.push(self.advance() as char);
            } else if ch == b'.' {
                // Check if next char after dot is a digit (float), otherwise it's a clause terminator
                if let Some(next) = self.peek_at(1) {
                    if next.is_ascii_digit() {
                        is_float = true;
                        s.push(self.advance() as char); // consume .
                        while let Some(d) = self.peek() {
                            if d.is_ascii_digit() {
                                s.push(self.advance() as char);
                            } else {
                                break;
                            }
                        }
                    } else {
                        break; // dot is clause terminator
                    }
                } else {
                    break; // dot at EOF
                }
            } else {
                break;
            }
        }

        if is_float {
            let val: f64 = s.parse().map_err(|e| format!("Invalid float '{s}': {e}"))?;
            if val.is_infinite() {
                return Err(format!(
                    "Float literal '{s}' overflows f64 at line {line} col {col}"
                ));
            }
            Ok(Token {
                kind: TokenKind::Float(val),
                line,
                col,
            })
        } else {
            let val: i64 = s
                .parse()
                .map_err(|e| format!("Invalid integer '{s}': {e}"))?;
            Ok(Token {
                kind: TokenKind::Integer(val),
                line,
                col,
            })
        }
    }
}
