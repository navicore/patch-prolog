# Design: REPL Tab cycling through completion matches

## Intent

Successive Tab presses cycle through the sorted candidate list instead of
always completing to the first match. Closes the ergonomic gap left by
`REPL_TAB_COMPLETION.md` (in `done/`): today `:l` always becomes `:list` and
`:load` is unreachable. Applies uniformly to command and predicate completion.

## Constraints

- **Must not break:** single-Tab completion (still completes to the first
  match); predicate vs command dispatch from the prior change; `;`-paging,
  history, and the rest of the input editor.
- **Out of scope (this change):** a visible candidate list / popup menu. Silent
  cycling only (option (a)) — the word swaps with no list drawn. A popup is a
  separate, larger TUI change.
- A stale completion session must never apply to edited text: any non-Tab edit
  or cursor move discards the session and starts fresh on the next Tab.

## Approach

`App` holds an optional `CompletionState` (sibling to `Paging`). On Tab:
- If a session is active **and** the current line still equals
  `base + candidates[idx]` (unchanged since the last Tab), advance `idx`
  (wrap to 0) and apply `candidates[idx]`.
- Otherwise compute fresh candidates (command or predicate via the existing
  `completion_mode`), set `idx = 0`, apply the first, and stash the session.

The "did the line change?" guard is the load-bearing correctness check: it
prevents cycling from clobbering text the user typed after the last Tab.

## Structure

**Module/file boundaries**

1. `crates/repl/src/app.rs` — add `completion: Option<CompletionState>` field;
   rewrite `complete()` to create/advance/discard sessions; clear the session on
   any non-Tab input event.
2. `crates/repl/src/input.rs` — no change expected; the line-equality guard
   lives in `app.rs`.

**Public interfaces / data shapes**

- `struct CompletionState { base: String, prefix: String, candidates: Vec<String>, idx: usize }`
  — internal to `App`; `base` + the active candidate rebuild the line, `prefix`
  is retained only to document what was matched.
- No new public API.

**Cycle semantics (load-bearing)**

- Repeated Tab with an unchanged line ⇒ `idx = (idx + 1) % candidates.len()`,
  apply `candidates[idx]`.
- Empty candidate set ⇒ no session, no-op (same as today).
- Command and predicate modes use the same path; only the candidate source
  (`command_candidates` vs `candidates`) differs, selected by `completion_mode`.

## Domain events

- **Tab (line unchanged since last Tab)** consumes: active `CompletionState`.
  Produces: input buffer mutated to the next candidate; `idx` advanced.
- **Tab (line changed / no session)** consumes: current text + mode. Produces: a
  fresh `CompletionState`, buffer set to the first candidate.
- **Any non-Tab input** (keystroke, cursor move, submit) produces: the
  `CompletionState` is dropped, so the next Tab starts fresh.
- **Follow-on:** a popup menu (deferred) would observe `candidates` + `idx` and
  render a highlight; the state shape above already carries what it needs.

## Checkpoints

- `:l` → Tab → `:list`; Tab again → `:load`; Tab again → wraps to `:list`.
- `app` → Tab → `append`; Tab cycles through every name starting with `app`.
- Edit the line (type a char) after a Tab ⇒ next Tab recomputes from scratch
  (no stale cycling).
- Single Tab on a one-match prefix behaves exactly as today.
- Empty-candidate Tab is a no-op (buffer unchanged, no session created).
