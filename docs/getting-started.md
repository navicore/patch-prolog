# Installation

patch-prolog is built from source. The toolchain is three binaries —
`plgc` (compiler), `plgr` (REPL), and `plgl` (language server) — installed
together.

## Requirements

| To… | You need |
|---|---|
| **build the toolchain** | Rust (see `rust-toolchain.toml`) and [`just`](https://github.com/casey/just) |
| **use `plgc`** | `clang` ≥ 15 — it links the generated code. No Rust required. |
| **run a compiled binary** | nothing — only system `libc`/`libm` |

That last row is the point: the programs `plgc` produces are standalone.
You hand someone the binary and it runs, with no Prolog system installed.

## Build and install

```sh
just install
```

This builds the runtime and compiler, then installs all three binaries
(`plgc`, `plgr`, `plgl`) with `cargo install`. To build in place without
installing:

```sh
just build                 # produces target/release/plgc
```

Shell completions for `plgc`:

```sh
plgc completions zsh > ~/.zfunc/_plgc      # or: bash | fish | elvish | powershell
```

## Your first program

Create `family.pl`:

```prolog
parent(tom, bob).
parent(bob, ann).

ancestor(X, Y) :- parent(X, Y).
ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).
```

Compile it to a native binary and query it:

```sh
plgc build family.pl -o family
./family --query "ancestor(tom, X)" --format text
# X = bob
# X = ann
```

Or skip the explicit build during development — `plgc run` compiles to a
temporary binary and executes it in one step (it still compiles; it never
interprets):

```sh
plgc run family.pl --query "ancestor(tom, X)"
# X = bob
# X = ann
```

To explore interactively, start the REPL:

```sh
plgr
```

```
plg> parent(tom, bob).
  defined.  (1 in session)
plg> parent(bob, ann).
  defined.  (2 in session)
plg> ?- parent(tom, X).
  X = bob .
```

From here, see **[Compiler Usage](compiler-usage.md)** for the full `plgc`
surface and the query wire-contract.
