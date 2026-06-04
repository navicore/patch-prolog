# Testing Guide

The test pyramid runs **against compiled binaries**, not an in-process
engine — the thing being tested is the compiler.

## Layers

1. **Unit tests** (`cargo test`), co-located per module:
   - `plg-shared`: interner / term / first-arg keys
   - `plg-frontend`: tokenizer + parser (ported from v1)
   - `plg-runtime`: heap, trail, unify, choice points, builtins, the
     minimal goal parser
   - `plg-compiler`: codegen golden-IR tests — `compile_to_ir(src)` and
     assert on IR substrings; no clang needed, fast.

2. **Integration tests** (`just test-integration`):
   `crates/compiler/tests/integration/` — the v1 corpus ported to a
   harness that compiles a fixture `.pl` once and runs the binary with
   many `--query` calls, asserting stdout (JSON/text) and exit codes.
   Batching matters: clang invocations dominate test time, so group
   assertions per fixture program, not per query.

3. **Differential tests** (`just diff-test`, M2–M4 only): the same
   (program, goal) corpus through the old patch-prolog interpreter
   (`../patch-prolog`, the semantics oracle) and through compiled
   binaries; normalized JSON must be identical. This is the main safety
   net while the backend is rewritten. Retired once M4 parity locks.

4. **Binary hygiene**:
   - `just check-binary-contents` — a hello-world binary must not
     contain symbols from builtin families it never references
     (linker dead-stripping is working).
   - `just footprint` — track hello-world binary size; the patch-seq
     bar is ~730 KB on Linux.

## Invariants every milestone must keep

- Exit codes: `0` no solutions, `1` solutions, `2` query parse error,
  `3` runtime error.
- JSON shape: `{"solutions":[...], "count":N, "exhausted":bool}` —
  byte-compatible with v1's runner output.
- Deep determinate recursion runs in constant C stack (a test compiles
  a long `ancestor/2` chain and runs it under a small `ulimit -s`).
- Compiled binaries run on a machine without Rust (CI runs them in a
  bare job step; nothing in the harness invokes cargo at query time).

## Running

```sh
just test               # all Rust unit tests
just test-integration   # compiled-binary corpus
just diff-test          # vs v1 oracle (needs ../patch-prolog)
just ci                 # everything CI runs
```
