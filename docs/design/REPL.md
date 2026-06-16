# REPL (`plgr`) — design note

**Status: proposed (2026-06-14).** Brings v1's REPL back (ROADMAP
"Future"), modelled on patch-seq's `seqr`, which already solved the
"compiler-not-interpreter" problem we share. Crate `crates/repl`
(`plg-repl`), binary **`plgr`** — the third of `plgc`/`plgl`/`plgr`.

## Intent

Give Prolog users an interactive loop — enter clauses, run `?-` queries,
see solutions, backtrack with `;` — **without** reintroducing an
in-process interpreter. The engine compiles whole programs to native
binaries; the REPL delivers "interactive feel" by *driving the compiler*,
not by walking clauses at runtime. seqr proves this is pleasant: it links
the compiler for instant parse/codegen feedback and **execs compiled
binaries as bounded subprocesses** for actual runs.

## Constraints

- **Rule 3 (LESSONS_FROM_V1) is the whole game: the REPL compiles, never
  interprets.** No `solve()` loop, no clause walk in `plg-repl`. v1's
  `prlg-repl` rode an in-process `Solver` — we copy seqr's
  compile-and-exec path instead. CI guard: the crate links `plg-compiler`
  + spawns binaries and contains no solver/clause-walk symbol.
- **No new runtime semantics.** Interactive "assert" is **not** `assert/1`
  (still unimplemented) — it is *append-to-session-buffer + recompile*,
  producing a fresh immutable binary each time. The KB is never mutated
  in place; this stays true to the immutable-binary architecture.
- **Reuse the shared sources.** Completion vocabulary comes from
  `plg-shared` (`BUILTINS` + `STDLIB_PL`); parse + the
  undefined-predicate lint come from `plg-frontend` — the same sources
  the LSP uses. No third copy of the builtin table, no shadow parser.

## Approach

TUI via `ratatui`/`crossterm` with **`vim-line`** (patch-seq's zero-dep
vim-motion line crate) for editing — the stack the user picked. `plgr`
links `plg-compiler` directly (as seqr links `seq-compiler`) for
in-process parse/codegen and reuses its clang driver to build the temp
binary; runs exec that binary as a subprocess.

`plgr` keeps an **ordered session source buffer** (clauses + directives)
and splits input into two classes — this split is the design's core,
because it makes the common case free:

1. **Program edits** — a clause/rule/directive, or `:load file.pl`.
   Append to the buffer, recompile the buffer to a **temp native binary**
   (the same path `plgc build` uses; link errors keep the previous binary
   and report). This is the only thing that pays the clang cost.
2. **Queries** — a `?- goal.` line. The program is unchanged, so **do not
   recompile**: invoke the *current* temp binary with
   `--query "goal" --format json` (the existing wire contract).
   Backtracking (`;`/next) pages a batch fetched with `--limit`; "more"
   past the batch re-invokes with a higher limit (binaries are stateless
   across processes — the one inelegance, noted).

Runs use seqr's `run_with_timeout` approach: spawn with `stdin` nulled,
poll, kill on a bounded `PLG_REPL_TIMEOUT` (default ~10s) so a divergent
query can't hang the REPL (belt-and-suspenders with the runtime step
limit). Parse/lint errors render **inline before any compile** (instant,
in-process via `plg-frontend`); compile/link errors and runtime errors
(exit 3) / query-parse errors (exit 2) surface in an output pane per the
wire contract. Multi-line clause entry accumulates across Enter until a
clause-terminating `.` (SWI convention). Meta-commands: `:load` `:list`
`:edit` (`$EDITOR` via `shlex`) `:reset` `:save` `:help` `:quit`. History
persisted under XDG/`home` like seqr.

**Presentation model (landed).** Like seqr — a ratatui app that
*presents* as a line-oriented terminal REPL: a borderless transcript that
flows top-down, the live input inline behind a prompt, results below, no
boxes (helper/IR panes are a later, on-demand addition). The prompt is
**`plg> `, deliberately neutral** — not `?-`. The same prompt accepts both
clause definitions and `?-` queries, so a `?-` prompt would mislead for
definitions and double up when the user types `?- goal.`; the user types
their own `?-`, echoed as `plg> ?- goal.`. Multi-line clauses continue
under `|  `. The vi-mode shows dimly bottom-right only when not in insert
mode.

Phase-2 candidates (not blocking): an LLVM-IR visualization pane (seqr
has one — on-brand for the correctness ethos), LSP-client completions,
and cross-session caching of compiled binaries.

## Domain Events

- **Clause/directive entered or `:load`** → buffer append → recompile to
  temp binary. Must follow: on success swap the "current binary" and
  refresh completion vocabulary (new predicate names); on error reject
  the entry, keep the prior binary, show the error.
- **Query entered** → run current binary `--query` → page solutions; `;`
  advances. Nothing mutates. Empty program still answers builtin/stdlib
  goals (compile the empty buffer once on first need).
- **`:reset`** → clear buffer, drop temp binary. **Exit** → persist
  history; `tempfile` cleans temp artifacts.

## Checkpoints

1. `plgr` launches; vim-line normal/insert editing works; history
   survives restart.
2. Enter `parent(tom,bob).` then `?- parent(tom,X).` → `X = bob`; `;`
   reports no-more. **Instrument that the fact triggered a recompile and
   the query did NOT** (queries never shell clang) — the core claim.
3. Multi-line rule entry until `.`; a parse error renders inline with no
   compile attempted.
4. `:load examples/family.pl`, then query it.
5. A divergent/expensive query is killed by the timeout; the REPL stays
   alive and responsive.
6. **Rule-3 guard (CI):** `plg-repl` links `plg-compiler` + execs
   binaries; no `solve`/clause-walk symbol exists in the crate.
7. Completion offers `plg-shared` builtins + stdlib + buffer predicates.
