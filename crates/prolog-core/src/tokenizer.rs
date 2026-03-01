use serde::{Deserialize, Serialize};

/// Token types for Edinburgh Prolog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TokenKind {
    // Identifiers
    Atom(String),       // lowercase-starting or single-quoted
    Variable(String),   // uppercase-starting or _
    Integer(i64),
    Float(f64),

    // Operators
    Neck,       // :-
    QueryOp,    // ?-
    Equals,     // =
    NotEquals,  // \=
    Is,         // is
    Lt,         // <
    Gt,         // >
    Lte,        // =<
    Gte,        // >=
    ArithEq,    // =:=
    ArithNeq,   // =\=
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Mod,        // mod
    Not,        // \+
    Cut,        // !
    Arrow,      // ->
    Semicolon,  // ;

    // Punctuation
    Dot,        // .
    Comma,      // ,
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    Pipe,       // |

    // End of input
    Eof,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

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

    fn peek(&self) -> Option<u8> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        let idx = self.pos + offset;
        if idx < self.input.len() {
            Some(self.input[idx])
        } else {
            None
        }
    }

    fn advance(&mut self) -> u8 {
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

        let line = self.line;
        let col = self.col;

        let ch = match self.peek() {
            None => return Ok(Token { kind: TokenKind::Eof, line, col }),
            Some(ch) => ch,
        };

        match ch {
            b'(' => { self.advance(); Ok(Token { kind: TokenKind::LParen, line, col }) }
            b')' => { self.advance(); Ok(Token { kind: TokenKind::RParen, line, col }) }
            b'[' => {
                self.advance();
                // Check for []
                if self.peek() == Some(b']') {
                    self.advance();
                    Ok(Token { kind: TokenKind::Atom("[]".into()), line, col })
                } else {
                    Ok(Token { kind: TokenKind::LBracket, line, col })
                }
            }
            b']' => { self.advance(); Ok(Token { kind: TokenKind::RBracket, line, col }) }
            b'|' => { self.advance(); Ok(Token { kind: TokenKind::Pipe, line, col }) }
            b',' => { self.advance(); Ok(Token { kind: TokenKind::Comma, line, col }) }
            b'!' => { self.advance(); Ok(Token { kind: TokenKind::Cut, line, col }) }
            b';' => { self.advance(); Ok(Token { kind: TokenKind::Semicolon, line, col }) }

            b'.' => {
                self.advance();
                // Check if followed by whitespace/EOF/comment (end of clause)
                // vs followed by digit (float - but we handle that in number parsing)
                Ok(Token { kind: TokenKind::Dot, line, col })
            }

            b':' => {
                self.advance();
                if self.peek() == Some(b'-') {
                    self.advance();
                    Ok(Token { kind: TokenKind::Neck, line, col })
                } else {
                    Err(format!("Unexpected ':' at line {} col {}", line, col))
                }
            }

            b'?' => {
                self.advance();
                if self.peek() == Some(b'-') {
                    self.advance();
                    Ok(Token { kind: TokenKind::QueryOp, line, col })
                } else {
                    Err(format!("Unexpected '?' at line {} col {}", line, col))
                }
            }

            b'=' => {
                self.advance();
                match self.peek() {
                    Some(b':') if self.peek_at(1) == Some(b'=') => {
                        self.advance(); self.advance();
                        Ok(Token { kind: TokenKind::ArithEq, line, col })
                    }
                    Some(b'\\') if self.peek_at(1) == Some(b'=') => {
                        self.advance(); self.advance();
                        Ok(Token { kind: TokenKind::ArithNeq, line, col })
                    }
                    Some(b'<') => {
                        self.advance();
                        Ok(Token { kind: TokenKind::Lte, line, col })
                    }
                    _ => {
                        Ok(Token { kind: TokenKind::Equals, line, col })
                    }
                }
            }

            b'\\' => {
                self.advance();
                match self.peek() {
                    Some(b'=') => {
                        self.advance();
                        Ok(Token { kind: TokenKind::NotEquals, line, col })
                    }
                    Some(b'+') => {
                        self.advance();
                        Ok(Token { kind: TokenKind::Not, line, col })
                    }
                    _ => Err(format!("Unexpected '\\' at line {} col {}", line, col)),
                }
            }

            b'<' => { self.advance(); Ok(Token { kind: TokenKind::Lt, line, col }) }
            b'>' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token { kind: TokenKind::Gte, line, col })
                } else {
                    Ok(Token { kind: TokenKind::Gt, line, col })
                }
            }

            b'+' => { self.advance(); Ok(Token { kind: TokenKind::Plus, line, col }) }
            b'*' => { self.advance(); Ok(Token { kind: TokenKind::Star, line, col }) }
            b'/' => { self.advance(); Ok(Token { kind: TokenKind::Slash, line, col }) }

            b'-' => {
                self.advance();
                // Check for -> (arrow)
                if self.peek() == Some(b'>') {
                    self.advance();
                    return Ok(Token { kind: TokenKind::Arrow, line, col });
                }
                // Check if this is a negative number: dash followed by digit
                if let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        return Ok(Token { kind: TokenKind::Minus, line, col });
                    }
                }
                Ok(Token { kind: TokenKind::Minus, line, col })
            }

            b'\'' => self.read_quoted_atom(line, col),

            b'0'..=b'9' => self.read_number(line, col),

            b'a'..=b'z' => self.read_atom(line, col),

            b'A'..=b'Z' | b'_' => self.read_variable(line, col),

            _ => {
                self.advance();
                Err(format!("Unexpected character '{}' at line {} col {}", ch as char, line, col))
            }
        }
    }

    fn read_atom(&mut self, line: usize, col: usize) -> Result<Token, String> {
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
            _ => TokenKind::Atom(s),
        };
        Ok(Token { kind, line, col })
    }

    fn read_variable(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                s.push(self.advance() as char);
            } else {
                break;
            }
        }
        Ok(Token { kind: TokenKind::Variable(s), line, col })
    }

    fn read_number(&mut self, line: usize, col: usize) -> Result<Token, String> {
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
            let val: f64 = s.parse().map_err(|e| format!("Invalid float '{}': {}", s, e))?;
            Ok(Token { kind: TokenKind::Float(val), line, col })
        } else {
            let val: i64 = s.parse().map_err(|e| format!("Invalid integer '{}': {}", s, e))?;
            Ok(Token { kind: TokenKind::Integer(val), line, col })
        }
    }

    fn read_quoted_atom(&mut self, line: usize, col: usize) -> Result<Token, String> {
        self.advance(); // skip opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(format!("Unterminated quoted atom at line {} col {}", line, col)),
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
                        Some(b'\'') => { s.push('\''); self.advance(); }
                        Some(b'\\') => { s.push('\\'); self.advance(); }
                        Some(b'n') => { s.push('\n'); self.advance(); }
                        Some(b't') => { s.push('\t'); self.advance(); }
                        Some(ch) => { s.push(ch as char); self.advance(); }
                        None => return Err(format!("Unterminated escape at line {} col {}", self.line, self.col)),
                    }
                }
                Some(ch) => {
                    s.push(ch as char);
                    self.advance();
                }
            }
        }
        Ok(Token { kind: TokenKind::Atom(s), line, col })
    }
}

#[cfg(test)]
mod tests {
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
    fn test_quoted_atoms() {
        assert_eq!(tok("'hello world'"), vec![TokenKind::Atom("hello world".into())]);
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
        assert_eq!(tok("3.14"), vec![TokenKind::Float(3.14)]);
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
        assert_eq!(tok("\\+"), vec![TokenKind::Not]);
    }

    #[test]
    fn test_punctuation() {
        assert_eq!(tok("( ) | , ."), vec![
            TokenKind::LParen, TokenKind::RParen,
            TokenKind::Pipe, TokenKind::Comma, TokenKind::Dot,
        ]);
        // [ ] with space is separate tokens, not []
        assert_eq!(tok("[ ]"), vec![
            TokenKind::LBracket, TokenKind::RBracket,
        ]);
    }

    #[test]
    fn test_cut() {
        assert_eq!(tok("!"), vec![TokenKind::Cut]);
    }

    #[test]
    fn test_clause() {
        let tokens = tok("parent(tom, mary).");
        assert_eq!(tokens, vec![
            TokenKind::Atom("parent".into()),
            TokenKind::LParen,
            TokenKind::Atom("tom".into()),
            TokenKind::Comma,
            TokenKind::Atom("mary".into()),
            TokenKind::RParen,
            TokenKind::Dot,
        ]);
    }

    #[test]
    fn test_rule() {
        let tokens = tok("happy(X) :- likes(X, food).");
        assert_eq!(tokens, vec![
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
        ]);
    }

    #[test]
    fn test_arithmetic() {
        let tokens = tok("X is 2 + 3 * 4.");
        assert_eq!(tokens, vec![
            TokenKind::Variable("X".into()),
            TokenKind::Is,
            TokenKind::Integer(2),
            TokenKind::Plus,
            TokenKind::Integer(3),
            TokenKind::Star,
            TokenKind::Integer(4),
            TokenKind::Dot,
        ]);
    }

    #[test]
    fn test_line_comment() {
        assert_eq!(tok("foo % this is a comment\nbar"), vec![
            TokenKind::Atom("foo".into()),
            TokenKind::Atom("bar".into()),
        ]);
    }

    #[test]
    fn test_block_comment() {
        assert_eq!(tok("foo /* block */ bar"), vec![
            TokenKind::Atom("foo".into()),
            TokenKind::Atom("bar".into()),
        ]);
    }

    #[test]
    fn test_empty_list() {
        assert_eq!(tok("[]"), vec![TokenKind::Atom("[]".into())]);
    }

    #[test]
    fn test_list_syntax() {
        let tokens = tok("[1, 2, 3]");
        assert_eq!(tokens, vec![
            TokenKind::LBracket,
            TokenKind::Integer(1),
            TokenKind::Comma,
            TokenKind::Integer(2),
            TokenKind::Comma,
            TokenKind::Integer(3),
            TokenKind::RBracket,
        ]);
    }

    #[test]
    fn test_minus_operator() {
        assert_eq!(tok("5 - 3"), vec![
            TokenKind::Integer(5),
            TokenKind::Minus,
            TokenKind::Integer(3),
        ]);
    }
}
