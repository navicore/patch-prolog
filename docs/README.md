# patch-prolog

**A standalone Prolog compiler.** `plgc` compiles an ISO-subset Prolog
program to a single native binary with zero runtime dependencies — no Rust
toolchain, no interpreter, no serialized clause database. Predicates become
native code via LLVM.

```sh
plgc build rules.pl -o my-linter      # ~676K standalone binary
./my-linter --query "violation(Field, Reason)"
echo $?   # 0 = no solutions (clean), 1 = solutions found
```

The compiled binary contains **no clause interpreter** — control flow is
generated per predicate as native code, and only primitive services (heap,
trail, unification, builtins, query parsing, output) come from a small
runtime statically linked in. The result runs anywhere with libc/libm and
nothing else.

## The toolchain

Three binaries share the engine:

| Binary | What it is |
|---|---|
| `plgc` | The compiler — `build`, `run`, and `check` your Prolog. |
| `plgr` | An interactive REPL that drives the compiler (never interprets). |
| `plgl` | A Language Server (diagnostics, completion, hover, go-to-definition). |

## Where to go next

- **[Installation](getting-started.md)** — install the toolchain and
  compile your first program.
- **[Compiler Usage](compiler-usage.md)** — `plgc` commands, the query
  wire-contract, and exit codes.
- **REPL Guide**, **Language Guide**, and the **Builtin & Stdlib
  Reference** follow as those pages land.
