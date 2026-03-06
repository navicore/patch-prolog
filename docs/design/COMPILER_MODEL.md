# Design: Standalone Compiler Model

## Problem

Building a linter from `.pl` rules requires the patch-prolog source tree. Users copy `.pl` files into `knowledge/`, run `cargo build`, and get a binary. They're recompiling the compiler every time they change their rules.

## Intent

Make `patch-prolog` a compiler. Input: `.pl` files. Output: a standalone native binary that accepts queries against those rules.

```
patch-prolog compile examples/linting.pl -o my-linter
./my-linter --query "violation(X, Y)"
```

Users need Rust installed. That's fine — the output is a real native binary with zero runtime dependencies.

## Approach

Scaffold a temporary Rust project, invoke cargo:

1. `patch-prolog compile *.pl -o my-linter` parses the `.pl` files and validates them
2. Generates a temporary Rust project:
   - `Cargo.toml` depending on `patch-prolog-core` (from crates.io or a path/git dep)
   - `build.rs` that writes the serialized `CompiledDatabase` as a `.bin` file
   - `main.rs` with the query CLI (same as today's `src/main.rs`)
3. Invokes `cargo build --release` in the temp project
4. Copies the resulting binary to the output path
5. Cleans up the temp project

The generated binary is identical to what we build today — same CLI, same exit codes, same JSON/text output. The only difference is how the rules got there.

## Key Decisions

**Where does `patch-prolog-core` come from?** Publish to crates.io. Users install `patch-prolog` via `cargo install patch-prolog`. The generated temp project depends on `patch-prolog-core` from crates.io — no source tree, no git pins, no vendoring. Clean and simple.

**What about stdlib?** The generated project needs `knowledge/stdlib.pl`. Options:
- Embed stdlib source in the `patch-prolog` binary, write it to the temp project's `knowledge/` dir alongside user rules
- Let users provide their own stdlib (or none)

Recommendation: embed stdlib in patch-prolog and always include it. Users can override with their own.

**Template for main.rs?** The generated `main.rs` can be:
- A verbatim copy of today's `src/main.rs` (embedded as a string in patch-prolog)
- A simplified version if we want to trim dependencies

Recommendation: embed the current `main.rs` as-is. It's small (172 lines) and already works.

## Constraints

- Must not break the existing `build.rs` pipeline — the source-tree workflow keeps working
- `patch-prolog-core` stays a library crate — no changes to the engine
- Generated binaries must have the same CLI interface, exit codes, and output formats
- Rust toolchain required on the user's machine (explicit non-goal to avoid this)

## Checkpoints

1. `patch-prolog compile examples/linting.pl -o my-linter` produces a binary
2. `./my-linter --query "violation(X, Y)"` returns correct results
3. `./my-linter --query "member(X, [a,b,c])" --format text` works (stdlib included)
4. Parse errors in `.pl` files are caught at compile time, not deferred to cargo
5. The existing `cargo build` workflow in the source tree still works unchanged
