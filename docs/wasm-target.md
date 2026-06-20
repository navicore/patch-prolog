# WASM Target

`plgc` can compile a Prolog program to a standalone **`wasm32-wasi`** module
instead of a native binary. The module preserves the exact `--query` wire
contract, so anything that drives a compiled binary drives the `.wasm`
identically — only the runner changes (a WASI engine instead of the OS).

This is **Tier 1**: a standalone WASI command that runs on `wasmtime`,
`wasmer`, Wasmtime-based platforms (Spin, Fermyon), and any WASI runtime. The
deployed `.wasm` needs only a wasm engine — no Rust, no clang, no `plgc`.

## Requirements

The build side needs a wasm-capable `plgc` plus the Rust toolchain's LLVM
tools (no wasi-sdk required):

```sh
rustup target add wasm32-wasip1          # std + bundled wasi-libc
rustup component add llvm-tools-preview   # llc + wasm-ld
```

Install a wasm-capable `plgc` (this embeds the wasm runtime archive):

```sh
just install-wasm
# equivalently:
#   just build-runtime-wasm
#   cargo install --path crates/compiler --features wasm --force
```

The default `cargo install` / `just install` is unchanged — wasm support is
opt-in at build time and adds nothing to a native-only `plgc`.

To *run* a module you need a WASI engine, e.g. `wasmtime` (`brew install
wasmtime`, or see wasmtime.dev).

## Usage

```sh
plgc build --target wasm32-wasi deps.pl -o deps.wasm
wasmtime run deps.wasm --query "needs(app, X)" --format json
```

`--query`, `--limit`, `--format`, the JSON shape, and the exit codes
(`0` none · `1` solutions · `2` parse error · `3` runtime error) are all
identical to a native build. If `-o` is omitted, the output defaults to the
input stem with a `.wasm` extension.

## How it works

The same LLVM IR `plgc` emits for native is retargeted to `wasm32-wasi`:

- `llc -mattr=+tail-call` lowers the engine's `musttail` calls to wasm
  `return_call` / `return_call_indirect`, so Prolog recursion and backtracking
  run in **constant stack** exactly as they do natively. If the tail-call
  feature is ever missing, `llc` fails at build time — it never silently emits
  a stack-growing call.
- `wasm-ld` links the program object, the `wasm32-wasip1` runtime archive, and
  the target's self-contained wasi-libc into a WASI command module.

The whole toolchain cost lands on the build side. The artifact you ship is as
self-contained as a native binary — it answers queries with only a wasm engine
present.

## Tuning

The metacall depth bound (`PLG_METACALL_DEPTH`) and step limit
(`PLG_MAX_STEPS`) work the same under WASI as natively. A wasm engine's default
stack is smaller than a native one (~1 MB vs ~8 MB), so for programs that lean
on deeply nested runtime-walked metacalls you may want a lower
`PLG_METACALL_DEPTH`.

## Edge / serverless (not yet)

Running inside V8 isolates (Cloudflare Workers and similar) needs a different
build than Tier 1 — those targets are `wasm32-unknown-unknown` with no WASI and
a host-call entry instead of a CLI. That work is planned but not yet available;
Tier 1 covers WASI runtimes today.
