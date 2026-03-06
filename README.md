# patch-prolog

A Prolog compiler for linting generative AI output. You write rules in standard Prolog, compile them into a self-contained Rust binary, and query that binary at runtime — no interpreter, no file loading, no runtime dependencies.

## How It Works

1. Write Prolog rules (`.pl` files) in the `knowledge/` directory
2. `cargo build` compiles them into the binary via `build.rs`
3. Run the binary with a query — it resolves against the embedded knowledge base

The rules are baked in. The binary is the program.

## Example: Lint an AI-Generated Schema

The `examples/linting.pl` file defines rules for checking AI-generated API schemas:

```prolog
% Flag sensitive fields that should not be exposed
violation(Field, sensitive_field) :-
    field(user, Field, _),
    sensitive(Field).

sensitive(ssn).
sensitive(password).
```

To compile and run it:

```bash
# Copy rules into the knowledge base
cp examples/linting.pl knowledge/

# Compile — rules are baked into the binary
cargo build --release

# Query for violations
./target/release/patch-prolog --query "violation(Field, Reason)"
# → {"solutions":[{"Field":"ssn","Reason":"sensitive_field"},{"Field":"password","Reason":"sensitive_field"}],"count":2,"exhausted":true}

# Exit code 1 = violations found
echo $?
# → 1
```

```bash
# Text output
./target/release/patch-prolog --query "violation(Field, Reason)" --format text
# Field = ssn
# Reason = sensitive_field
# Field = password
# Reason = sensitive_field
```

## Example: Family Relationships

```bash
cp examples/family.pl knowledge/
cargo build --release

./target/release/patch-prolog --query "grandparent(tom, X)" --format text
# X = bob
# X = carol
```

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | No solutions (compliant) |
| `1`  | Solutions found (violations) |
| `2`  | Parse error |
| `3`  | Runtime error |

## Writing Rules

Place `.pl` files in `knowledge/`. They are compiled into the binary on `cargo build` — the binary has no runtime file dependencies.

The standard library (`knowledge/stdlib.pl`) is always included and provides: `member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`.

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
