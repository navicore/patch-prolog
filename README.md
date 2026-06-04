# patch-prolog2

A **standalone Prolog compiler**. `plgc` compiles an ISO-subset Prolog
program to a single native binary with zero runtime dependencies — no
Rust toolchain, no interpreter, no serialized clause database. Predicates
become native code via LLVM.

```sh
plgc build rules.pl -o my-linter
./my-linter --query "violation(Field, Reason)"
echo $?   # 0 = no solutions (clean), 1 = solutions found
```

This project supersedes [patch-prolog](https://git.navicore.tech/navicore/patch-prolog),
which shipped an excellent Prolog *engine* but never a compiler — see
[docs/design/LESSONS_FROM_V1.md](docs/design/LESSONS_FROM_V1.md). The
language semantics (ISO subset, ~60 builtins, safety guarantees) carry
over unchanged; the execution model is rebuilt on the architecture
proven by [patch-seq](https://git.navicore.tech/navicore/patch-seq):
LLVM IR text generation, clang linking, and a Rust runtime staticlib
embedded in the compiler binary.

## Requirements

- To **build the compiler**: Rust (see `rust-toolchain.toml`), `just`
- To **use `plgc`**: clang ≥ 15 (for linking) — no Rust required
- To **run compiled binaries**: nothing

## Quick start

```sh
just build                 # builds libplg_runtime.a then plgc
target/release/plgc build examples/family.pl -o family
./family --query "grandparent(tom, X)"
```

## Commands

| Command | Purpose |
|---|---|
| `plgc build <in.pl>... [-o out] [--keep-ir] [--debug]` | compile to a native executable |
| `plgc run <in.pl>... --query "g(X)"` | compile to a temp binary and run it (never interprets) |
| `plgc check <in.pl>...` | parse + static analysis only |
| `plgc completions <shell>` | shell completion scripts |

## Docs

Project context lives in `docs/` (myspec convention):
[ARCHITECTURE](docs/ARCHITECTURE.md) ·
[ROADMAP](docs/ROADMAP.md) ·
[ISO compliance](docs/ISO_COMPLIANCE.md) ·
[Compilation model](docs/design/COMPILATION_MODEL.md) ·
[Runtime ABI](docs/design/RUNTIME_ABI.md) ·
[Lessons from v1](docs/design/LESSONS_FROM_V1.md)
