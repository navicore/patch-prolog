# User-Facing Documentation — scope & shape

**Status: proposed (2026-06-16).** We shipped a compiler (`plgc`), REPL
(`plgr`), LSP (`plgl`), and a stable language subset fast, and let design
docs stand in for documentation. This scopes the first-class doc set so
each shipped feature has a real home — written incrementally, not all now.

## Intent

Give users a single, first-class place to learn each shipped surface (the
language, the CLI, the REPL, the LSP) — guides and reference pages, not
design docs or source. Restore design docs to ephemeral activity
artifacts: **nothing user-facing links into `docs/design/`.** Truth about
behavior lives in a guide; design docs only record *why*.

## Constraints

- **No first-class doc links into `docs/design/`** — not README, not
  `SUMMARY.md`, not any guide. Today's README links
  `docs/design/LESSONS_FROM_V1.md`; that must become prose or a non-design
  target. Design docs are not link targets.
- **Don't fork the engine's authority.** The builtin/stdlib reference
  enumerates the same names as `plg-shared::BUILTINS` + `knowledge/
  stdlib.pl`; it must be checkable against them, not a hand-drifting copy.
- **Reframe, don't rewrite.** `OPERATORS.md` and `ISO_COMPLIANCE.md` are
  already reference-grade; fold/relabel them into the set (and fix the
  stale `prlg` → `plgc` naming in `OPERATORS.md`).
- Out of scope: writing every page now (this fixes the *set* and *order*);
  any language/behavior change; the design-doc archival mechanics.

## Approach

Adopt patch-seq's proven **mdbook** layout — `docs/SUMMARY.md` as the TOC,
README as the Introduction, deployed the same way (`setup-rust-docs`
pattern). The doc set, grouped:

- **Getting Started** — Installation & requirements (`plgc`/`plgr`/`plgl`,
  clang); **Compiler Usage** (`plgc build/run/check/completions`,
  `--query`/`--limit`/`--format`, exit codes & the wire contract, shebang
  script mode).
- **Language** — **Language Guide** (terms, unification, backtracking,
  cut, control, arithmetic, lists — the Prolog mental model); **Grammar &
  Operators** (fold `OPERATORS.md`); **Builtin & Stdlib Reference** (~55
  builtins + the list stdlib); **Semantics & ISO Conformance** (reframe
  `ISO_COMPLIANCE.md` for users).
- **Tooling** — **REPL Guide** (`plgr`: define-vs-`?-` model, `:` commands,
  `;`-paging, vim-line editing + history, completion, compile-on-edit);
  **LSP / Editor Guide** (`plgl`: diagnostics, completion, hover,
  goto-definition, undefined-predicate warnings; Neovim + generic client
  setup; the `--deny-undefined` tie-in).
- **Appendix** — Examples walkthrough; `ARCHITECTURE.md` stays as the
  system overview.

Write incrementally, in order: (1) `SUMMARY.md` skeleton + Getting Started
/ Compiler Usage (the front door), (2) **REPL Guide** (just shipped, most
interactive), (3) Language Guide + Builtin/Stdlib Reference, (4) LSP Guide.

## Domain Events

- **A feature ships** → create/update its one first-class page; move the
  driving design doc to `docs/design/done/` and **delink it everywhere**.
- **Builtin/vocabulary changes** → the Builtin/Stdlib Reference updates;
  the drift check against `BUILTINS` + stdlib fires.
- **Docs build** (mdbook) → `SUMMARY.md` renders; a link into
  `docs/design/` (or a dead link) fails the build.

## Checkpoints

1. `docs/SUMMARY.md` exists and mdbook builds; **no first-class doc links
   into `docs/design/`** (grep gate).
2. A new user installs and runs `plgc` and `plgr` from Getting Started
   alone — no design docs, no source reading.
3. Builtin/Stdlib Reference covers every name in `plg-shared::BUILTINS` +
   the stdlib; drift is caught.
4. Each shipped surface (language, compiler CLI, REPL, LSP) has exactly
   one first-class home.
5. README links only to first-class targets (the `LESSONS_FROM_V1` link is
   gone).
