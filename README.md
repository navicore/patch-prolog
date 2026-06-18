# patch-prolog2

A **standalone Prolog compiler**. `plgc` compiles an ISO-subset Prolog
program to a single native binary with zero runtime dependencies — no
Rust toolchain, no interpreter, no serialized clause database. Predicates
become native code via LLVM.

```sh
plgc build rules.pl -o my-linter      # ~676K standalone binary
./my-linter --query "violation(Field, Reason)"
echo $?   # 0 = no solutions (clean), 1 = solutions found
```

This project supersedes [patch-prolog](https://git.navicore.tech/navicore/patch-prolog),
which shipped an excellent Prolog *engine* but never a compiler. The
language semantics (ISO subset, v1's full builtin vocabulary, the
embedded list stdlib, safety guarantees) carry over at byte-level wire
parity, verified by a ported 200-assertion corpus and a differential
harness against the v1 implementation. The execution model is built on
the architecture proven by
[patch-seq](https://git.navicore.tech/navicore/patch-seq): LLVM IR text
generation, clang linking, and a Rust runtime staticlib embedded in the
compiler binary.

## Requirements

- To **build the compiler**: Rust (see `rust-toolchain.toml`), `just`
- To **use `plgc`**: clang ≥ 15 (for linking) — no Rust required
- To **run compiled binaries**: nothing (libc/libm only)

## Quick start

```sh
just build                 # builds libplg_runtime.a then plgc
target/release/plgc build examples/deps.pl -o deps
./deps --query "needs(app, X)"
./deps --query "findall(D, needs(app, D), Ds)" --format text
```

Scripts work too:

```prolog
#!/usr/bin/env plgc
greet(hello, world).
```

```sh
chmod +x greet.pl && ./greet.pl --query "greet(X, Y)" --format text
```

## Commands

| Command | Purpose |
|---|---|
| `plgc build <in.pl>... [-o out] [--keep-ir] [--debug]` | compile to a native executable (`--debug`: -O0 + DWARF) |
| `plgc run <in.pl>... --query "g(X)"` | compile to a temp binary and run it (never interprets) |
| `plgc check <in.pl>...` | parse + static analysis only |
| `plgc completions <shell>` | shell completion scripts |
| `plgc prog.pl [args...]` | script mode (shebang-friendly) |

Compiled binaries take `--query "goal"`, `--limit N`,
`--format json|text` (default json) and exit with `0` no solutions ·
`1` solutions · `2` query parse error · `3` runtime error. The step
ceiling (default 10,000, uncatchable) is tunable via `PLG_MAX_STEPS`.

## The language

ISO 13211-1 subset, inherited from v1 as the spec (deliberate
exclusions: no modules, DCG, `op/3`, assert/retract, postfix
operators): full backtracking with first-argument indexing, cut
(ISO-transparent in `;` — a documented divergence from a v1 bug),
`->`/`;`/`\+`/`once`, `catch/throw` with the ISO error-term taxonomy,
`findall/3`, `call/N`, `between/3`, checked i64 arithmetic with floored
`mod`, the standard order of terms, ~60 builtins, and a compiled-in
list stdlib (`member`, `append`, `length`, `reverse`, `nth0/1`,
`last`). See [docs/ISO_COMPLIANCE.md](docs/ISO_COMPLIANCE.md).

Deep recursion is safe: all control transfers are guaranteed tail
calls (`musttail`), so a million-deep recursive chain runs in
constant C stack.

## Documentation

The full documentation site is published at
**<https://docs.navicore.tech/patch-prolog/>** — built from `docs/` with
mdBook. Source pages:
[Getting Started](docs/getting-started.md) ·
[Compiler Usage](docs/compiler-usage.md) ·
[Language Guide](docs/language-guide.md) ·
[Operators](docs/OPERATORS.md) ·
[Builtin & Stdlib Reference](docs/builtin-reference.md) ·
[Semantics & ISO Conformance](docs/ISO_COMPLIANCE.md) ·
[REPL Guide](docs/repl-guide.md) ·
[LSP & Editor Guide](docs/lsp-guide.md) ·
[Examples](docs/examples.md) ·
[Architecture](docs/ARCHITECTURE.md)

Build the site locally with `just docs-serve` (live reload) or `just docs`
(one-shot into `book/`).

## Releasing

Crates publish to [crates.io](https://crates.io) from **Forgejo Actions**
(`.forgejo/workflows/release.yml`) when a `v*` tag is pushed:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The workflow then, on the same `navicore-rust` runner CI uses (so the
`rust-toolchain.toml` pin governs):

1. sets the `[workspace.package]` version and the `=`-pinned inter-crate
   dependencies to the tag, regenerates `Cargo.lock`, and commits the bump
   back to `main`;
2. publishes in dependency order with a pause between each so crates.io
   indexes it before the next depends on it:
   `patch-prolog-shared` → `-frontend` → `-runtime` → `-compiler` →
   `-lsp` → `-repl`.

The crates.io **package** names are `patch-prolog-*`; the library and binary
names are unchanged (`use plg_shared`; the binaries are `plgc`/`plgl`/`plgr`),
so `patch-prolog-runtime` still builds the `libplg_runtime.a` the compiler
embeds.

Required repo secrets (Forgejo → Settings → Actions → Secrets and Variables):

- `PAT` — Forgejo token with `write:repository`, to push the version bump to `main`.
- `CRATES_IO_TOKEN` — crates.io API token from <https://crates.io/settings/tokens>.
