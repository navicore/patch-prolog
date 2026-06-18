# Roadmap

Goal: feature parity with patch-prolog's documented ISO subset, as a
true standalone compiler. Each milestone has a hard verification gate.

## M0 — Scaffold ✅ (2026-06-04)

Workspace, four crates, justfile, Forgejo CI, docs skeleton, examples
ported. The build.rs runtime-embed chain works end to end.

## M1 — Frontend port ✅ (2026-06-04)

Ported v1's tokenizer/parser/term/error into `plg-shared` +
`plg-frontend`, split into focused modules, with all 48 v1 frontend
unit tests (15 tokenizer + 27 parser + 6 error).
`plgc check examples/deps.pl` parses and reports `file:line:col`.

## M2 — Minimal end-to-end compilation ✅ (2026-06-04)

Runtime: Machine, cell heap, trail, generic unify, choice points,
registry, minimal goal parser, v1 wire-format output, exit codes, solve
driver. Codegen: facts, rules with conjunctive bodies, multi-clause
predicates with choice points, recursion. `plgc build` + `plgc run`
(compile-temp-and-exec, never interprets).

Gate results:
- `./fam --query "parent(tom, X)"` etc. byte-identical to the v1
  interpreter (including solution order, limit/exhausted semantics,
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
`=`/`\=`/`==`/`\==`/`@`-comparisons/`compare/3`, full v1 arithmetic
(`is/2`, comparisons; floored mod, checked overflow, NaN rejection —
error strings oracle-captured byte-for-byte), first-argument indexing
as IR `switch`, runtime query parser grown to the standard operator set
plus floats, query-level control walker (goal TERMS only — never
clauses).

Gate results:
- 14-test M3 integration suite asserts oracle-captured bytes (cut,
  disjunction order, NAF, ITE, arithmetic values and error messages).
- Adversarial diff sweep vs the v1 interpreter: all matches except the
  documented ISO cut divergence (below) and variable-numbering noise.
- Indexing: the 5k-fact chain query dropped from ~10s (M2 linear scan)
  to 3ms; keyed single-candidate dispatch pushes NO choice point.
- Cut under 512KB stack across 1500-deep recursion: constant C stack.
- **Deliberate v1 divergence**: v1 treated `!` as opaque inside `;`
  (non-ISO, undocumented). plgc follows ISO 7.8.4 — see
  ISO_COMPLIANCE.md "Cut".
- linting.pl (needs `\+`) now compiles and matches the oracle.

## M4 — Builtin parity and errors ✅ (2026-06-04)

Full v1 builtin vocabulary (and ONLY it — non-v1 names like `atomic`
raise existence_error exactly like v1): type checks, `functor/3`,
`arg/3`, `=..`, `copy_term/2`, atom/number conversions, `msort/sort`,
`succ/plus`, `unify_with_occurs_check/2`, `write/writeln/nl`,
`findall/3`, `between/3` (nondet, uniform predicate signature),
`call/N` + variable-goal metacall, `catch/3`/`throw/1`. Structured
error balls (relocatable copies surviving heap rewind) with v1's
byte-identical rendering; cut stops at catch frames; step limit stays
uncatchable. Boxed i64 (TAG_BIG) lifts the i61 immediate limit. v1's
stdlib.pl embedded verbatim and compiled into every binary. The runtime
goal walker gained proper cut barriers (qbarrier, snapshotted in every
continuation frame) and v1's operator surface (`:` xfy, prefix `+`/`\`,
standalone operator atoms, cycle-safe rendering of `X = f(X)`).

Gate results:
- The ported v1 corpus — 89 grouped tests representing ~200 of v1's 248
  (the rest are in-process library-API tests with no wire analog;
  skip list documented in tests/v1_errors.rs) — passes with ZERO
  ignores. All six divergences the port surfaced were triaged as plgc
  bugs (per the ISO-over-bug-compat rule, checked against the LIVE
  oracle) and fixed: walker cut barriers, cyclic-term render crash,
  between/3 i64 boxing, query-parser operator surface.
- Permanent 57-goal differential corpus (`just diff-test`,
  auto-skipped in CI) matches the oracle byte-for-byte modulo variable
  numbering. The oracle dependency is now optional — semantics are
  pinned by the ported corpus itself.
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
clang cost and nothing is ever interpreted (LESSONS_FROM_V1 rule 3).
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
  parser with `plgc` — one vocabulary, no shadow parser (the v1 rule).

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
  ball and byte-exact v1 messages are unchanged (query-side / stdlib
  raises use the `NO_SITE` sentinel → no suffix); golden IR unchanged.
- `just diff-test` strips the suffix before comparing to the v1 oracle,
  so the differential corpus still matches byte-for-byte.
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
- Equivalence: the differential corpus (now including compound facts)
  is byte-identical to the v1 oracle; the same facts compiled fact-table
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
- Cross-compilation (`--target`)
- bagof/setof, DCG, modules, `op/3` (v1 scope decisions — the language
  definition excludes them deliberately)
