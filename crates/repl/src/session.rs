//! Session state: the ordered source buffer plus input classification.
//!
//! The buffer is the program-so-far. Adding a clause validates it by
//! parsing the *whole* buffer with `plg-frontend` (instant feedback,
//! no compile) and, on success, marks the session dirty so the next run
//! recompiles. Queries never touch the buffer — see `docs/design/REPL.md`.

use plg_frontend::Parser;
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

    pub fn reset(&mut self) {
        self.clauses.clear();
        self.dirty = false;
    }
}
