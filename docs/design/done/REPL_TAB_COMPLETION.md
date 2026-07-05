# Design: REPL tab completion — commands + stdlib

## Intent

Make in-REPL Tab completion context-aware: when the line is a `:`-command,
complete against REPL commands instead of predicates. Separately, close a
vocabulary gap — stdlib predicates (`append/3`, `member/2`, …) never complete
today because the REPL draws from builtins + session predicates only, not the
embedded stdlib. Both are the same subsystem (vocabulary + dispatch).

## Constraints

- **Must not break:** existing predicate/builtin completion; the `:-` directive
  case (a directive *body* is predicate code — `:-` must route to PREDICATE
  completion, not commands); `parse_meta`'s accepted command set.
- **Out of scope:** successive-Tab cycling / a visual candidate menu. Today Tab
  completes to the first (sorted) candidate; that stays. Cycling is a separate
  follow-up — the `:l` → `:load` vs `:list` ambiguity is its canonical
  motivating case. Arity-aware completion is also out (name-only, as today).

## Approach

Detection lives in `app::complete()` (the one place that knows the input):
**command-mode iff `text.trim_start().starts_with(':') && !starts_with(":-")`**.
The existing backward prefix-scan already yields `"l"` for `:l` (`:` is a
separator), so prefix extraction is unchanged — only routing + vocabulary
change. Command completion matches the prefix against **canonical** command
names and returns canonical names (e.g. `:l` → `list` / `load`, not `ls`); the
leading `:` is supplied by the buffer's existing `base`.

## Structure

**Module/file boundaries**

1. `crates/repl/src/session.rs` — promote the command surface to data: a
   `const COMMANDS: &[CommandSpec]` that becomes the single source `parse_meta`
   reads from. Owns the command vocabulary.
2. `crates/repl/src/completion.rs` — add `command_candidates(prefix)`; extend
   `candidates()` to fold in stdlib names. Pure vocabulary filtering, no
   input/context.
3. `crates/repl/src/app.rs::complete()` — detect command-mode, dispatch.
4. `crates/frontend/src/lib.rs` — `pub fn stdlib_predicates() -> &'static [(String, usize)]`,
   `OnceLock`-cached, parsing `plg_shared::STDLIB_PL` (same pattern
   `session::predicate_names` uses). The parsed stdlib vocabulary, shared.
   (The LSP has its own copy in `crates/lsp/src/completion.rs`; migrating it to
   this helper is a follow-up, not this change.)

**Public interfaces**

- `completion::candidates(prefix: &str, session_preds: &[String]) -> Vec<String>`
  — signature unchanged; now also includes stdlib names.
- `completion::command_candidates(prefix: &str) -> Vec<String>` — new; canonical
  command names matching `prefix` (no leading `:`). Empty prefix ⇒ all
  commands, sorted.
- `session::COMMANDS: &[CommandSpec]` — new; single source for parse_meta +
  completion.
- `plg_frontend::stdlib_predicates() -> &'static [(String, usize)]` — new.

**Data shapes**

- `struct CommandSpec { name: &'static str, aliases: &'static [&'static str] }`,
  e.g. `CommandSpec { name: "load", aliases: &["l"] }`. Canonical names:
  `quit, load, list, reset, save, edit, help`. Completion matches/returns
  canonical names only; aliases are for typing/parsing.
- Stdlib pairs `[(String, usize)]`; the REPL maps to names (name-only
  completion).

## Domain events

- **Tab press** consumes: current input text + cursor. Produces: a mutated
  input buffer (the prefix word replaced by the first sorted candidate, or no
  change if no match).
- **Mode dispatch** (new): on each Tab, the line is classified `Command` vs
  `Predicate`. `Command` ⇒ `command_candidates`; `Predicate` ⇒ `candidates`.
  `:-…` must classify as `Predicate`.
- **Follow-on (out of scope here):** successive-Tab cycling depends on
  candidates being deterministically ordered (they are — sorted, deduped), so
  the cycling follow-up can index into the returned `Vec` without re-querying.

## Checkpoints

- `completion::command_candidates`: `"l"` → `["list","load"]`; `"e"` →
  `["edit"]`; `""` → all seven canonicals sorted; `"x"` → `[]`.
- `completion::candidates` now contains `append`, `member`, `length`,
  `reverse`, `nth0`, `nth1`, `last`.
- A test pinning `COMMANDS` ↔ `parse_meta` in sync: every canonical + alias
  parses to the right `MetaCmd`, and every `parse_meta` arm appears in
  `COMMANDS`.
- `plg_frontend::stdlib_predicates` includes at least `(append,3)`, `(member,2)`.
- Dispatch test: `app::complete` routes `:l` → command vocabulary and `:- foo`
  → predicate vocabulary. (May require factoring a pure `completion_mode(text)`
  predicate out of `complete()` for unit testability.)
