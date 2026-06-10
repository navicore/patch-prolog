# LSP Port (ADR)

**Status: DONE (2026-06-09).** Full v1 LSP parity reached. Deltas 1–2
landed (builtin vocabulary table + `STDLIB_PL` now in `plg-shared`,
`docs/design/BUILTIN_VOCAB.md`); delta-3 (point-position diagnostics)
shipped. The crate is `plg-lsp`, binary **`plgl`** (alongside
`plgc`/`plgr`) — not the `plg-lsp` binary name floated below. All four
features verified end-to-end over stdio: parse-error diagnostics,
completion (shared vocabulary + stdlib + buffer predicates,
user-shadows-stdlib-shadows-builtin; operators/`!` filtered via
`completable()`), hover (built-in docs + user clause heads), and
goto-definition (first clause head; builtins/stdlib skipped, stdlib set
derived from `STDLIB_PL` not hardcoded). The `patch-prolog.nvim` plugin
points its default `cmd` at `plgl`.

Known parity-preserving gap (not a regression): hover/goto-def use
`word_at_position`, which extracts identifier runs only — operator-named
builtins (`=..`, `@<`, `\+`) can't be hovered. v1 had the same limit.
Symbol-token recognition would be an additive enhancement.

Bring v1's language server to patch-prolog2. Verdict from reading v1's
`crates/lsp` (1,259 lines: buffer/completion/definition/diagnostics/
hover/main): it is a **pure frontend consumer** — its only engine
imports are `Parser`, `StringInterner`, `Clause`, `ProgramDirectives`,
`Term`, `builtins::{builtin_atom_names, builtin_functor_names}`, and
`STDLIB_PL`. It never touches the solver. Everything it needs exists
API-compatibly in `plg-frontend`/`plg-shared` (the M1 port), so this is
mostly a mechanical port with import renames — like M1, not like M2.

## Plan

New crate `crates/lsp` (`plg-lsp`, binary `plg-lsp`), deps:
`plg-frontend`, `plg-shared`, tower-lsp, tokio, serde/serde_json,
tracing — all isolated from the runtime/compiled-binary dependency
graph (no size impact; the no-heavy-deps rule applies only to
`plg-shared`/`plg-runtime`). Port the six v1 files with imports
swapped; keep v1's design rule verbatim: **the LSP consumes
plg-frontend — no shadow parser.** Version-pin to the workspace like
every other crate.

## The three real deltas

1. **Builtin vocabulary needs a single source of truth.** v1 exposed
   `builtin_atom_names()`/`builtin_functor_names()` from core; plgc2's
   vocabulary currently lives twice (codegen `lower.rs` tables +
   runtime `control.rs` dispatch) and the LSP would be a third copy.
   Promote a data-only table to `plg-shared` —
   `(name, arity, one-line doc)` for every builtin and control
   construct — and have codegen's `DET_BUILTINS` (symbol mapping stays
   compiler-side), the runtime dispatch, completion, AND hover docs all
   derive from it. This turns an existing latent-drift risk into a
   compile-time-shared invariant; do it as the port's first step.

2. **`STDLIB_PL` moves to `plg-shared`.** It currently lives in the
   compiler crate, but `plgc`'s lib embeds the 22M runtime archive via
   `include_bytes!` — the LSP must NOT depend on it. The stdlib source
   is language definition; `plg-shared` (zero deps) is its natural
   home, with the compiler re-exporting for compatibility. Completion
   then offers stdlib predicates exactly as v1 did.

3. **Diagnostics positions.** v1's parser (and ours) reports errors as
   text with embedded `at line N col M`; the LSP lifts point positions
   from that (the same extraction `plgc check` does). Port that as-is
   for parity. A follow-up worth its own issue: structured spans
   (ranges, not points) in `plg-frontend` errors — an additive frontend
   change that would benefit `plgc check` output too. Not a blocker.

## Optional enhancement (post-port)

plgc-aware diagnostics the v1 LSP couldn't have: surface
"goal references undefined, non-dynamic predicate F/A" as a warning by
reusing the registry-construction logic from codegen (`how_to_call`),
and compile-limit notes (arity > 16). These come from the compiler
crate — if added, gate them behind running `plgc check` as a subprocess
rather than linking `plgc`'s lib (the runtime-embed problem again).

## Out of scope

REPL (separate ADR when wanted — it would drive `plgc run`
compile-and-exec, never an in-process interpreter, per
LESSONS_FROM_V1.md rule 3).
