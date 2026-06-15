//! REPL application state and orchestration.
//!
//! Holds the session buffer, the current compiled binary, the input
//! editor, and the scrollback. Enforces the design's two input classes:
//! clause/`:load` edits recompile; `?-` queries run the current binary.

use crate::completion;
use crate::engine::{self, Compiled};
use crate::input::{Editor, Outcome};
use crate::run::{self, RunResult};
use crate::session::{Input, MetaCmd, Session, classify};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;

/// Solutions fetched per query batch (paging for `;` is a phase-2 TODO).
const QUERY_LIMIT: usize = 100;

#[derive(Default)]
pub struct App {
    pub session: Session,
    pub input: Editor,
    pub output: Vec<String>,
    /// Partial multi-line clause accumulated until a terminating `.`.
    pub pending: String,
    pub should_quit: bool,
    compiled: Option<Compiled>,
    /// Submitted lines, oldest first, for Up/Down recall.
    history: Vec<String>,
    /// Cursor into `history` during recall; `None` = editing a fresh line.
    hist_pos: Option<usize>,
}

impl App {
    pub fn new() -> Self {
        let mut app = App::default();
        app.log("plgr — patch-prolog REPL.  :help for commands, :quit to exit.");
        app
    }

    fn log(&mut self, msg: impl Into<String>) {
        self.output.push(msg.into());
    }

    fn log_block(&mut self, text: &str) {
        for line in text.lines() {
            self.output.push(line.to_string());
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'd') if ctrl => self.should_quit = true,
            KeyCode::Tab => self.complete(),
            _ => match self.input.handle(key) {
                Outcome::Continue => {}
                Outcome::Submit(line) => self.submit(line),
                Outcome::History(prev) => self.history_nav(prev),
                Outcome::Cancel => self.input.clear(),
            },
        }
    }

    /// Recall a previous/next submitted line into the editor.
    fn history_nav(&mut self, prev: bool) {
        if self.history.is_empty() {
            return;
        }
        let pos = match (self.hist_pos, prev) {
            (None, true) => self.history.len() - 1,
            (None, false) => return,
            (Some(i), true) => i.saturating_sub(1),
            (Some(i), false) if i + 1 < self.history.len() => i + 1,
            (Some(_), false) => {
                self.hist_pos = None;
                self.input.set("");
                return;
            }
        };
        self.hist_pos = Some(pos);
        let entry = self.history[pos].clone();
        self.input.set(&entry);
    }

    /// Replace the word under the cursor with the first completion.
    fn complete(&mut self) {
        let text = self.input.text();
        let prefix: String = text
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if prefix.is_empty() {
            return;
        }
        let cands = completion::candidates(&prefix, &[]);
        if let Some(first) = cands.first() {
            let base = &text[..text.len() - prefix.len()];
            self.input.set(&format!("{base}{first}"));
        }
    }

    /// Accumulate input until a logical entry is complete, then dispatch.
    /// Meta-commands and queries are single-line; clauses run to a `.`.
    fn submit(&mut self, line: String) {
        if !line.trim().is_empty() {
            self.history.push(line.clone());
        }
        self.hist_pos = None;
        if self.pending.is_empty() {
            let t = line.trim();
            if t.is_empty() {
                return;
            }
            if t.starts_with(':') || t.starts_with("?-") || t.ends_with('.') {
                self.dispatch(line);
            } else {
                self.pending = line;
            }
        } else {
            self.pending.push('\n');
            self.pending.push_str(&line);
            if line.trim_end().ends_with('.') {
                let entry = std::mem::take(&mut self.pending);
                self.dispatch(entry);
            }
        }
    }

    fn dispatch(&mut self, entry: String) {
        // Echo per input kind — a clause is a definition, not a `?-` query.
        match classify(&entry) {
            Input::Empty => {}
            Input::Meta(cmd) => {
                self.log(entry.trim().to_string());
                self.meta(cmd);
            }
            Input::Query(goal) => {
                self.log(format!("?- {goal}."));
                self.run_query(&goal);
            }
            Input::Clause(c) => {
                self.log(c.clone());
                match self.session.add_clause(&c) {
                    Ok(()) => {
                        self.recompile();
                        if !self.session.dirty {
                            self.log(format!(
                                "  defined.  ({} in session)",
                                self.session.clauses.len()
                            ));
                        }
                    }
                    Err(e) => {
                        self.log(format!("  error: {e}"));
                        if let Some(hint) = capitalization_hint(&c) {
                            self.log(format!("  hint: {hint}"));
                        }
                    }
                }
            }
        }
    }

    /// Recompile the buffer to a fresh binary (only after a program edit).
    fn recompile(&mut self) {
        match engine::compile(&self.session.source()) {
            Ok(c) => {
                self.compiled = Some(c);
                self.session.dirty = false;
            }
            Err(e) => {
                self.compiled = None;
                self.log("  compile failed:");
                self.log_block(&e);
            }
        }
    }

    fn run_query(&mut self, goal: &str) {
        // Queries never recompile; only a stale buffer (an edit since the
        // last build) forces one first.
        if self.session.dirty || self.compiled.is_none() {
            self.recompile();
        }
        let Some(compiled) = &self.compiled else {
            self.log("  no compiled program (fix the errors above)");
            return;
        };
        match run::query(&compiled.binary, goal, QUERY_LIMIT) {
            RunResult::Ok(out) => self.log_block_owned(out),
            RunResult::Failed(err) => self.log_block_owned(err),
            RunResult::Timeout(secs) => self.log(format!("  timed out after {secs}s")),
            RunResult::Error(e) => self.log(format!("  {e}")),
        }
    }

    fn log_block_owned(&mut self, text: String) {
        if text.trim().is_empty() {
            self.log("  false.");
        } else {
            self.log_block(&text);
        }
    }

    fn meta(&mut self, cmd: MetaCmd) {
        match cmd {
            MetaCmd::Quit => self.should_quit = true,
            MetaCmd::List => {
                if self.session.clauses.is_empty() {
                    self.log("  (empty session)");
                } else {
                    let listing = self.session.clauses.join("\n");
                    self.log_block(&listing);
                }
            }
            MetaCmd::Reset => {
                self.session.reset();
                self.compiled = None;
                self.log("  session cleared");
            }
            MetaCmd::Load(path) => self.load_file(Path::new(&path)),
            MetaCmd::Save(path) => self.save(path.as_deref()),
            MetaCmd::Edit => self.log("  :edit is not wired yet (TODO: $EDITOR via shlex)"),
            MetaCmd::Help => self.log_block(HELP),
            MetaCmd::Unknown(c) => self.log(format!("  unknown command: {c} (try :help)")),
        }
    }

    /// Consult a file into the session (append its text + recompile).
    /// TODO: split into per-clause entries so `:list` reads naturally.
    pub fn load_file(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => match self.session.add_clause(text.trim()) {
                Ok(()) => {
                    self.log(format!("  loaded {}", path.display()));
                    self.recompile();
                }
                Err(e) => self.log(format!("  error loading {}: {e}", path.display())),
            },
            Err(e) => self.log(format!("  cannot read {}: {e}", path.display())),
        }
    }

    fn save(&mut self, path: Option<&str>) {
        let Some(path) = path else {
            self.log("  :save needs a file path");
            return;
        };
        match std::fs::write(path, self.session.source()) {
            Ok(()) => self.log(format!("  saved {path}")),
            Err(e) => self.log(format!("  cannot write {path}: {e}")),
        }
    }
}

/// A capitalized leading identifier is read as a *variable* in Prolog, so
/// `Foo(...)` / `Foo :- ...` can't be a clause head. Detect that common
/// trip-up and suggest the lowercase form. (A parser-level version would
/// also serve `plgc`/`plgl` — noted as future frontend work.)
fn capitalization_hint(entry: &str) -> Option<String> {
    let head: String = entry
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let first = head.chars().next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    let lower = head.to_lowercase();
    Some(format!(
        "`{head}` starts with a capital, so Prolog reads it as a variable — \
         predicate names must start lowercase (did you mean `{lower}`?)"
    ))
}

const HELP: &str = "\
  commands:
    foo(a). bar(X):-foo(X).   add a clause/rule (recompiles)
    ?- goal.                  run a query against the current program
    :load FILE                consult a .pl file into the session
    :list                     show the session buffer
    :save FILE                write the session buffer to FILE
    :reset                    clear the session
    :help / :quit             this help / exit
  multi-line clauses continue until a line ends with `.`";
