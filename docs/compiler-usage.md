# Compiler Usage

`plgc` has four commands:

```
plgc build        Compile .pl sources to a native executable
plgc run          Compile to a temp binary and run it immediately
plgc check        Parse and statically check sources without compiling
plgc completions  Generate shell completion scripts
```

Run `plgc <command> --help` for the exact flags.

## `plgc build`

```sh
plgc build [OPTIONS] [INPUTS]...
```

Compiles one or more `.pl` files (concatenated in order) into a single
native binary.

| Flag | Effect |
|---|---|
| `-o, --output <PATH>` | Output path (default: the stem of the first input). |
| `--debug` | Build at `-O0` with debug-friendly output and DWARF. |
| `--keep-ir` | Keep the generated `.ll` LLVM IR next to the output, for inspection. |
| `--deny-undefined` | Treat calls to undefined predicates as **errors**, not warnings (see below). |
| `--target <TARGET>` | `native` (default), `wasm32-wasi` — a standalone `.wasm` CLI module ([WASM Target](wasm-target.md)) — or `worker` — a V8-isolate reactor for the edge ([WASM Worker](wasm-worker.md)). |

```sh
plgc build rules.pl -o my-linter
plgc build base.pl rules.pl -o app      # inputs concatenated in order
plgc build rules.pl --target wasm32-wasi -o rules.wasm   # needs a wasm-enabled plgc
plgc build rules.pl --target worker      # reactor .wasm + deploy glue (edge)
```

## `plgc run`

```sh
plgc run [OPTIONS] --query <QUERY> [INPUTS]...
```

Compiles to a temporary binary and runs it immediately — the fast
edit-test loop. It **compiles every time**; it never falls back to an
interpreter, so what you test is what you ship.

| Flag | Effect |
|---|---|
| `--query <GOAL>` | The goal to solve, e.g. `"needs(app, X)"`. Required. |
| `--limit <N>` | Maximum number of solutions to report. |
| `--format <FORMAT>` | Output format: `text` or `bson`. **Default: `text`** (human-readable). |
| `--deny-undefined` | As for `build`. |

```sh
plgc run deps.pl --query "needs(app, X)"
plgc run deps.pl --query "needs(app, X)" --limit 1 --format bson
```

## `plgc check`

```sh
plgc check [OPTIONS] [INPUTS]...
```

Parses and statically checks sources without producing a binary — fast
feedback for editors and CI. Reports syntax errors and, by default,
**warns** about calls to predicates that are defined nowhere. `--deny-undefined`
turns those warnings into a non-zero exit.

## Querying a compiled binary

A binary built by `plgc` answers queries directly — this is the same
contract `plgc run` and the REPL use under the hood:

```
./my-binary --query "goal(X)" [--limit N] [--format text|bson]
```

The engine speaks **two** wire encodings, **no JSON**:

- **`--format text`** (the **default**) prints one variable binding per line
  (`X = bob`), the form you read in a terminal. A goal with no solutions
  prints `false.`; a ground goal that succeeds prints `true.`.
- **`--format bson`** prints a single self-delimiting bson document — for a
  program calling the binary and parsing a structured result. Term values
  are `BinData(0x00)` carrying a lossless `TermBuf`:

```
{count:2, exhausted:true, solutions:[{X: BinData(...), ...}, ...]}
```

`exhausted` is `false` when more solutions exist beyond `--limit`. A freshly
built binary advertises both encodings unless the program restricts them with
`:- io_format([...])`.

### Exit codes

The exit code is the answer for shell pipelines — you often don't need to
read stdout at all:

| Code | Meaning |
|---|---|
| `0` | No solutions (the goal failed). |
| `1` | One or more solutions found. |
| `2` | The `--query` could not be parsed. |
| `3` | A runtime error (e.g. an uncaught exception, the step limit). |

```sh
if ./my-linter --query "violation(_, _)" >/dev/null; then
    echo "lint violations found"; exit 1
fi
```

## Undefined predicates

Because patch-prolog compiles the *whole* program, it knows every defined
predicate at build time. A direct call to a predicate that is defined
nowhere can never succeed, so `build`, `run`, and `check` **warn** about it
by default (and `plgl` shows a squiggle). This stays ISO-faithful: such a
call still compiles and raises a catchable `existence_error` if reached.

Pass `--deny-undefined` to promote the warning to a hard error (no binary,
non-zero exit) when you want correctness enforced over leniency.

## Script mode

A `.pl` file with a `plgc` shebang behaves as its own compiled binary:
`plgc` compiles it to a temp binary and execs it, passing along the
arguments after the file. You invoke it with the same `--query` contract as
any compiled binary.

```prolog
#!/usr/bin/env plgc
main :- write(hello), nl.
```

```sh
chmod +x hello.pl
./hello.pl --query "main"
# hello
```
