# WASM Worker (edge / V8 isolates)

`plgc` can compile a Prolog program to a **reactor** wasm module that runs
inside a V8 isolate — Cloudflare Workers, `workerd`, or Node — instead of a
process. This is **Tier 2**: there is no `main`, no argv, no stdio. The module
*exports* a small buffer ABI and a JS host calls it over linear memory; a query
becomes a function call inside a warm, globally-replicated isolate.

This is the *scale-to-zero / global edge* shape. The compiled artifact is the
same no-VM, no-interpreter engine as a native build — small enough to clear the
single-digit-MB module limits edge isolates impose, and stateless by design, so
it fits the isolate model by construction. For WASI runtimes (wasmtime, Spin,
…), use [Tier 1](wasm-target.md) instead.

The answers are **byte-identical to a native build**, including error text — the
reactor runs the same engine and emits the same **bson** envelope a native
`--format bson` build does; the emitted host glue decodes that bson to the
JSON HTTP response. Only the transport differs (a memory buffer instead of
stdout).

## Requirements

Build side — a wasm-capable `plgc` plus the Rust LLVM tools (no wasi-sdk):

```sh
rustup target add wasm32-unknown-unknown   # the reactor target
rustup component add llvm-tools-preview     # llc + wasm-ld
just install-wasm                           # plgc with both wasm archives embedded
```

`just install-wasm` builds and embeds **both** wasm runtime archives (Tier 1
`wasm32-wasip1` and Tier 2 `wasm32-unknown-unknown`) behind the one `wasm`
feature, so a single install covers both targets. The default `cargo install` /
`just install` is unchanged — wasm support is opt-in at build time.

Run side — anything with a V8 isolate and wasm tail calls:

- **Cloudflare Workers** (deploy with `wrangler`), or local **`workerd`**
  (`npm i -g workerd`) for the dev loop, or
- **Node** ≥ 20 for scripting/testing the module directly.

The constant-stack guarantee relies on wasm `return_call`; V8 ships tail calls
and the module is rejected at instantiation by any engine that lacks them — never
a silent stack overflow.

## Usage

```sh
plgc build --target worker deps.pl
```

This emits `deps.worker.wasm` plus four **overrideable** scaffolding files next
to it:

| File | Role |
|---|---|
| `deps.worker.wasm` | the reactor module (the real artifact, always regenerated) |
| `reactor.mjs` | the buffer-ABI marshalling (`runQuery` / `assertExports`) |
| `worker.js` | a Cloudflare/`workerd` `fetch` handler that calls `reactor.mjs` |
| `wrangler.toml` | Cloudflare deploy config |
| `config.capnp` | local `workerd serve` config |

The glue files are **written only if absent**, so a rebuild refreshes the
`.wasm` but never clobbers your edits — delete one to regenerate it. `--target
wasm32-unknown-unknown` is accepted as a synonym for `worker`. If `-o` is
omitted the output defaults to the input stem with a `.worker.wasm` extension
(distinct from Tier 1's `.wasm`, so building both targets doesn't overwrite).

### Run it locally

```sh
just wasm-worker-serve examples/deps.pl
# compiles + serves on workerd, then:
curl 'http://localhost:8080/?query=needs(app, X)'
# {"count":5,"exhausted":true,"output":"","solutions":[{"X":"auth"},…]}
```

The handler reads the goal from `?query=` or, failing that, the POST body:

```sh
curl -X POST --data 'depends_on(app, D)' http://localhost:8080/
```

## Response shape

The JSON response shape carries one Tier-2 addition — an `output` field
carrying anything the program wrote with `write/1`/`writeln/1`/`nl/0` (an
isolate has no stdout, so output is captured losslessly rather than lost):

```json
{"count":1,"exhausted":true,"output":"","solutions":[{"X":"auth"}]}
```

The field is always present (empty when nothing was written), so a host can rely
on its shape. The bson the reactor emits is identical to a native
`--query --format bson` (the `output` field is the engine's captured `write/1`
bytes, present in both). Errors render as `{"error":"…"}`, with the same
message text a native build produces.

## The buffer ABI

If you replace the glue, the contract the module exports is:

```
plg_init()                                   build the Machine once per isolate
plg_rt_alloc(len) -> ptr                      host writes the query bytes here
plg_rt_run_query(ptr, len, limit, steps, depth) -> (len<<32)|ptr   bson result
plg_rt_free(ptr, len)                         host frees the query / result buffer
```

`reactor.mjs` is the single source of this dance and is imported by both the
deployed `worker.js` and the test harness, so the two can't drift. Free every
buffer by its **requested** length (the allocator is length-keyed), and read the
result view *after* `plg_rt_run_query` returns — solving can grow linear memory
and detach an earlier view.

### Concurrency contract

**One in-flight query per isolate.** The module keeps a single program Machine;
a V8 isolate is single-threaded, but a Worker can interleave async tasks, so the
host must not start a second query before the first returns. The emitted
`worker.js` satisfies this automatically: its `runQuery` is fully synchronous
(the only `await` is reading the POST body, *before* the ABI call), so
concurrent `fetch` invocations serialize on the event loop.

## Tuning

`plg_rt_run_query`'s last three arguments bound a query *before* the platform's
own CPU/wall limit cuts it off — they mirror the native knobs:

| Arg | Native equivalent | `0` means |
|---|---|---|
| `limit` | `--limit` | unbounded solutions |
| `steps` | `PLG_MAX_STEPS` | module default (10,000) |
| `depth` | `PLG_METACALL_DEPTH` | module default (1,000) |

The emitted `worker.js` calls `runQuery(reactor(), query)` with the module
defaults. To raise the step ceiling for heavier programs, pass options —
`runQuery(reactor(), query, { stepLimit: 100_000_000n })` (the step limit is an
`i64`, hence the `BigInt`). Depth matters more here than natively: a wasm
isolate's stack (~1 MB) is far smaller than a native one (~8 MB). Ordinary
recursion and `call/1` tail recursion are constant-stack regardless — a
1,000,000-deep `call/1` returns in a V8 isolate via `return_call`.

## Deploy to Cloudflare

The emitted `wrangler.toml` makes the module a deployable Worker. From the
directory holding the build output:

```sh
plgc build --target worker deps.pl     # emits deps.worker.wasm + glue
npx wrangler deploy                     # uploads the Worker + wasm module
```

`wrangler` bundles `worker.js` and `reactor.mjs` and uploads the `.wasm` as a
compiled module (the `CompiledWasm` rule in `wrangler.toml`). Then query the
edge:

```sh
curl 'https://deps.<your-subdomain>.workers.dev/?query=needs(app, X)'
```

The isolate cold-starts once (atom table + registry built at `plg_init`), then
every request is a warm in-memory call — no process fork, no per-request parse
of the program. Edit the generated `name`/`compatibility_date` in
`wrangler.toml` and the routing/limits in `worker.js` to taste; both are
scaffolding, not regenerated.

### Footprint

A reactor module is a few MB (the `deps` example is ~1.7 MB raw, well within the
Workers budget and compressing further). Large fact tables inflate the module's
`.rodata`; a program whose data pushes past the isolate size cap belongs on Tier
1 / containers instead. The toolchain cost is entirely build-side — the deployed
`.wasm` runs with only a wasm engine present.

## How it works

The reactor reuses the same LLVM IR as native and Tier 1, retargeted to
`wasm32-unknown-unknown`:

- The module emits **no `main`** — instead an exported `plg_init` builds the
  Machine and hands it to the runtime; the host drives the buffer ABI exports.
- `llc -mattr=+tail-call` lowers the engine's `musttail` chains to
  `return_call` / `return_call_indirect`, so recursion and backtracking run in
  constant stack inside the isolate. A missing tail-call feature is a loud
  build/load error, never a silent overflow.
- `wasm-ld --no-entry` links the program object against the
  `wasm32-unknown-unknown` runtime archive with **no crt and no libc** — the
  reactor touches no WASI, so the resulting module imports nothing and
  instantiates in a bare isolate.
