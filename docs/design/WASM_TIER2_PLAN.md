# WASM Tier 2 — productization plan

Status: **execution plan** (working doc; delete when Tier 2 ships). The *why*
and the gate findings live in `WASM.md` (Tier 2 section + productization brief);
this is the *how/do-this*, concrete enough to execute without the spike author's
session context.

## Where we are

- **Gate passed** on workerd (real Workers runtime, V8): a 1,000,000-deep
  `call/1` recursion runs in a V8 isolate (constant stack via `return_call`); the
  buffer ABI round-trips byte-identical to native incl. the `existence_error`
  path; module 1.65 MB raw. See `WASM.md` "Tier 2 — gate result".
- **In-tree seed**: `crates/runtime/src/reactor.rs` — the throwaway buffer ABI
  (`plg_init`/`plg_rt_set_machine`, `plg_rt_alloc`/`plg_rt_free`,
  `plg_rt_run_query` returning packed `(len<<32)|ptr`, `reset()`,
  `solutions_json`/`error_json`). Builds clean, dead-stripped from native.
- **Tier 1 shipped (M10)** is the pattern Tier 2 extends. Already exist and work:
  - `Target` enum + `--target` plumbing (`lib.rs`, `main.rs::parse_target`).
  - `codegen/program.rs` triple + entry-name switch (native `main` vs wasi
    `__main_argc_argv`).
  - `link.rs::link_wasm` + `wasm_toolchain()` (locates rustup `llc`/`rust-lld` +
    self-contained wasi-libc via `rustc --print sysroot`/`host-tuple`).
  - `build.rs` feature-gated archive embed; `Cargo.toml` `wasm` feature;
    `justfile` `build-runtime-wasm` / `install-wasm` / `wasm-smoke`.

## Decisions to make FIRST (they gate the work)

- **D1 — target name / CLI.** The Tier-2 artifact is a *reactor* (no `main`,
  exports), not a CLI. Don't overload `wasm32-wasi`. Recommend a distinct
  `--target worker` (or `wasm32-worker`). Triple under the hood stays
  `wasm32-unknown-unknown`.
- **D2 — Worker glue: emit vs template.** Does `plgc` emit `worker.js` +
  `wrangler.toml` (Cloudflare) / `config.capnp` (workerd) next to the `.wasm`, or
  ship a copyable template? Recommend **emit** a minimal, overrideable
  `worker.js` + `wrangler.toml` ("it just works"); keep the workerd `.capnp` as a
  local-test artifact.
- **D3 — concurrency contract** (finding #2). `MACHINE` is one-in-flight per
  isolate. Recommend **documenting** "one in-flight query per isolate" (matches
  typical Worker use) rather than threading per-request state through the ABI.
- **D4 — `write/1` semantics** (no stdout in an isolate). Recommend **capture**
  printed output into the result (lossless) over no-op/error.
- **D5 — feature shape.** Tier 1 `--features wasm` embeds the wasi archive.
  Recommend **one `wasm` feature embeds both** archives (wasi + unknown-unknown);
  simpler install story, cost is build-time only.

## Work items (in order)

### Phase A — runtime: I/O-free core + productized reactor ABI
- **A1. Extract the I/O-free core** (finding #6, also unblocks INVOCATION.md's
  resident mode). New `runtime/src/core.rs`: `run_query(m, query, limit) ->
  Outcome` plus a render that takes an `io::Write` sink (or returns bytes).
  `entry.rs` (WASI/CLI, writes stdout) and the reactor (writes a buffer) both
  call it. Folds in: limit-aware `exhausted` = `limit.is_none_or(|l| count < l)`
  (finding #4), one error-JSON and one success-JSON impl. Kills the duplication
  between `entry.rs::output_json/output_error` and `reactor.rs`.
- **A2. `Machine::reset_per_query()`** next to the field declarations
  (finding #3), replacing the spike's by-name `reset()`. Makes "did I forget a
  field?" a local question — the leak class only shows under sustained traffic.
- **A3. Productize the reactor ABI** from `reactor.rs`: keep Layout-exact
  `alloc`/`free` (finding #1 — never `Vec::with_capacity` for host-freed
  buffers); `plg_rt_run_query` takes **per-request step + metacall-depth limits**
  (finding #5 — depth matters: wasm ~1 MB stack vs native ~8 MB); apply the D3
  contract; decide `MACHINE` teardown (finding #8 — only needed if a live isolate
  swaps its program). Note the packed return assumes wasm32 (finding #7).
- **A4. `write/1` capture** per D4.

### Phase B — codegen: reactor target
- **B1.** Add the D1 target arm to the `Target` enum (`lib.rs`).
- **B2.** `program.rs`: for the reactor target emit **no** `main`/
  `__main_argc_argv`; emit an exported `plg_init` that calls
  `plg_rt_init(@plg_atom_strs, …, @plg_registry, …, @plg_srcmap, …, @plg_files,
  …)` then `plg_rt_set_machine(%m)`. Triple `wasm32-unknown-unknown`. (This is
  the spike's `awk` transform, done properly in codegen.)
- **B3.** Golden-IR test pinning the reactor entry shape (has `@plg_init`, no
  `@main`).

### Phase C — link + build + CLI
- **C1.** `link.rs::link_wasm_reactor`: `wasm-ld --no-entry
  --export=plg_init,plg_rt_run_query,plg_rt_alloc,plg_rt_free` against the
  `wasm32-unknown-unknown` archive; **no crt/libc** (unlike Tier 1). Reuse
  `wasm_toolchain()` (llc/wasm-ld discovery) — only the archive + link flags
  differ. `llc -mtriple=wasm32-unknown-unknown -mattr=+tail-call`.
- **C2.** `build.rs` + feature (D5): build + embed the `wasm32-unknown-unknown`
  archive (parallel to the wasi one); `just build-runtime-wasm-reactor`.
- **C3.** CLI: extend `parse_target` (D1); `compile_files` routes the reactor
  target to `link_wasm_reactor`; emit Worker glue per D2; output `.wasm`.

### Phase D — Worker glue + deploy ergonomics (per D2)
- **D1g.** Productize the spike's `worker.js` (init once → `alloc` → write query
  → `run_query` → decode `(len<<32)|ptr` BigInt → `free`). Emit `wrangler.toml`
  (Cloudflare) and a `config.capnp` (local workerd). Query source: HTTP
  `?query=` and/or POST body.
- **D2g.** `just wasm-worker-serve <prog.pl>`: compile + run on local workerd
  (the spike workflow, productized).

### Phase E — CI (both tiers; Tier 1's CI is also still deferred)
- **E1. Toolchain on the `navicore-rust` runner image** (prerequisite): rustup
  `wasm32-wasip1` + `wasm32-unknown-unknown`, `llvm-tools-preview`, `wasmtime`,
  `workerd` + `node`.
- **E2. Gate recipes:** existing `just wasm-smoke` (Tier 1) + new
  `just wasm-reactor-smoke` (Tier 2, via Node or workerd). Wire into a **separate
  wasm CI job/workflow** so a runner missing the toolchain doesn't break the main
  `just ci`.
- **E3. Assertions:** byte-identical to native (both tiers) + the
  constant-stack-on-V8 deep-recursion case (1M-deep in the isolate — proven
  manually in the spike, automate it).

### Phase F — docs
- **F1.** Tier-2 user doc (extend `wasm-target.md` or new `wasm-worker.md`):
  prereqs (the Phase-E local list), usage (`--target …`, deploy), the D3
  contract, tuning (per-request limits).
- **F2.** First-class **Cloudflare deployment tutorial**: compile →
  `wrangler deploy` → curl the edge. Lands *with* the implementation (needs the
  real CLI + glue). Replaces `wasm-target.md`'s "Edge / serverless (not yet)".
- **F3.** ROADMAP **M11 — WASM Tier 2** with gate results.

## Definition of done (verification gates)

- `plgc build --target <worker> prog.pl` → a `.wasm` (+ glue) that answers HTTP
  on workerd **byte-identical to native**, including errors.
- 1,000,000-deep recursion in the isolate runs in constant stack — **automated**.
- Default `cargo install` byte-for-byte unchanged; reactor behind the feature.
- I/O-free core is single-source (no JSON duplication); golden IR pins the
  reactor entry; full native suite + clippy + fmt green.
- A wasm CI job runs both tiers' gates green on a toolchain-equipped runner.

## Local prerequisites (Tier 2)

```sh
rustup target add wasm32-unknown-unknown
rustup component add llvm-tools-preview     # shared with Tier 1
npm i -g workerd                            # local Workers runtime
# node is used to drive/test the buffer ABI (V8); wrangler for real Cloudflare
```
Tier 1 also needs `wasm32-wasip1` + `wasmtime` (see `wasm-target.md`).
