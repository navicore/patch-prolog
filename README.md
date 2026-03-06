# patch-prolog

A Rust-based Prolog engine for linting generative AI output. Rules are written in standard Prolog (`.pl` files), compiled into the binary at build time, and queried at runtime with zero file I/O.

## Quick Start

```bash
# Build
cargo build --release

# Run a query against the compiled knowledge base
patch-prolog --query "violation(X)"

# Limit results
patch-prolog --query "member(X, [a,b,c])" --limit 2

# Text output instead of JSON
patch-prolog --query "parent(tom, X)" --format text
```

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | No solutions (compliant) |
| `1`  | Solutions found (violations) |
| `2`  | Parse error |
| `3`  | Runtime error |

## Adding Rules

Place `.pl` files in the `knowledge/` directory. They are compiled into the binary at build time — no runtime file loading.

```prolog
% knowledge/my_rules.pl
violation(X) :- component(X, Type), \+ approved(Type).
```

The standard library (`knowledge/stdlib.pl`) provides: `member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`.

## Built-in Predicates

~55 built-in predicates covering core operations, type checking, control flow, arithmetic, I/O, term ordering, introspection, sorting, and number conversion. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full list.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — System structure, module responsibilities, data flows
- [docs/TESTING.md](docs/TESTING.md) — Test organization, running tests, writing tests
- [docs/design/](docs/design/) — Design decisions and technical rationale
- [ROADMAP.md](ROADMAP.md) — Completed phases and future work

## Development

```bash
cargo test --all    # Run all 277 tests (134 unit + 143 integration)
cargo build         # Debug build
cargo run -- --query "true" --format text   # Quick smoke test
```

## Architecture

Rust workspace with two crates:

- **`patch-prolog`** — CLI binary (`src/main.rs`, `build.rs`)
- **`prolog-core`** — Engine library (`crates/prolog-core/`) — tokenizer, parser, unifier, solver, built-ins

The engine compiles Prolog at build time via `build.rs`, serializes with bincode, and embeds the compiled database in the binary. At runtime, queries are parsed and resolved against the embedded knowledge base. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for details.
