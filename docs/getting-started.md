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

Create `deps.pl` — a small build-dependency graph:

```prolog
depends_on(app, auth).
depends_on(app, ui).
depends_on(auth, crypto).
depends_on(ui, render).
depends_on(render, crypto).

% needs/2 is the transitive closure of depends_on.
needs(X, Y) :- depends_on(X, Y).
needs(X, Y) :- depends_on(X, Z), needs(Z, Y).
```

Compile it to a native binary and query it — `needs/2` walks the graph
transitively:

```sh
plgc build deps.pl -o deps
./deps --query "needs(app, X)" --format text
# X = auth
# X = ui
# X = crypto
# X = render
# X = crypto
```

Or skip the explicit build during development — `plgc run` compiles to a
temporary binary and executes it in one step (it still compiles; it never
interprets):

```sh
plgc run deps.pl --query "needs(app, X)"
```

To explore interactively, start the REPL:

```sh
plgr
```

```
plg> depends_on(app, auth).
  defined.  (1 in session)
plg> depends_on(app, ui).
  defined.  (2 in session)
plg> ?- depends_on(app, X).
  X = auth ;
  X = ui .
```

(At the `;` prompt, press `;` for the next solution or any other key to
stop.)

From here, see **[Compiler Usage](compiler-usage.md)** for the full `plgc`
surface and the query wire-contract.
