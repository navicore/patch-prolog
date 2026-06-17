//! Tokenizer for ISO Prolog source.
//!
//! Ported from patch-prolog's `tokenizer.rs`, split into focused submodules:
//! - [`token`]: `TokenKind` / `Token` value types and their `Display`.
//! - [`chars`]: unquoted atoms and variables.
//! - [`numbers`]: integer / float literals.
//! - [`quoted`]: single-quoted atoms.
//! - [`symbols`]: multi-character symbolic operator dispatch.
//!
//! The driver ([`Tokenizer::next_token`]) handles whitespace/comments,
//! single-character punctuation, and dispatches everything else.

mod chars;
mod numbers;
mod quoted;
mod symbols;
mod token;

pub use token::{Token, TokenKind};

pub struct Tokenizer<'a> {
    input: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Tokenizer {
            input: input.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
        let mut tok = Tokenizer::new(input);
        let mut tokens = Vec::new();
        loop {
            let t = tok.next_token()?;
            if t.kind == TokenKind::Eof {
                tokens.push(t);
                break;
            }
            tokens.push(t);
        }
        Ok(tokens)
    }

    pub(super) fn peek(&self) -> Option<u8> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    pub(super) fn peek_at(&self, offset: usize) -> Option<u8> {
        let idx = self.pos + offset;
        if idx < self.input.len() {
            Some(self.input[idx])
        } else {
            None
        }
    }

    pub(super) fn advance(&mut self) -> u8 {
        let ch = self.input[self.pos];
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            match ch {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.advance();
                }
                b'%' => {
                    // Line comment
                    while let Some(ch) = self.peek() {
                        if ch == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                b'/' if self.peek_at(1) == Some(b'*') => {
                    // Block comment
                    self.advance(); // /
                    self.advance(); // *
                    loop {
                        match self.peek() {
                            None => break,
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.advance();
                                self.advance();
                                break;
                            }
                            _ => {
                                self.advance();
                            }
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_whitespace();
        let lo = self.pos as u32;
        let mut token = self.next_token_inner()?;
        // Stamp byte offsets once, at the single dispatch point, so the
        // per-kind helpers don't each have to track them.
        token.lo = lo;
        token.hi = self.pos as u32;
        Ok(token)
    }

    fn next_token_inner(&mut self) -> Result<Token, String> {
        let line = self.line;
        let col = self.col;

        let ch = match self.peek() {
            None => return Ok(Token::new(TokenKind::Eof, line, col)),
            Some(ch) => ch,
        };

        match ch {
            b'(' => self.single(TokenKind::LParen, line, col),
            b')' => self.single(TokenKind::RParen, line, col),
            b'[' => {
                self.advance();
                // Check for []
                if self.peek() == Some(b']') {
                    self.advance();
                    Ok(Token::new(TokenKind::Atom("[]".into()), line, col))
                } else {
                    Ok(Token::new(TokenKind::LBracket, line, col))
                }
            }
            b']' => self.single(TokenKind::RBracket, line, col),
            b'|' => self.single(TokenKind::Pipe, line, col),
            b',' => self.single(TokenKind::Comma, line, col),
            b'!' => self.single(TokenKind::Cut, line, col),
            b';' => self.single(TokenKind::Semicolon, line, col),
            b'.' => {
                // A bare `.` is the clause terminator. Numbers handle their own
                // fractional dot in `read_number`.
                self.single(TokenKind::Dot, line, col)
            }

            b':' | b'?' | b'=' | b'\\' | b'<' | b'>' | b'@' | b'+' | b'*' | b'^' | b'/' | b'-' => {
                self.read_symbol(ch, line, col)
            }

            b'\'' => self.read_quoted_atom(line, col),

            b'0'..=b'9' => self.read_number(line, col),

            b'a'..=b'z' => self.read_atom(line, col),

            b'A'..=b'Z' | b'_' => self.read_variable(line, col),

            _ => {
                self.advance();
                Err(format!(
                    "Unexpected character '{}' at line {} col {}",
                    ch as char, line, col
                ))
            }
        }
    }

    /// Consume one byte and emit a fixed single-character token.
    fn single(&mut self, kind: TokenKind, line: usize, col: usize) -> Result<Token, String> {
        self.advance();
        Ok(Token::new(kind, line, col))
    }
}

#[cfg(test)]
mod tests;
