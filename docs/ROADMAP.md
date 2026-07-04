# Roadmap

Goal: feature parity with patch-prolog's documented ISO subset, as a
true standalone compiler. Each milestone has a hard verification gate.

## M0 — Scaffold ✅ (2026-06-04)

Workspace, four crates, justfile, Forgejo CI, docs skeleton, examples
ported. The build.rs runtime-embed chain works end to end.

## M1 — Frontend ✅ (2026-06-04)

Tokenizer/parser/term/error in `plg-shared` +
`plg-frontend`, split into focused modules, with all 48 frontend
unit tests (15 tokenizer + 27 parser + 6 error).
`plgc check examples/deps.pl` parses and reports `file:line:col`.

## M2 — Minimal end-to-end compilation ✅ (2026-06-04)

Runtime: Machine, cell heap, trail, generic unify, choice points,
registry, minimal goal parser, text wire-format output, exit codes, solve
driver. Codegen: facts, rules with conjunctive bodies, multi-clause
predicates with choice points, recursion. `plgc build` + `plgc run`
(compile-temp-and-exec, never interprets).

Gate results:
- `./fam --query "parent(tom, X)"` etc. produce the expected solutions
  (including solution order, limit/exhausted semantics,
  existence/step-limit error text, exit codes).
- 5,000-deep `ancestor` recursion (≈12.5M backtracking ops) under a
  256KB stack — the musttail CPS design holds even at `-O0`.
- Compiled binaries link only libc/libm/libgcc_s; no Rust anywhere.
- Footprint: hello-world 4.2M with DWARF (`-g`), **432K stripped**
  (under the patch-seq ~730K bar; M5 decides the `-g` default).
- Known M2 limits: builtins/control reserved (compile fine, raise a
  clear runtime error when reached — `reserved_builtin()` in
  codegen/clause.rs shrinks as M3/M4 land); integer literals limited to
  i61 immediates (boxing in M4); runtime query parser handles
  atoms/vars/ints/compounds/lists/conjunctions (operators arrive with
  their builtins).

## M3 — Control and arithmetic ✅ (2026-06-04)

Disjunction, if-then-else, cut (transparent in `;`/`->`/`,`, local in
call-like contexts — both tested), negation-as-failure, `once/1`,
`=`/`\=`/`==`/`\==`/`@`-comparisons/`compare/3`, full arithmetic
(`is/2`, comparisons; floored mod, checked overflow, NaN rejection),
first-argument indexing
as IR `switch`, runtime query parser grown to the standard operator set
plus floats, query-level control walker (goal TERMS only — never
clauses).

Gate results:
- 14-test M3 integration suite asserts fixed bytes (cut,
  disjunction order, NAF, ITE, arithmetic values and error messages).
- A broad property sweep over the same corpus: all matches except the
  documented ISO cut divergence (below) and variable-numbering noise.
- Indexing: the 5k-fact chain query dropped from ~10s (M2 linear scan)
  to 3ms; keyed single-candidate dispatch pushes NO choice point.
- Cut under 512KB stack across 1500-deep recursion: constant C stack.
- **ISO cut rule**: `!` inside `;` cuts the whole clause per
  ISO 7.8.4 — see ISO_COMPLIANCE.md "Cut".
- linting.pl (needs `\+`) now compiles.

## M4 — Builtin parity and errors ✅ (2026-06-04)

Full builtin vocabulary: type checks, `functor/3`,
`arg/3`, `=..`, `copy_term/2`, atom/number conversions, `msort/sort`,
`succ/plus`, `unify_with_occurs_check/2`, `write/writeln/nl`,
`findall/3`, `between/3` (nondet, uniform predicate signature),
`call/N` + variable-goal metacall, `catch/3`/`throw/1`. Structured
error balls (relocatable copies surviving heap rewind) with byte-stable
rendering; cut stops at catch frames; step limit stays
uncatchable. Boxed i64 (TAG_BIG) lifts the i61 immediate limit.
stdlib.pl embedded and compiled into every binary. The runtime
goal walker gained proper cut barriers (qbarrier, snapshotted in every
continuation frame) and the operator surface (`:` xfy, prefix `+`/`\`,
standalone operator atoms, cycle-safe rendering of `X = f(X)`).

Gate results:
- The integration corpus — 89 grouped tests (~200 cases; the rest are
  in-process library-API tests with no wire analog) — passes with ZERO
  ignores. Six surfaced divergences were triaged as bugs (per the
  ISO-over-bug-compat rule) and fixed: walker cut barriers, cyclic-term
  render crash, between/3 i64 boxing, query-parser operator surface.
- Semantics are pinned by the integration corpus itself.
- 123 runtime + 48 frontend unit tests; full `just ci` green.

## M5 — Polish ✅ (2026-06-04)

- **Binary size**: release links strip the runtime archive's DWARF
  (`-Wl,--strip-debug`); hello-world dropped 4.4M → **676K**
  (`--debug` keeps DWARF + `-O0`). Guarded by
  `tests/binary_size.rs`: a 1.3M ceiling plus a Linux `ldd` check
  enforcing the standalone contract (libc/libm/libgcc/loader only) —
  both in `just ci` via `check-binary-contents`.
- **Script mode**: `#!/usr/bin/env plgc` shebang works — `plgc
  prog.pl [args…]` compiles to a temp binary and execs it (the
  parser blanks a leading `#!` line, preserving line numbers).
- Shell completions (`plgc completions <shell>`) verified; docs
  corrected (COMPILATION_MODEL's compiled-vs-runtime line now matches
  the implementation); footprint recorded here.

Footprint record (x86_64-linux, clang 19, 2026-06-04): hello-world
676K default · 4.4M `--debug` · binary answers `--query` with only
base system libraries loaded.

## M6 — REPL (`plgr`) ✅ (2026-06-16)

Interactive loop that drives the compiler instead of interpreting —
user guide: [REPL Guide](repl-guide.md). TUI (`ratatui`/`crossterm` + patch-seq's
`vim-line`), modelled on patch-seq's `seqr`. Keeps an ordered session
source buffer; **clause/`:load` edits recompile the buffer to a temp
native binary, `?-` queries re-invoke the current binary via `--query`
and page solutions for `;`** — so the common case (querying) never pays
clang cost and nothing is ever interpreted.
Reuses `plg-shared` (`BUILTINS`/`STDLIB_PL`) and `plg-frontend`
(parse + undefined-pred lint) — the same sources the LSP uses.

Gate results:
- A clause entry triggers a recompile; a subsequent query does NOT shell
  clang (the core efficiency claim, instrumented).
- `depends_on(app,auth).` then `?- depends_on(app,X).` → `X = auth`;
  multi-line rule entry until `.`; `:load examples/deps.pl` then query it.
- Divergent query killed by `PLG_REPL_TIMEOUT`; REPL stays alive.
- Rule-3 guard: `plg-repl` shells out to `plgc` and links no solver
  runtime — a CI test asserts no `plg-runtime` dependency and no
  `solve`/clause-walk symbol. (In-process `plg-compiler` linking remains
  the design target.)

## M7 — Language server (`plgl`) ✅ (2026-06-10)

Editor support built on the same `plg-frontend`/`plg-shared` sources as
the compiler and REPL — never links the runtime. Diagnostics (parse
errors + undefined-predicate warnings), completion (stdlib/builtins/user
predicates, prefix-filtered, user shadows stdlib), hover, and
goto-definition. User guide: [LSP Guide](lsp-guide.md). (Built alongside
the compiler, before the REPL; recorded here retroactively — the
milestone numbers are a feature log, not a strict timeline.)

Gate results:
- Per-feature unit tests in `crates/lsp` (diagnostic positioning,
  completion sourcing/filtering/shadowing, definition lookup).
- The undefined-predicate lint shares `plg-shared::BUILTINS` and the
  parser with `plgc` — one vocabulary, no shadow parser.

## M8 — Source spans & error provenance ✅ (2026-06-18)

Source positions made a first-class property of the AST and carried
through to both compile-time diagnostics and runtime error text, in
three layers:
- **Frontend**: `Span`/`Spanned`/`SourceMap` in `plg-shared`; structured
  `ParseError { message, span }` (the `... at line N col M` trailer
  dropped); the tokenizer carries byte offsets.
- **LSP / `plgc check`**: diagnostics map spans → ranges — precise
  parse-error underlines and call-site-precise undefined-predicate
  squiggles (the buffer-scan / string-trailer hacks deleted); `plgc
  check` renders `file:line:col` from the span + `SourceMap`.
- **Runtime**: compiled errors (existence, arithmetic, and type-checking
  builtins) append ` at file:line:col` from a `.rodata` side-table
  handed over at init, set per-raise via an RAII `ErrorSiteGuard`.

Gate results:
- Integration tests pin the suffix per error class; the ISO `error/2`
  ball and error messages are unchanged (query-side / stdlib
  raises use the `NO_SITE` sentinel → no suffix); golden IR unchanged.
- `throw/1` intentionally excluded (a user-thrown ball isn't a system
  error).

## M9 — Fact-table compilation ✅ (2026-06-18)

The first post-parity feature: a predicate whose clauses are all bodyless
facts with ground head args compiles to one `.rodata` data table + a
generic runtime lookup, instead of one clause function each — same
semantics, same single immutable binary, near-instant rebuilds at 100k+
facts. Serves the production thesis (immutable binary as the only prod
artifact; fact churn = deploy cadence). Built in three staged PRs:
- **A**: immediate (atom/int) columns + the generic CPS lookup
  (`plg_rt_fact_first`/`_next`); delivery to the continuation is a
  `musttail` in generated IR (constant C stack through recursive fact
  predicates).
- **B**: a first-argument index (a `.rodata` array of row indices sorted
  by column 0) — bound-key queries binary-search the matching range.
- **C**: ground compound / list / float / big-int columns serialized into
  a per-predicate `.rodata` blob (the `copyterm`/`TermBuf` cell format),
  restored onto the heap on lookup. The word/cell ABI is single-sourced
  in `plg-shared::cell` so codegen and runtime can't drift.

Gate results:
- Equivalence: the integration corpus (now including compound facts)
  passes; the same facts compiled fact-table
  vs per-clause give identical query output, order, and `--limit`.
- Stack safety: 2000-deep recursion *through* a fact table holds under a
  512KB stack (`deep_recursion_runs_in_constant_c_stack`).
- Footprint: one table (+ index, + blob) per predicate, not N functions;
  100k facts compile in ~1s; `binary_size` gate green.
- Coverage: integration tests per column kind + `findall`/`call`
  re-entry + undefined→`existence_error`; golden tests pin table/index/
  blob emission.
- Deferred (recorded in the design doc): multi-argument indexing,
  float-keyed indexing, compound-column-0 interning, the `unsafe`
  bounds-elision seam (gated on a profile).

## M10 — WASM Tier 1 (`wasm32-wasi`) ✅ (2026-06-20)

`plgc build --target wasm32-wasi` emits a standalone WASI module from the same
LLVM IR as native, preserving the `--query` wire contract verbatim. The engine
runtime compiles to `wasm32-wasip1` unchanged; `llc -mattr=+tail-call` lowers
the `musttail` chains to `return_call`/`return_call_indirect`, so recursion and
backtracking stay constant-stack on wasm. Built with the Rust toolchain's own
LLVM tools (`llc`/`wasm-ld`) + the wasm target's self-contained wasi-libc — no
wasi-sdk. The wasm runtime archive is embedded behind a `wasm` cargo feature,
so the default `cargo install plgc` is byte-for-byte unchanged. User guide:
[WASM Target](wasm-target.md).

Gate results:
- Stack safety (Checkpoint 0): 5,000,000-deep recursion runs in bounded stack
  under wasmtime; a missing `+tail-call` is a loud `llc` build error, never a
  silent overflow.
- Equivalence: a compiled example answers `--query` (atoms, ints, `findall`)
  byte-identically to the native build (`just wasm-smoke`).
- Footprint asymmetry preserved: the toolchain cost is build-side only; the
  shipped `.wasm` runs with only a wasm engine present.
- Deferred: CI wiring (local-only for now — `just wasm-smoke`), and `plgc run
  --target wasm32-wasi` via a bundled engine. Tier 2 (V8 isolates / Cloudflare
  Workers) remains design-only at this milestone.

## M11 — WASM Tier 2 (`--target worker`) ✅ (2026-06-20)

`plgc build --target worker` emits a **reactor** module for
`wasm32-unknown-unknown` — no `main`, no WASI: it exports `plg_init` plus a
linear-memory buffer ABI a V8 isolate (Cloudflare Workers / `workerd`) calls per
request. The same LLVM IR as native and Tier 1 is retargeted, so a query is a
warm in-isolate call answering byte-identically to native. The I/O-free query
core (`runtime/src/core.rs`) is factored out and shared by the WASI shell and
the reactor — one bson wire shape (JSON derived host-side), no duplication. The reactor takes per-request
solution/step/metacall-depth limits over the ABI, captures `write/1` output
losslessly (no stdout in an isolate), and reuses one Machine per isolate via
`Machine::reset_per_query`. The one `wasm` feature now embeds both wasm archives
(`wasm32-wasip1` + `wasm32-unknown-unknown`); `plgc` also drops overrideable
deploy glue (`worker.js` + `reactor.mjs` + `wrangler.toml` + `config.capnp`)
next to the `.wasm`. User guide: [WASM Worker](wasm-worker.md).

Gate results:
- HTTP equivalence on **`workerd`** (the real Workers runtime, V8 isolate):
  `--query` over GET/POST answers byte-identically to native, **including the
  `existence_error` path** (`just wasm-worker-serve` + curl).
- Constant stack on V8: a 1,000,000-deep `call/1` returns in the isolate via
  `return_call` — **automated** (`just wasm-reactor-smoke`, run under Node's V8).
- Self-contained module: zero imports, exactly the four host exports
  (`plg_init`/`plg_rt_run_query`/`plg_rt_alloc`/`plg_rt_free`) + `memory`, no
  `_start`; ~1.7 MB raw for the `deps` example.
- Default `cargo install plgc` byte-for-byte unchanged — the reactor archive and
  glue live behind the `wasm` feature.
- Single-sourced glue: the buffer-ABI marshalling lives once in the emitted
  `reactor.mjs`, imported by both the deployed `worker.js` and the smoke test,
  so the tested code is the shipped code.
- CI: both tiers' gates run in a **separate wasm workflow**
  (`just wasm-ci` = `wasm-lint` + `wasm-smoke` + `wasm-reactor-smoke`), so a
  runner missing the wasm toolchain never breaks the core `just ci` — closing
  M10's deferred CI item for Tier 1 too.

## Future (explicitly out of scope)

- Bundled backend ("the Zig route") only if compiles must happen ON
  hardened machines — same ADR. (Runtime `--facts` loading was
  considered and REJECTED for the prod shape: it reopens the
  mutable-file surface the immutable-binary architecture closes.)
- WAM-style instruction-level codegen; inline head-unification and
  integer-arithmetic fast paths (performance escape hatches)
- Copying GC for long-running determinate queries
- REPL (`plgr`) enhancements: an LLVM-IR visualization pane, LSP-client-backed
  completion, and cross-session caching of compiled binaries (all optional)
- `assert/retract` beyond the silent-fail dynamic contract
- Native cross-compilation to other host arches. (WASM Tier 2 — V8 isolates /
  Cloudflare Workers — shipped in M11.)
- bagof/setof, DCG, modules, `op/3` (scope decisions — the language
  definition excludes them deliberately)
