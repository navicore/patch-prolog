# Fact-table compilation

**Status: design (2026-06-17).** The first post-parity feature (ROADMAP
"Future" → "Likely the first post-parity feature").

## Intent

Compile a predicate whose clauses are **all ground facts** (no body, no
variables in the head) to a single `.rodata` data table + one generic
lookup function, instead of one clause function per fact. Why: the
production thesis is *immutable binary as the only prod artifact; fact
churn = deploy cadence*, and at 100k+ ground facts the per-clause model
emits 100k functions — huge IR, slow `clang`, big binary. Data scales
where code doesn't: near-instant rebuilds, smaller binaries, same
semantics and same single immutable artifact.

## The `unsafe` question (answered)

Fact-table is a **safe feature**; it does not justify new `unsafe`:
- **Correctness:** ground args are immediates (`ATOM`/`INT`/`FLT` words)
  or, for ground compounds/lists, a serialized blob restored to the heap
  via the existing `copyterm`/`TermBuf` path. Reading the static table is
  the *existing* FFI init-handoff `unsafe` (`slice::from_raw_parts` at the
  C boundary, as for `@plg_registry`/`@plg_srcmap`) — not a new class.
- **Compile-time win:** pure codegen (emit data, not code) — safe.
- **Only legit `unsafe`, and it is DEFERRED:** an "enumerate all rows"
  query at scale scans the whole table and unifies each row. Bounds
  elision (`get_unchecked`) and zero-copy in-place unification against
  `.rodata` could pay *there* — a true runtime hot path, so it clears the
  "runtime performance" bar in principle. But: gated on a profile proving
  the scan is the bottleneck (a sorted/indexed safe baseline makes the
  common bound-key query O(log n), so most queries never scan), and
  verified by the differential corpus + ASan/fuzz on the native binary
  (Miri can't see clang output). Ships safe first; unsafe is a recorded
  follow-up, never speculative.

## Constraints

- **Semantics identical** to the same facts compiled per-clause: solution
  order = program order, backtracking, `--limit`/`exhausted`, first-arg
  indexing, `call/N` + `findall/3` re-entry — all unchanged. The wire
  contract and exit codes do not change.
- **Opt-in by shape, per predicate.** Any clause with a body or a
  non-ground head arg disqualifies the predicate → it falls back to the
  current per-clause codegen. Mixed programs must compile.
- **Footprint:** the table must be *smaller* than the per-clause
  equivalent; hello-world (no fact predicates) unchanged; `binary_size`
  gate stays green. `plg-shared`/`plg-runtime` stay zero-dep; any index
  builder lives compiler-side.
- **Out of scope:** `:- dynamic` (mutable; the immutable-binary thesis
  rejects it), multi-argument indexing (first-arg only for v1 — the
  `iddqd`/multi-index question is a *later* compiler-side optimization),
  runtime `--facts` loading (rejected), the deferred `unsafe` seam.

## Approach

- **Detection (codegen):** a predicate is a fact table iff every clause is
  a fact and every head arg is ground. Else, unchanged path.
- **Emit (codegen):** one `.rodata` global per fact predicate — N rows of
  arity columns; immediates inline, ground compounds as offsets into a
  serialized-term section — plus a first-arg index (rows sorted by col 0,
  binary-searchable) when col 0 is discriminating. Register `(functor,
  arity)` to a tiny generated entry fn that calls the runtime lookup with
  `&TABLE` + row count.
- **Lookup (runtime, `plg_rt_fact_lookup`):** given the query args,
  select the candidate row range (binary search when arg0 is bound, full
  range otherwise), and for each row unify its columns with the args
  (immediates directly; compounds restored via `copyterm`), delivering
  each success through the CPS continuation and pushing a choice point to
  resume at the next row — the multi-clause backtracking shape, in data.

## Domain Events

- **Produced:** a `.rodata` fact table + index per qualifying predicate;
  a registry entry pointing at the generic lookup; compile time for a
  fact predicate drops from O(facts) functions to O(1) data emission.
- **Consumed:** `plg_rt_fact_lookup` reads the table (init-handoff /
  emitted pointer), enumerates matching rows, unifies (restoring compound
  columns), yields solutions via the continuation with choice points —
  observably identical to per-clause facts. `call/N`/`findall/3` re-enter
  it through the registry like any predicate.
- **Must follow:** differential corpus gains a fact-heavy predicate;
  `binary_size` confirms data < code; a 100k-fact compile-time benchmark
  confirms the rebuild claim.

## Checkpoints

1. **Equivalence:** the *same* facts compiled fact-table vs per-clause
   give byte-identical query output (order, count, exhausted,
   backtracking, `--limit`) — a test compiles both and diffs.
2. **Compile time:** a 100k-fact predicate compiles in roughly constant
   data-emission time; record IR size + `clang` time before/after.
   *Measured (Stage A, 2026-06-18):* 100k 2-column facts emit one `.rodata`
   table + two functions (not 100k functions) and compile in ~1.0s total
   (`plgc` + `clang`), 4.5M binary — the **compile-time / footprint** win.
   *Stage B (2026-06-18):* a first-arg index (a `.rodata` array of row indices
   sorted by column 0) gives bound-key queries an O(log n) binary search to
   the matching row range; unbound or non-immediate first args still full-scan,
   identical to Stage A. The index adds one word per row.
3. **Footprint:** fact-table binary < per-clause equivalent; hello-world
   unchanged; `binary_size` + `ldd` gates green.
4. **Mixed + re-entry:** a program mixing fact and rule predicates
   compiles and runs; `call`/`findall` over a fact predicate work; an
   undefined predicate still raises `existence_error`.
5. **`unsafe` seam (deferred):** only if checkpoint-2/3 profiling shows
   the all-rows scan is the bottleneck — then an `unsafe` bounds-elision /
   zero-copy pass, gated on a measured win + differential + ASan/fuzz.
