# Design: LSP `stdlib_predicates` dedup

## Intent

The LSP has its own `stdlib_predicates()` in `crates/lsp/src/completion.rs`;
`plg_frontend::stdlib_predicates()` now exists as the shared source (added with
REPL stdlib completion). Make the LSP consume the shared one and delete the
local copy — one source of truth for the stdlib vocabulary, no third copy.

## Constraints

- **Must not break:** LSP completion offerings (builtins + stdlib + user); the
  `definition` skip-goto-def behavior for stdlib predicates.
- **Out of scope:** changing what predicates the stdlib *contains* (that's
  `plg_shared::STDLIB_PL`); touching the REPL, which already uses the shared
  helper.

## Approach

Replace the LSP's local `OnceLock`-cached parser with a re-export of
`plg_frontend::stdlib_predicates`. Both parse `STDLIB_PL` and extract
`(name, arity)` from clause heads; the only difference is the parser used
(`parse_best_effort`, tolerant, vs `parse_program_with_directives`, strict) —
the stdlib parses cleanly under both, so the result sets are identical. A test
pins that equality before the local copy is removed.

## Structure

**Module/file boundaries**

1. `crates/lsp/src/completion.rs` — delete the local `stdlib_predicates()`;
   `use plg_frontend::stdlib_predicates;` (or re-export under the old path if
   callers reference it unqualified).
2. `crates/lsp/src/definition.rs` — repoint its use (skip goto-def for stdlib
   predicates) to the shared function.

**Public interfaces**

- `plg_frontend::stdlib_predicates() -> &'static [(String, usize)]` — the single
  source (already exists, unchanged).
- No new LSP public API; the local `pub(crate)` fn is deleted.

**Data shapes**

- `[(String, usize)]` — `(name, arity)` pairs; identical shape to the deleted
  LSP helper.

## Domain events

- **Completion request** consumes: shared `stdlib_predicates()` instead of the
  local copy. Produces: the same stdlib completion items.
- **Goto-definition** consumes: shared `stdlib_predicates()` to decide a
  predicate has no user source to jump to. Produces: the same skip behavior.

## Checkpoints

- Before deletion: a test asserting the local and shared `(name, arity)` sets
  are identical (the stdlib parses the same under both parsers).
- After deletion: existing LSP completion + definition unit tests pass
  unchanged.
- `grep stdlib_predicates crates/lsp` shows only the `use`, no local defn.
