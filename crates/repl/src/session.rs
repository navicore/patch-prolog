//! Session state: the ordered source buffer plus input classification.
//!
//! The buffer is the program-so-far. Adding a clause validates it by
//! parsing the *whole* buffer with `plg-frontend` (instant feedback,
//! no compile) and, on success, marks the session dirty so the next run
//! recompiles. Queries never touch the buffer.

use plg_frontend::{Parser, SourceMap, TokenKind, Tokenizer};
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

/// One REPL `:`-command: a canonical name plus the aliases accepted at the
/// prompt. The single source of truth for the command surface — `parse_meta`
/// resolves a typed token against this, and completion offers the canonical
/// names. Aliases are for typing/parsing, not completion output.
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

impl CommandSpec {
    /// True if `token` is this command's canonical name or one of its aliases.
    fn matches(&self, token: &str) -> bool {
        self.name == token || self.aliases.contains(&token)
    }
}

/// The REPL command surface, in data. `parse_meta` and completion both read
/// from this; a test pins them in sync.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "quit",
        aliases: &["q"],
    },
    CommandSpec {
        name: "load",
        aliases: &["l"],
    },
    CommandSpec {
        name: "list",
        aliases: &["ls"],
    },
    CommandSpec {
        name: "reset",
        aliases: &[],
    },
    CommandSpec {
        name: "save",
        aliases: &[],
    },
    CommandSpec {
        name: "edit",
        aliases: &["e"],
    },
    CommandSpec {
        name: "help",
        aliases: &["h"],
    },
];

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
    let head = it.next().unwrap_or("");
    // Resolve the typed token against COMMANDS (canonical or alias), then
    // dispatch on the canonical name with per-command argument handling.
    let Some(spec) = COMMANDS.iter().find(|c| c.matches(head)) else {
        return MetaCmd::Unknown(head.to_string());
    };
    match spec.name {
        "quit" => MetaCmd::Quit,
        "load" => match arg(&mut it) {
            Some(f) => MetaCmd::Load(f),
            None => MetaCmd::Unknown(":load needs a file path".into()),
        },
        "list" => MetaCmd::List,
        "reset" => MetaCmd::Reset,
        "save" => MetaCmd::Save(arg(&mut it)),
        "edit" => MetaCmd::Edit,
        "help" => MetaCmd::Help,
        _ => unreachable!("COMMANDS names a command parse_meta doesn't handle"),
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

    /// Validate `text` as a whole program (one clear error, line/col
    /// relative to `text`; the buffer is left untouched on failure), then
    /// append its clauses as **individual ordered entries** so `:list` and
    /// per-clause handling read naturally. Returns the number added.
    ///
    /// This is the single entry path for both `:load` and interactive
    /// clause entry — a typed `foo. bar.` lands as two entries, same as a
    /// consulted file.
    pub fn load_source(&mut self, text: &str) -> Result<usize, String> {
        let mut interner = StringInterner::new();
        Parser::parse_program_with_directives(text, &mut interner).map_err(|e| {
            let (line, col) = SourceMap::new(text).line_col(e.span.lo);
            format!("line {line} col {col}: {}", e.message)
        })?;
        let clauses = split_clauses(text);
        let n = clauses.len();
        self.clauses.extend(clauses);
        if n > 0 {
            self.dirty = true;
        }
        Ok(n)
    }

    /// Predicate names defined or declared in the buffer — clause heads
    /// plus `:- dynamic` declarations — for completion. Re-parses the buffer
    /// (cheap; buffers are small) and resolves functor atoms; on a parse
    /// error (mid-edit) returns what's available, else empty.
    pub fn predicate_names(&self) -> Vec<String> {
        let mut interner = StringInterner::new();
        let Ok((clauses, directives)) =
            Parser::parse_program_with_directives(&self.source(), &mut interner)
        else {
            return Vec::new();
        };
        let mut names: Vec<String> = clauses
            .iter()
            .filter_map(|c| c.head.functor_arity())
            .map(|(id, _)| interner.resolve(id).to_string())
            .chain(
                directives
                    .dynamic
                    .iter()
                    .map(|(id, _)| interner.resolve(*id).to_string()),
            )
            .collect();
        names.sort();
        names.dedup();
        names
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
    use super::{COMMANDS, CommandSpec, MetaCmd, Session, parse_meta, split_clauses};

    #[test]
    fn predicate_names_cover_heads_and_dynamic_decls() {
        let mut s = Session::default();
        s.load_source(
            "parent(tom, bob).\nancestor(X, Y) :- parent(X, Y).\n:- dynamic(extra/1).\ntest.",
        )
        .unwrap();
        // Sorted, deduped: rule + fact heads plus the `:- dynamic` predicate.
        assert_eq!(s.predicate_names(), ["ancestor", "extra", "parent", "test"]);
    }

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

    #[test]
    fn commands_and_parse_meta_are_in_sync() {
        // Every canonical name and alias parses to its MetaCmd (not Unknown)
        // and resolves to the right canonical. A placeholder arg is supplied
        // uniformly so `load` (which requires one) parses; the other commands
        // ignore it.
        for spec in COMMANDS {
            for token in std::iter::once(spec.name).chain(spec.aliases.iter().copied()) {
                let cmd = parse_meta(&format!("{token} _arg"));
                assert!(!matches!(cmd, MetaCmd::Unknown(_)), "{token:?} → Unknown");
                assert_eq!(canonical_of(cmd), spec.name, "{token:?} → wrong canonical");
            }
        }
        // A token COMMANDS doesn't name is Unknown.
        assert!(matches!(parse_meta("nope _arg"), MetaCmd::Unknown(_)));
    }

    /// The canonical name each MetaCmd variant corresponds to (for the sync
    /// test). `Unknown` has none.
    fn canonical_of(cmd: MetaCmd) -> &'static str {
        match cmd {
            MetaCmd::Quit => "quit",
            MetaCmd::Load(_) => "load",
            MetaCmd::List => "list",
            MetaCmd::Reset => "reset",
            MetaCmd::Save(_) => "save",
            MetaCmd::Edit => "edit",
            MetaCmd::Help => "help",
            MetaCmd::Unknown(_) => "",
        }
    }

    #[test]
    fn commands_table_is_the_documented_surface() {
        // The exact canonical set the completion/doc checkpoints assume.
        let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            ["quit", "load", "list", "reset", "save", "edit", "help"]
        );
        // `matches` accepts the canonical name and each alias.
        let load = CommandSpec {
            name: "load",
            aliases: &["l"],
        };
        assert!(load.matches("load") && load.matches("l"));
        assert!(!load.matches("list"));
    }
}
