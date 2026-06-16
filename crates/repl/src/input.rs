//! Vim-motion line editor: a thin host adapter over the `vim-line` crate.
//!
//! `vim-line` is terminal-agnostic and owns no text, so this module does
//! the two glue jobs its contract leaves to the host: translate crossterm
//! key events into `vim_line::Key`, and own the text buffer (applying the
//! `TextEdit`s the editor returns). Editor actions (submit, history,
//! cancel) are surfaced to `app` as an `Outcome`.

use crossterm::event::{KeyCode as CtCode, KeyEvent, KeyModifiers};
use vim_line::{Action, Key, KeyCode, LineEditor, VimLineEditor};

/// What a key press resolved to, once any edits were applied.
pub enum Outcome {
    /// Edits applied (or nothing happened); keep editing.
    Continue,
    /// The user submitted this line.
    Submit(String),
    /// History recall — previous (older) entry.
    HistoryPrev,
    /// History recall — next (newer) entry.
    HistoryNext,
    /// The user cancelled the current line.
    Cancel,
}

pub struct Editor {
    inner: VimLineEditor,
    text: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            inner: VimLineEditor::new(),
            text: String::new(),
        }
    }
}

impl Editor {
    /// Current line text.
    pub fn text(&self) -> String {
        self.text.clone()
    }

    /// Cursor as a character column (vim-line reports a byte offset).
    pub fn cursor_col(&self) -> usize {
        let b = self.inner.cursor().min(self.text.len());
        self.text
            .get(..b)
            .map(|s| s.chars().count())
            .unwrap_or_else(|| self.text.chars().count())
    }

    /// Mode label for display ("NORMAL", "INSERT", …).
    pub fn status(&self) -> &str {
        self.inner.status()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.inner.reset();
    }

    /// Replace the line (used by completion and history recall).
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        let end = self.text.len();
        self.inner.set_cursor(end, &self.text);
    }

    /// Feed a key, apply any resulting edits, and report the outcome.
    pub fn handle(&mut self, event: KeyEvent) -> Outcome {
        let Some(key) = to_key(event) else {
            return Outcome::Continue;
        };
        let result = self.inner.handle_key(key, &self.text);
        // Apply edits back-to-front so each edit's byte offsets stay valid.
        for edit in result.edits.into_iter().rev() {
            edit.apply(&mut self.text);
        }
        match result.action {
            Some(Action::Submit) => {
                let line = std::mem::take(&mut self.text);
                self.inner.reset();
                Outcome::Submit(line)
            }
            Some(Action::SubmitCommand(cmd)) => {
                self.inner.reset();
                Outcome::Submit(format!(":{cmd}"))
            }
            Some(Action::HistoryPrev) => Outcome::HistoryPrev,
            Some(Action::HistoryNext) => Outcome::HistoryNext,
            Some(Action::Cancel) => Outcome::Cancel,
            // History-search vocabulary is reserved for a future vim-line
            // search sub-mode — not emitted today, so a no-op here.
            Some(Action::HistorySearch(_) | Action::HistoryAccept | Action::HistoryCancel) => {
                Outcome::Continue
            }
            None => Outcome::Continue,
        }
    }
}

/// Translate a crossterm key event into a `vim_line::Key`.
fn to_key(event: KeyEvent) -> Option<Key> {
    let code = match event.code {
        CtCode::Char(c) => KeyCode::Char(c),
        CtCode::Esc => KeyCode::Escape,
        CtCode::Backspace => KeyCode::Backspace,
        CtCode::Delete => KeyCode::Delete,
        CtCode::Left => KeyCode::Left,
        CtCode::Right => KeyCode::Right,
        CtCode::Up => KeyCode::Up,
        CtCode::Down => KeyCode::Down,
        CtCode::Home => KeyCode::Home,
        CtCode::End => KeyCode::End,
        CtCode::Tab => KeyCode::Tab,
        CtCode::Enter => KeyCode::Enter,
        _ => return None,
    };
    let m = event.modifiers;
    Some(Key {
        code,
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
    })
}
