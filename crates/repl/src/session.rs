//! Session state: the ordered source buffer plus input classification.
//!
//! The buffer is the program-so-far. Adding a clause validates it by
//! parsing the *whole* buffer with `plg-frontend` (instant feedback,
//! no compile) and, on success, marks the session dirty so the next run
//! recompiles. Queries never touch the buffer — see `docs/design/REPL.md`.

use plg_frontend::{Parser, TokenKind, Tokenizer};
use plg_shared::StringInterner;

/// One line of REPL input, classified.
pub enum Input {
    /// A clause/rule/directive — a program edit (recompiles on next run).
    Clause(String),
    /// A `?- goal` query — runs against the current binary, no recompile.
    Query(String),
    /// A `:`-prefixed meta-command.
    Meta(MetaCmd),
    /// Blank line.
    Empty,
}

pub enum MetaCmd {
    Load(String),
    List,
    Reset,
    Save(Option<String>),
    Edit,
    Help,
    Quit,
    Unknown(String),
}

/// Classify a single, complete logical entry.
pub fn classify(entry: &str) -> Input {
    let t = entry.trim();
    if t.is_empty() {
        return Input::Empty;
    }
    if let Some(rest) = t.strip_prefix("?-") {
        let goal = rest.trim().trim_end_matches('.').trim();
        return Input::Query(goal.to_string());
    }
    // `:- ...` is a Prolog *directive* (e.g. `:- dynamic(f/1).`), a program
    // edit — not a REPL meta-command. Must be checked before the `:` below.
    if t.starts_with(":-") {
        return Input::Clause(t.to_string());
    }
    if let Some(rest) = t.strip_prefix(':') {
        return Input::Meta(parse_meta(rest));
    }
    Input::Clause(t.to_string())
}

fn parse_meta(s: &str) -> MetaCmd {
    let mut it = s.split_whitespace();
    let arg = |it: &mut std::str::SplitWhitespace| it.next().map(str::to_string);
    match it.next().unwrap_or("") {
        "q" | "quit" => MetaCmd::Quit,
        "load" | "l" => match arg(&mut it) {
            Some(f) => MetaCmd::Load(f),
            None => MetaCmd::Unknown(":load needs a file path".into()),
        },
        "list" | "ls" => MetaCmd::List,
        "reset" => MetaCmd::Reset,
        "save" => MetaCmd::Save(arg(&mut it)),
        "edit" | "e" => MetaCmd::Edit,
        "help" | "h" => MetaCmd::Help,
        other => MetaCmd::Unknown(other.to_string()),
    }
}

#[derive(Default)]
pub struct Session {
    /// Ordered source entries (clauses/directives), program order.
    pub clauses: Vec<String>,
    /// Buffer changed since the last successful compile.
    pub dirty: bool,
}

impl Session {
    /// The full program source the next compile will see.
    pub fn source(&self) -> String {
        let mut s = self.clauses.join("\n");
        s.push('\n');
        s
    }

    /// Validate that `entry` parses (clauses are independent in ISO
    /// Prolog, so it is checked on its own — keeping error line/col
    /// relative to what the user typed) and, if so, append it and mark
    /// the session dirty. On a parse error the buffer is left untouched.
    pub fn add_clause(&mut self, entry: &str) -> Result<(), String> {
        let mut interner = StringInterner::new();
        Parser::parse_program_with_directives(entry, &mut interner)?;
        self.clauses.push(entry.to_string());
        self.dirty = true;
        Ok(())
    }

    /// Consult `text` as a whole program: validate it once (one clear error
    /// for a bad file), then append its clauses as **individual ordered
    /// entries** so `:list` and per-clause handling read naturally. Returns
    /// the number of clauses added.
    pub fn load_source(&mut self, text: &str) -> Result<usize, String> {
        let mut interner = StringInterner::new();
        Parser::parse_program_with_directives(text, &mut interner)?;
        let clauses = split_clauses(text);
        let n = clauses.len();
        self.clauses.extend(clauses);
        if n > 0 {
            self.dirty = true;
        }
        Ok(n)
    }

    pub fn reset(&mut self) {
        self.clauses.clear();
        self.dirty = false;
    }
}

/// Split program source into individual clause strings, each including its
/// terminating `.` and preserving original text (comments included). Uses
/// the real tokenizer, so quoted atoms, line/block comments, and floats
/// (`3.14`) never produce a false clause boundary — only a bare `.`
/// end-token does. Comment-only / whitespace-only spans are dropped.
pub fn split_clauses(src: &str) -> Vec<String> {
    let Ok(tokens) = Tokenizer::tokenize(src) else {
        // Let the caller's whole-program parse report the real error.
        return vec![src.trim().to_string()];
    };
    let mut clauses = Vec::new();
    let mut start = 0;
    let mut has_content = false;
    for tok in &tokens {
        match tok.kind {
            TokenKind::Dot => {
                let end = byte_offset(src, tok.line, tok.col) + 1;
                if has_content {
                    let chunk = src[start..end].trim();
                    if !chunk.is_empty() {
                        clauses.push(chunk.to_string());
                    }
                }
                start = end;
                has_content = false;
            }
            TokenKind::Eof => break,
            _ => has_content = true,
        }
    }
    clauses
}

/// Byte offset of a token at 1-based (`line`, `col`). The tokenizer indexes
/// bytes and bumps `col` per byte, so `col` is a byte column within its line.
fn byte_offset(src: &str, line: usize, col: usize) -> usize {
    let mut offset = 0;
    for (i, l) in src.split_inclusive('\n').enumerate() {
        if i + 1 == line {
            return offset + col - 1;
        }
        offset += l.len();
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::split_clauses;

    #[test]
    fn splits_multiple_clauses_in_order() {
        let got = split_clauses("fact(1).\nfact(2).\ntest :- fact(1).\n");
        assert_eq!(got, ["fact(1).", "fact(2).", "test :- fact(1)."]);
    }

    #[test]
    fn directive_is_its_own_clause() {
        let got = split_clauses(":- dynamic(fruit/1).\nfruit(apple).");
        assert_eq!(got, [":- dynamic(fruit/1).", "fruit(apple)."]);
    }

    #[test]
    fn float_dot_is_not_a_boundary() {
        assert_eq!(
            split_clauses("pi(3.14).\ne(2.71)."),
            ["pi(3.14).", "e(2.71)."]
        );
    }

    #[test]
    fn dot_inside_quoted_atom_is_not_a_boundary() {
        assert_eq!(split_clauses("p('a.b').\nq(x)."), ["p('a.b').", "q(x)."]);
    }

    #[test]
    fn keeps_clauses_spanning_multiple_lines() {
        let got = split_clauses("a(X) :-\n    b(X),\n    c(X).\nd.");
        assert_eq!(got, ["a(X) :-\n    b(X),\n    c(X).", "d."]);
    }

    #[test]
    fn skips_comment_only_and_blank_spans() {
        // A trailing/standalone comment is not emitted as a clause; a leading
        // comment rides with the clause that follows it.
        let got = split_clauses("% header\nfoo.\n% trailing\n");
        assert_eq!(got, ["% header\nfoo."]);
    }

    #[test]
    fn block_comment_does_not_split_and_rides_with_clause() {
        // A `/* */` comment between clauses rides with the following clause...
        assert_eq!(
            split_clauses("p. /* between */ q."),
            ["p.", "/* between */ q."]
        );
        // ...and a `.` *inside* a block comment is not a clause boundary.
        assert_eq!(split_clauses("p(/* . */ x)."), ["p(/* . */ x)."]);
    }
}
