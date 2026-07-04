//! REPL application state and orchestration.
//!
//! Holds the session buffer, the current compiled binary, the input
//! editor, and the scrollback. Enforces the design's two input classes:
//! clause/`:load` edits recompile; `?-` queries run the current binary.

use crate::completion;
use crate::engine::{self, Compiled};
use crate::input::{Editor, Outcome};
use crate::persist;
use crate::run::{self, Fetch};
use crate::session::{Input, MetaCmd, Session, classify};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};
use vim_line::history::{Recall, Store};

/// Solutions fetched per query batch. Small so we don't over-compute when
/// the user only wants the first answer; doubled on demand as they page
/// past it (the stateless-binary re-fetch the design notes).
const PAGE_BATCH: usize = 25;
/// Ceiling on the re-fetch limit, so paging a divergent/huge enumeration
/// can't grow the per-fetch cost (and timeout exposure) without bound.
const SOLUTION_CAP: usize = 4096;

/// The input prompt. Neutral (not `?-`): the same prompt accepts clause
/// definitions and `?-` queries, so it must not imply query context —
/// the user types their own `?-` for a query (echoed as `plg> ?- goal.`).
pub const PROMPT: &str = "plg> ";
/// Continuation prompt for multi-line clause entry (SWI-style).
pub const CONT: &str = "|  ";

/// Active `;`-paging over a query's solutions.
///
/// Re-fetch relies on a *limit-independent solution order*: a bigger batch's
/// first `pos` solutions are exactly the ones already revealed, so `pos`
/// carries across re-fetches. Standard left-to-right SLD resolution gives
/// this; it's the invariant that makes the doubling strategy correct.
struct Paging {
    goal: String,
    /// The fetched batch (rendered solution strings).
    solutions: Vec<String>,
    /// Index of the next unrevealed solution.
    pos: usize,
    /// Limit used to fetch `solutions` (doubles on refetch).
    limit: usize,
    /// Engine reported the search exhausted at `limit` (no more beyond).
    exhausted: bool,
}

#[derive(Default)]
pub struct App {
    pub session: Session,
    pub input: Editor,
    pub output: Vec<String>,
    /// Partial multi-line clause accumulated until a terminating `.`.
    pub pending: String,
    pub should_quit: bool,
    /// Set by `:edit`; `main` performs the terminal-suspend → `$EDITOR`
    /// dance (it owns the `Terminal`) and clears this.
    pub should_edit: bool,
    compiled: Option<Compiled>,
    /// Shared command-history store (vim-line): ring + dedup + draft stash.
    /// Drives `k`/`j` and arrow recall; persisted across sessions.
    store: Store,
    /// The directory this REPL session is keyed to (for per-directory
    /// program persistence — see `persist.rs`). Captured once at startup.
    cwd: PathBuf,
    /// `Some` while paging a query's solutions with `;`.
    paging: Option<Paging>,
}

impl App {
    #[allow(clippy::field_reassign_with_default)] // Default + post-init, like load_history below
    pub fn new() -> Self {
        let mut app = App::default();
        app.cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        app.load_history();
        app.log("plgr — patch-prolog REPL.  :help for commands, :quit to exit.");
        app.restore_session();
        app
    }

    /// Restore this directory's persisted session, if any. Loaded clauses
    /// recompile so the first query works immediately; a `% cwd:` metadata
    /// line (if present) is stripped by `persist::load`.
    fn restore_session(&mut self) {
        let Some(src) = persist::load(&self.cwd) else {
            return;
        };
        if src.trim().is_empty() {
            return;
        }
        match self.session.load_source(&src) {
            Ok(n) if n > 0 => {
                self.recompile();
                self.log(format!(
                    "  restored {n} clause(s) from this directory's session."
                ));
            }
            _ => {}
        }
    }

    /// Write the current session source to this directory's persisted file
    /// (best effort). An empty session deletes the file so a `:reset` sticks.
    pub fn save_session(&self) {
        persist::save(&self.cwd, &self.session.source());
    }

    fn log(&mut self, msg: impl Into<String>) {
        self.output.push(msg.into());
    }

    /// Append a transcript line on behalf of the host (e.g. `:edit` status
    /// from `main`, which owns the terminal but not the scrollback).
    pub fn note(&mut self, msg: impl Into<String>) {
        self.log(msg);
    }

    /// Replace the whole session with edited source (from `:edit`) and
    /// recompile. Atomic: a syntactically bad edit is rejected and the
    /// existing buffer is left intact.
    pub fn apply_edit(&mut self, content: &str) {
        match self.replace_session(content) {
            Ok(n) => {
                self.log(format!("  edited.  ({n} clause(s))"));
                self.recompile();
            }
            Err(e) => self.log(format!("  edit rejected (buffer unchanged): {e}")),
        }
    }

    /// Pure core of the `:edit` reload: parse `content` as a whole program
    /// on a *scratch* session and swap it in only on success (the buffer is
    /// untouched on a parse error). No recompile here, so the atomic-reject
    /// guarantee is unit-testable without invoking the compiler.
    fn replace_session(&mut self, content: &str) -> Result<usize, String> {
        let mut next = Session::default();
        let n = next.load_source(content)?;
        self.session = next;
        Ok(n)
    }

    fn log_block(&mut self, text: &str) {
        for line in text.lines() {
            self.output.push(line.to_string());
        }
    }

    /// True while paging a query's solutions (the input line is replaced by
    /// a `;`/stop hint — see `ui`).
    pub fn is_paging(&self) -> bool {
        self.paging.is_some()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // While paging, keys drive solution navigation, not the editor.
        if self.paging.is_some() {
            self.page_key(key);
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'd') if ctrl => self.should_quit = true,
            KeyCode::Tab => self.complete(),
            _ => match self.input.handle(key) {
                Outcome::Continue => {}
                Outcome::Submit(line) => self.submit(line),
                Outcome::HistoryPrev => self.history_prev(),
                Outcome::HistoryNext => self.history_next(),
                Outcome::Cancel => self.input.clear(),
            },
        }
    }

    /// Recall the previous (older) history entry, stashing the in-progress
    /// line as the draft on the first step (the store handles both).
    fn history_prev(&mut self) {
        if let Some(Recall::Entry(entry)) = self.store.prev(&self.input.text()) {
            let entry = entry.to_string();
            self.input.set(&entry);
        }
    }

    /// Recall the next (newer) entry, or restore the stashed draft once we
    /// step back past the newest entry.
    fn history_next(&mut self) {
        let recalled = match self.store.next() {
            Some(Recall::Entry(e)) | Some(Recall::Draft(e)) => Some(e.to_string()),
            None => None,
        };
        if let Some(text) = recalled {
            self.input.set(&text);
        }
    }

    /// History file path: `$HOME/.local/share/plgr_history` (mirrors seqr's
    /// convention). The store owns dedup/bounding; this host owns the I/O.
    fn history_path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/plgr_history"))
    }

    /// Seed the store from disk at startup. Missing/unreadable file = empty
    /// history; the store bounds itself to its capacity.
    fn load_history(&mut self) {
        let Some(path) = Self::history_path() else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            self.store.load(text.lines().filter(|l| !l.is_empty()));
        }
    }

    /// Write the store's deduped, bounded entries back to disk (best effort).
    pub fn save_history(&self) {
        // Never overwrite a real history file with nothing: an empty store
        // means either a fresh empty session or a failed `load_history`
        // (transient read error) — in both cases, leave the file alone.
        if self.store.is_empty() {
            return;
        }
        let Some(path) = Self::history_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body: String = self.store.entries().map(|e| format!("{e}\n")).collect();
        let _ = std::fs::write(&path, body);
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
        let preds = self.session.predicate_names();
        let cands = completion::candidates(&prefix, &preds);
        if let Some(first) = cands.first() {
            let base = &text[..text.len() - prefix.len()];
            self.input.set(&format!("{base}{first}"));
        }
    }

    /// Accumulate input until a logical entry is complete, then dispatch.
    /// Meta-commands and queries are single-line; clauses run to a `.`.
    fn submit(&mut self, line: String) {
        if self.pending.is_empty() {
            let t = line.trim();
            if t.is_empty() {
                return;
            }
            // Dispatch a complete line immediately; otherwise accumulate.
            // A meta-command (`:` but not the `:-` directive) is single-line
            // by spec; everything else — clause, `?-` query, `:-` directive —
            // runs to a terminating `.`, so a one-line query/directive must
            // NOT short-circuit before its `.` (it'd truncate mid-goal).
            let is_meta = t.starts_with(':') && !t.starts_with(":-");
            if is_meta || t.ends_with('.') {
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
        let kind = classify(&entry);
        if matches!(kind, Input::Empty) {
            return;
        }
        // Record the complete logical entry in history (the store dedups
        // consecutive duplicates and ignores empties).
        self.store.push(entry.trim());
        // Echo what was typed at the prompt, terminal-style — the literal
        // entry (including the user's own `?-` for queries), continuation
        // lines under a `|` like SWI.
        for (i, line) in entry.trim().lines().enumerate() {
            if i == 0 {
                self.log(format!("{PROMPT}{line}"));
            } else {
                self.log(format!("{CONT}{line}"));
            }
        }
        match kind {
            Input::Empty => {}
            Input::Meta(cmd) => self.meta(cmd),
            Input::Query(goal) => self.run_query(&goal),
            // A typed entry may contain more than one clause (e.g. a paste
            // of `foo. bar.`); split it into individual ordered entries via
            // the same path `:load` uses, so the buffer stays one-clause-per
            // -entry and `:list` reads right.
            Input::Clause(c) => match self.session.load_source(&c) {
                Ok(_) => {
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
            },
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
        match run::fetch(&compiled.binary, goal, PAGE_BATCH) {
            Fetch::NoSolutions => self.log("  false."),
            Fetch::Failed(e) => self.log(format!("  {e}")),
            Fetch::Timeout(secs) => self.log(format!("  timed out after {secs}s")),
            Fetch::Error(e) => self.log(format!("  {e}")),
            Fetch::Found {
                solutions,
                exhausted,
            } => {
                self.paging = Some(Paging {
                    goal: goal.to_string(),
                    solutions,
                    pos: 0,
                    limit: PAGE_BATCH,
                    exhausted,
                });
                self.page_next();
            }
        }
    }

    /// A key while paging: `;`/space reveals the next solution; anything else
    /// stops (Ctrl-C/D still quits).
    fn page_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'd')) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Char(';' | ' ') => self.page_next(),
            _ => self.paging = None,
        }
    }

    /// Reveal the next solution, re-fetching a bigger batch first if we've
    /// run past the current one and the engine wasn't exhausted. Ends paging
    /// after the last solution. Each line gets a trailing ` ;` (more) or ` .`
    /// (last), mirroring SWI.
    fn page_next(&mut self) {
        if self.paging.is_none() {
            return;
        }
        let need_more = {
            let p = self.paging.as_ref().unwrap();
            p.pos >= p.solutions.len() && !p.exhausted
        };
        if need_more {
            let (goal, limit) = {
                let p = self.paging.as_ref().unwrap();
                (p.goal.clone(), p.limit)
            };
            let new_limit = limit.saturating_mul(2).min(SOLUTION_CAP);
            if new_limit == limit {
                // Hit the cap and the engine still isn't exhausted.
                self.log(format!("  stopped at {limit} solutions (batch cap)."));
                self.paging = None;
                return;
            }
            let Some(binary) = self.compiled.as_ref().map(|c| c.binary.clone()) else {
                self.paging = None;
                return;
            };
            match run::fetch(&binary, &goal, new_limit) {
                Fetch::Found {
                    solutions,
                    exhausted,
                } => {
                    let p = self.paging.as_mut().unwrap();
                    p.solutions = solutions;
                    p.limit = new_limit;
                    p.exhausted = exhausted;
                }
                _ => {
                    self.paging = None;
                    return;
                }
            }
        }
        let revealed = {
            let p = self.paging.as_mut().unwrap();
            if p.pos >= p.solutions.len() {
                None
            } else {
                let sol = p.solutions[p.pos].clone();
                p.pos += 1;
                let more = p.pos < p.solutions.len() || !p.exhausted;
                Some((sol, more))
            }
        };
        match revealed {
            Some((sol, more)) => {
                self.log(format!("  {sol}{}", if more { " ;" } else { " ." }));
                if !more {
                    self.paging = None;
                }
            }
            None => {
                self.log("  no more solutions.");
                self.paging = None;
            }
        }
    }

    fn meta(&mut self, cmd: MetaCmd) {
        match cmd {
            MetaCmd::Quit => self.should_quit = true,
            MetaCmd::List => {
                if self.session.clauses.is_empty() {
                    self.log("  (empty session)");
                } else {
                    let mut listing = String::new();
                    for (i, clause) in self.session.clauses.iter().enumerate() {
                        for (j, line) in clause.lines().enumerate() {
                            if j == 0 {
                                listing.push_str(&format!("  {:>3}  {line}\n", i + 1));
                            } else {
                                listing.push_str(&format!("       {line}\n"));
                            }
                        }
                    }
                    self.log_block(&listing);
                }
            }
            MetaCmd::Reset => {
                persist::remove(&self.cwd);
                self.session.reset();
                self.compiled = None;
                self.log("  session cleared");
            }
            MetaCmd::Load(path) => self.load_file(Path::new(&path)),
            MetaCmd::Save(path) => self.save(path.as_deref()),
            MetaCmd::Edit => self.should_edit = true,
            MetaCmd::Help => self.log_block(HELP),
            MetaCmd::Unknown(c) => self.log(format!("  unknown command: {c} (try :help)")),
        }
    }

    /// Consult a file: validate it whole, split it into individual ordered
    /// clauses in the session buffer, then recompile.
    pub fn load_file(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => match self.session.load_source(&text) {
                Ok(n) => {
                    self.log(format!("  loaded {} ({n} clause(s))", path.display()));
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
    :edit                     edit the whole session in $EDITOR, recompile
    :save FILE                write the session buffer to FILE
    :reset                    clear the session (and its saved state for this dir)
    :help / :quit             this help / exit
  the session is saved per-directory and restored on restart;
  multi-line clauses continue until a line ends with `.`";

#[cfg(test)]
mod tests {
    use super::App;

    #[test]
    fn replace_session_rejects_bad_source_and_keeps_buffer() {
        let mut app = App::default();
        app.replace_session("ok(1).").unwrap();
        let before = app.session.clauses.clone();
        let err = app.replace_session("garbage(").unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(
            app.session.clauses, before,
            "a bad edit must leave the buffer untouched"
        );
    }

    #[test]
    fn replace_session_swaps_in_split_clauses() {
        let mut app = App::default();
        let n = app.replace_session("foo. bar(X) :- foo.").unwrap();
        assert_eq!(n, 2);
        assert_eq!(app.session.clauses, ["foo.", "bar(X) :- foo."]);
    }
}
