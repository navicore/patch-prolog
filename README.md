# patch-prolog

[![patch-prolog-compiler](https://img.shields.io/crates/v/patch-prolog-compiler.svg?label=patch-prolog-compiler)](https://crates.io/crates/patch-prolog-compiler)
[![patch-prolog-repl](https://img.shields.io/crates/v/patch-prolog-repl.svg?label=patch-prolog-repl)](https://crates.io/crates/patch-prolog-repl)
[![patch-prolog-lsp](https://img.shields.io/crates/v/patch-prolog-lsp.svg?label=patch-prolog-lsp)](https://crates.io/crates/patch-prolog-lsp)

**A standalone Prolog compiler.** `plgc` compiles an ISO-subset Prolog
program to a single native binary with zero runtime dependencies — no
Rust toolchain, no interpreter, no serialized clause database. Predicates
become native code via LLVM.

<!-- docs:skip-start -->
**Home Code Repository** is at [git.navicore.tech](https://git.navicore.tech/navicore/patch-prolog)

**PRs and issues** welcome at the [GitHub mirror](https://github.com/navicore/patch-prolog)

[Documentation](https://docs.navicore.tech/patch-prolog)

**API docs (rustdoc)** per crate on docs.rs:
[patch-prolog-shared](https://docs.rs/patch-prolog-shared) · [patch-prolog-frontend](https://docs.rs/patch-prolog-frontend) · [patch-prolog-runtime](https://docs.rs/patch-prolog-runtime) · [patch-prolog-compiler](https://docs.rs/patch-prolog-compiler) · [patch-prolog-lsp](https://docs.rs/patch-prolog-lsp) · [patch-prolog-repl](https://docs.rs/patch-prolog-repl)
<!-- docs:skip-end -->

```sh
plgc build rules.pl -o my-linter      # ~676K standalone binary
./my-linter --query "violation([field(id,integer)], Field, Reason)"
echo $?   # 0 = no solutions (clean), 1 = solutions found
```

The compiled binary contains **no clause interpreter** — control flow is
generated per predicate as native code, and only primitive services (heap,
trail, unification, builtins, query parsing, output) come from a small
runtime statically linked in. The result runs anywhere with libc/libm and
nothing else.

The
language semantics (ISO subset, the full builtin vocabulary, the
embedded list stdlib, safety guarantees) are pinned by a 200-assertion
integration corpus. The execution model is built on
the architecture proven by
[patch-seq](https://git.navicore.tech/navicore/patch-seq): LLVM IR text
generation, clang linking, and a Rust runtime staticlib embedded in the
compiler binary.

## The toolchain

Three binaries share the engine:

| Binary | What it is |
|---|---|
| `plgc` | The compiler — `build`, `run`, and `check` your Prolog. |
| `plgr` | An interactive REPL that drives the compiler (never interprets). |
| `plgl` | A Language Server (diagnostics, completion, hover, go-to-definition). |

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

An ISO 13211-1 subset, with ISO conformance as the guide. A few
minor, deliberate deviations and several safety extensions beyond
ISO are documented in [docs/ISO_COMPLIANCE.md](docs/ISO_COMPLIANCE.md);
the subset is defined by its omissions and its features.

Deliberate omissions: no modules, DCG, `op/3`, `assert/retract`, or
postfix operators.

Features:

- Full backtracking with first-argument indexing
- Cut, transparent through `,`/`;`/`->` (ISO semantics)
- `->`/`;`/`\+`/`once`
- `catch/throw` with the ISO error-term taxonomy
- `findall/3`, `call/N`, `between/3`
- Checked i64 arithmetic with floored `mod`
- The standard order of terms
- ~60 builtins, plus a compiled-in list stdlib
  (`member`, `append`, `length`, `reverse`, `nth0/1`, `last`)

Deep recursion is safe: all control transfers are guaranteed tail
calls (`musttail`), so a million-deep recursive chain runs in
constant C stack.

## Documentation

<!-- docs:skip-start -->
The full documentation site is published at
**<https://docs.navicore.tech/patch-prolog/>** — built from `docs/` with
mdBook.
<!-- docs:skip-end -->

Source pages:
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

<!-- docs:skip-start -->
Build the site locally with `just docs-serve` (live reload) or `just docs`
(one-shot into `book/`).
<!-- docs:skip-end -->

<!-- docs:skip-start -->
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
- `DOCS_CLIENT_ID` — anz client ID for publishing the mdbook
- `DOCS_CLIENT_SECRET` — anz client secret for publishing the mdbook

<!-- docs:skip-end -->
