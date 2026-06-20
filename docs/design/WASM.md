# Design: WASM target (edge / scale-to-zero)

Status: **exploratory** — a real intended track, not yet a milestone. Gated
behind one make-or-break risk (see Checkpoint 0). Sibling of `INVOCATION.md`:
WASM is the *distribution / scale-to-zero* axis, distinct from native per-node
throughput.

## Intent

Emit a **wasm module** from the same compiler so a compiled program runs where a
native ELF can't: edge isolates (Cloudflare Workers), WASI runtimes
(wasmtime/wasmer/Spin), browsers later. The payoff is **global low-latency,
scale-to-zero, fixed-knowledge-base** querying — exactly the workload our
immutable-binary thesis already assumes (frozen artifact, no mutable per-request
state, scale by replication not coordination). The edge-isolate model and our
architecture are unusually well-aligned: both freeze the program and replicate it.

## Strategic fit: wasm-readiness is commodity; our *shape* is the differentiator

Emitting wasm is a checkbox the whole LLVM-native world checks — by itself it is
no advantage. The advantage is *fit*. Against the real comparison set (other ways
to put logic programming in a stack — classic Prolog VMs, embedded logic
runtimes), "just recompile to wasm" is false for everyone but us:

- **Size.** Edge isolates cap module size at single-digit MB; a Prolog VM is
  megabytes before the program. Our ~676K no-VM artifact clears the gate that
  excludes them. In containers size is cosmetic; at the edge it is admission
  control.
- **Statelessness.** The isolate model is hostile to mutable, long-lived
  runtimes. Incumbents fit only by amputating their mutable database, threads,
  and FFI. We have nothing to amputate — stateless by thesis, so we fit by
  construction.

The edge revalues exactly the four properties we already hold — **small,
stateless, immutable, AOT** — and devalues the rich-mutable-runtime design that
is the incumbents' strength elsewhere. wasm is the *delivery vehicle that exposes
that shape to the edge*, not the moat itself.

One concrete, ongoing consequence: the value is a **latent option, kept by not
regressing those four properties.** Any future feature that adds mutable runtime
state or bloats past the isolate size cap *spends* it — an independent reason
`assert/retract` and runtime `--facts` stay rejected. The discipline is the asset.

The opportunity is **present, not speculative**: a vast, cheap edge fabric
already exists — Cloudflare especially — deployed ahead of demand and currently
priced near-free. The job is to be the artifact optimally shaped to run on
infrastructure that is already built and waiting. So: **target the portable
artifact, keep native primary (containers are here today), and protect the
shape.** WASM is additive — ride the substrate that already exists; don't couple
the roadmap to anyone's platform-war timeline.

## Why this is feasible (and the one thing that could kill it)

- **Cell format is target-width independent.** REF payloads are `usize` *indices*
  into `Machine.heap: Vec<Word>`; trail/CP marks are vector lengths; words stay
  `i64`. wasm32's 32-bit linear memory does not break the tagged-word encoding.
- **IR is nearly target-neutral.** `target_triple()` is the only host assumption;
  plumb a `--target` and pass clang `-target wasm32-…`. Localized change.
- **THE RISK — `musttail` lowering, not tail calls per se.** wasm *has* tail
  calls (`return_call`/`return_call_indirect`, feature `+tail-call`), shipped and
  on in V8/Workers and wasmtime; and our uniform CPS signature makes *every*
  transfer a valid `musttail` (native already builds — `musttail` is a hard
  guarantee, so if any transfer weren't tail-valid, native would already error).
  The open question is narrow: does our LLVM wasm backend lower that `musttail`
  to `return_call(_indirect)` with the feature wired through. It **fails loud** —
  missing flag → LLVM *compile error*; engine without the feature → module
  *rejected at load* — never a silent prod stack-overflow. (The only silent path
  is self-downgrading `musttail`→`tail`; golden-IR tests pin `musttail`, and a
  trampoline fallback *is* that downgrade — rejected.) Backtracking and `call/1`
  dispatch go through *indirect* calls, so the half we truly depend on is
  `return_call_indirect`. Retired FIRST, before any porting (Checkpoint 0).

## Approach — two tiers

**Tier 1 — `wasm32-wasi` standalone module.** Runtime compiles to
`wasm32-wasip1` (std mostly intact: args, stdio, exit). The existing
`main`/`--query`/JSON-stdout model survives nearly as-is; runs on
wasmtime/wasmer/Spin. Cheapest proof; retires Checkpoint 0; preserves the wire
contract verbatim. This is the first deliverable.

**Tier 2 — Cloudflare Workers (`wasm32-unknown-unknown` + JS glue).** No process,
no argv, no stdio — you *export* a function to a JS isolate. Add an entry
`run_query(ptr,len) -> ptr` operating on **linear-memory buffers** (query bytes
in, JSON bytes out); a thin JS Worker does HTTP and calls it. This requires
factoring the **I/O-free query core** (`Machine` + `parse_query` + `solve`) out
of the CLI shell — *the same extraction `INVOCATION.md`'s resident mode wants*,
so the two tracks share it. This is the "sing globally" tier: warm isolate, query
= a wasm call (µs, no fork, atom-map built once at module init).

## Constraints / what must not break

- **`musttail` is load-bearing** — never ship a wasm target that silently
  trampolines or overflows. Tail calls verified end-to-end or it doesn't ship.
- **Footprint rule, harder.** Workers cap module size (≈1–10 MB incl. wasm). A
  100k-fact `.rodata` blob can exceed it → large KBs stay on WASI/containers;
  small/medium go to Workers. Record the size cliff.
- **Platform CPU/wall limits.** Workers kill long requests independent of our
  step limit (default 10k); the step limit must be set conservatively per tier so
  *we* bound the query before the platform does.
- **Immutable-binary thesis preserved** — a Worker isolate is a frozen module; no
  runtime clause mutation. (Alignment, not tension — don't break it.)
- **Native stays primary.** WASM is an additional emit target, not a pivot.
- **Wire contract** — Tier 1 preserves `--query`/JSON exactly; Tier 2 preserves
  the JSON *shape* over the buffer transport.

## Out of scope

- Championing/replacing k8s (explicit non-goal — ride, don't evangelize).
- The wasm Component Model / WIT interfaces (later; start with raw wasm + minimal
  glue).
- Browser target (possible after Workers/WASI).
- Mutable KB / `assert` (still rejected, everywhere).

## Domain events

- *Build time*: `.pl → wasm module` (new emit target). Tier 2 also emits a
  `run_query` export + generated JS Worker glue.
- *Runtime (WASI)*: argv query in → JSON stdout out (unchanged from native).
- *Runtime (Workers)*: HTTP request → JS Worker → `run_query(mem buffer)` →
  solutions buffer → HTTP response. New event: **query-as-memory-call inside a
  warm, globally-replicated isolate** replacing process-exec.

## Checkpoints

0. **GATE — indirect tail calls on the target engine.** A 100k-deep recursion
   *through backtracking and registry dispatch* (so it exercises
   `return_call_indirect`, not just direct self-calls) runs in bounded stack on
   wasmtime, then Workers' V8, with `+tail-call`. The expected failure is a loud
   build/load error, not a runtime trap; if it can't be wired, STOP — rest is moot.
1. **Tier 1**: `deps.pl` compiled to `wasm32-wasi` answers `--query` on wasmtime
   **byte-identical** to native.
2. **Tier 2**: the same program as a Cloudflare Worker answers an HTTP request at
   the edge; warm-isolate query latency measured (sub-ms target after warm).
3. **Size/cold-start matrix**: module size + cold-start across small / 10k / 100k-
   fact programs — defines which KB sizes fit Workers vs need WASI/containers.

## Recommendation

Keep WASM as a real, scoped-later milestone, sequenced behind Checkpoint 0. Do
Tier 1 first (cheap, retires the gate, immediately useful on WASI platforms);
Tier 2 unlocks the global-edge story and shares the I/O-free-core refactor with
the resident-mode lever. Target the portable artifact and exploit the edge fabric
that already exists — the model is shaped to run on it today; native keeps the
container world. Protect the shape; that is the moat, not the wasm checkbox.

## Tier 2 — gate result & productization brief (2026-06-20)

The Tier 2 gate spike (throwaway: `crates/runtime/src/reactor.rs` + an IR `awk`
transform + a workerd Worker in `/tmp`) **passed on workerd** (real Workers
runtime, V8 isolate, over HTTP):

- No-WASI viable: the I/O-free `run_query` path never touches the
  stdout/argv/exit stubs `wasm32-unknown-unknown` provides.
- **`musttail` → `return_call` lowers and runs on V8 at depth** — a 1,000,000-deep
  `call/1` recursion returns in a V8 isolate (constant stack). The load-bearing
  finding: the PR #20/#24 IR investment carries to Workers with no rework.
- Buffer ABI round-trips byte-identical to native, including the
  `existence_error` path (so the JSON + error rendering need no parallel impl).
- Module size 1.65 MB raw (compresses well under the Workers budget).

Productization is now "engineering, no existential unknowns." Disposition: keep
`reactor.rs` as the seed; do **not** extract the I/O-free core until
productization (both call sites — the WASI shell and the reactor — exist now, so
the abstraction is shaped by two real consumers, not one and a guess).

Productization checklist (gate findings to carry forward):

1. **Allocator ABI rule.** Any host-freed buffer must be
   `alloc::alloc(Layout::from_size_align(len, 1))`, never `Vec::with_capacity(len)`:
   the host frees by *requested* length, so an actual-capacity > requested-length
   (which `Vec` may produce) corrupts the allocator. This bug is what made the
   spike's deep query abort; the next contributor will reach for `Vec` reflexively.
2. **Concurrency contract.** The single `static AtomicPtr` `MACHINE` is
   single-in-flight per isolate. V8 isolates are single-threaded, but one Worker
   can interleave async tasks. Pick: (a) document "one in-flight query per
   isolate" (simple, matches typical Worker use — recommended), or (b) move
   per-request state out of `MACHINE` and through the buffer ABI.
3. **`reset_per_query`.** The spike's `reset()` clears each per-query field by
   name; it silently knows the Machine's field set. Make it a
   `Machine::reset_per_query()` next to the field declarations so a future field
   added without reset coverage is a *local* question — the leak class only
   surfaces under sustained traffic, never in single-query smoke tests.
4. **`exhausted` from the limit.** The spike hard-codes `"exhausted":true` (no
   `--limit`). Productization takes the limit over the ABI and computes it like
   `entry.rs`: `limit.is_none_or(|l| count < l)`.
5. **Per-request step + metacall-depth limits** over the ABI, mirroring
   `PLG_MAX_STEPS` / `PLG_METACALL_DEPTH`. The depth one matters: a wasm engine's
   ~1 MB stack is smaller than native's ~8 MB (see Tier 1 docs).
6. **I/O-free core extraction.** `entry.rs` (≈ the `output_json`/`output_error`
   block) and `reactor.rs`'s `solutions_json`/`error_json` are nearly the same
   code with different output sinks. The shared core wants three knobs:
   limit-aware `exhausted`, error JSON shape, success JSON shape — an `io::Write`
   parameter is enough. Shared with INVOCATION.md's resident-mode lever.
7. **Packed return assumes wasm32.** `(len << 32) | ptr` is correct on the target
   but would need rethinking on wasm64 (an emerging target).
8. **`MACHINE` is never freed** — correct for cold-start-per-isolate. A teardown
   entry point is only needed if a live isolate must swap its embedded program.
9. **`# Safety` doc convention:** every `no_mangle` entry documents its safety
   except the genuinely-safe `plg_rt_alloc` — keep "no Safety doc = safe" true.
10. **`len == 0` sentinel convention:** `raw_alloc(0)` returns a dangling pointer
    and `plg_rt_free` no-ops on `len == 0`; the two halves agree by convention,
    not by API. Keep them paired.
