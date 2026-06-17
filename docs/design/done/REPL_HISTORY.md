# REPL history — consuming vim-line's history (consumer side)

**Status: landed (2026-06-15).** Built against `vim-line` 7.6.0
(`history` feature). The `plg-repl` migration below is done; this doc now
records what shipped. See patch-seq `docs/design/COMMAND_HISTORY.md` for
the store/keymap design.

## What shipped (deltas from the original plan)

- **Store adopted, bespoke history deleted.** `App` dropped
  `history: Vec<String>` + `hist_pos` + `history_nav()`; it now holds a
  `vim_line::history::Store`. `k`/`j` (at the line boundary) and the
  arrows both recall, via `Store::prev`/`next` returning `Recall::{Entry,
  Draft}`. The `history` Cargo feature is enabled on the `vim-line` dep.
- **The reserved search actions** (`HistorySearch`/`Accept`/`Cancel`) are
  matched as no-ops in `input.rs` — vim-line defines them but emits none
  yet (the search sub-mode is a later pass), mirroring seqr.
- **History entries are the *logical* entry** (pushed in `dispatch`, after
  multi-line accumulation), not each physical line as the old code did.
- **Persistence path:** `$HOME/.local/share/plgr_history` via `std::env`
  — mirrors seqr's `seqr_history` convention and avoids adding a `home`/
  XDG dependency (revised from the `$XDG_STATE_HOME` draft below). Loaded
  in `App::new`, written in `main` after the event loop. On-disk format is
  newline-delimited; saves are consecutive-dedup'd (the store's invariant,
  per the patch-seq doc).

## Intent

`plgr` currently hand-rolls history: `App.history: Vec<String>` +
`App.hist_pos` + `history_nav()` in `crates/repl/src/app.rs`, driven only
by `Outcome::History` (which fires only for `Up`/`Down`). Once `vim-line`
completes the history keymap (`k`/`j`) and ships a reusable store, `plgr`
should **delete its bespoke history and adopt the shared store** — gaining
`k`/`j` nav, incremental search, and (new) on-disk persistence, with no
divergence from seqr.

## Constraints

- **Blocked until** `vim-line` publishes the history support (bump the
  caret `vim-line = "7"` lockfile to the new patch; see
  `docs/design/REPL.md`).
- **No behavior regression:** `Up`/`Down` recall must keep working; `k`/`j`
  is additive.
- The REPL still owns *what* a history entry is (a submitted physical
  line) and the persistence path; the store owns the ring/search/draft.

## Approach

- **Remove** `App.history`, `App.hist_pos`, and `history_nav()`; **remove**
  the `Outcome::History` plumbing only if `vim-line`'s glue subsumes it —
  otherwise keep `Outcome::History`/search variants in `input.rs` as the
  thin pass-through, but route them into the **vim-line store** instead of
  the local `Vec`.
- On `submit`, call `store.push(line)` (replacing the manual
  `self.history.push`). On a history intent, set the editor text from the
  store's returned entry (the adapter's `Editor::set` already exists).
- **Persistence (new):** load history from
  `$XDG_STATE_HOME/plgr/history` (fallback `~/.local/state/plgr/history`,
  then `$XDG_CACHE_HOME`) at startup via `store.load(...)`, and dump
  `store.entries()` on `:quit`. The store does no I/O — `plgr` reads/writes
  the file. (Mirror seqr's path convention if it sets one.)
- **Search UI:** when `vim-line` enters its history-search sub-mode,
  surface the query in the prompt line (e.g. `(reverse-i-search)\`q\`:`)
  in `ui.rs`, reusing the same inline-prompt rendering.

## Domain Events

- **`k`/`j` at boundary or `Up`/`Down`** → `vim-line` intent → store
  cursor move → `editor.set(entry)`.
- **Submit** → `store.push(entry)`; reset the store cursor/draft.
- **Startup / `:quit`** → `store.load(file)` / write `store.entries()`.

## Checkpoints

1. `k`/`j` (normal mode) and `Up`/`Down` both recall history in `plgr`.
2. `App` no longer carries `history`/`hist_pos`; `grep` confirms the
   bespoke nav is gone.
3. History survives across `plgr` sessions (persisted file).
4. Reverse-search recalls a prior entry and lets you edit before submit.
5. `just ci` green after the `vim-line` lockfile bump.
