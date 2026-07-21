# Testing Guide

The test pyramid runs **against compiled binaries**, not an in-process
engine — the thing being tested is the compiler.

## Layers

1. **Unit tests** (`cargo test`), co-located per module:
   - `plg-shared`: interner / term / first-arg keys
   - `plg-frontend`: tokenizer + parser
   - `plg-runtime`: heap, trail, unify, choice points, builtins, the
     minimal goal parser
   - `plg-compiler`: codegen golden-IR tests — `compile_to_ir(src)` and
     assert on IR substrings; no clang needed, fast.

2. **Integration tests** (`just test-integration`):
   `crates/compiler/tests/integration/` — a corpus that compiles a
   fixture `.pl` once and runs the binary with many `--query` calls,
   asserting stdout (text/bson) and exit codes. Batching matters: clang
   invocations dominate test time, so group assertions per fixture
   program, not per query.

3. **Binary hygiene**:
   - `just check-binary-contents` — a hello-world binary must not
     contain symbols from builtin families it never references
     (linker dead-stripping is working).
   - `just size-gate` — the CI size gate: hello-world release binary
     must stay under the 1.4 MB ceiling, built in a fresh target dir so
     the measurement is deterministic (#63). `just footprint` is the
     dev-time recorder; the patch-seq bar is ~730 KB on Linux.

## Invariants every milestone must keep

- Exit codes: `0` no solutions, `1` solutions, `2` query parse error,
  `3` runtime error.
- Wire shape: the `bson` envelope is `{count, exhausted, solutions[], output?}`
  (the `text` format carries solutions only, no `count`/`exhausted`).
- Deep determinate recursion runs in constant C stack (a test compiles
  a long `ancestor/2` chain and runs it under a small `ulimit -s`).
- Compiled binaries run on a machine without Rust (CI runs them in a
  bare job step; nothing in the harness invokes cargo at query time).

## Running

```sh
just test               # all Rust unit tests
just test-integration   # compiled-binary corpus
just ci                 # everything CI runs
```
