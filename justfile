# patch-prolog2 Build System
#
# This is the SOURCE OF TRUTH for all build/test/lint operations.
# Forgejo Actions calls these recipes directly - no duplication!

# Default recipe: show available commands
default:
    @just --list

# Build everything (runtime first — ordering matters, see `install` note)
build: build-runtime build-compiler

# `build-compiler` depends on a fresh `target/release/libplg_runtime.a`
# because `crates/compiler/build.rs` embeds it via `include_bytes!`.
# Without the explicit `build-runtime` first, a build-dep-only rebuild
# of plg-runtime updates `deps/libplg_runtime-<hash>.a` but leaves the
# no-hash file stale, producing a plgc whose self-reported version
# doesn't match its embedded runtime bytes.
install: build
    @echo "Installing the compiler (plgc)..."
    cargo install --path crates/compiler --force
    @echo "Installing the language server (plgl)..."
    cargo install --path crates/lsp --force
    @echo "Installing the REPL (plgr)..."
    cargo install --path crates/repl --force
    @echo "✅ Installed: plgc, plgl, plgr"

# Build the Rust runtime as static library
build-runtime:
    @echo "Building runtime..."
    cargo build --locked --release -p patch-prolog-runtime
    @echo "✅ Runtime built: target/release/libplg_runtime.a"

# Build the compiler
build-compiler:
    @echo "Building compiler..."
    cargo build --locked --release -p patch-prolog-compiler
    @echo "✅ Compiler built: target/release/plgc"

# Compile all example programs to native binaries.
# Examples needing not-yet-implemented builtins are skipped with a note
# (the compiler reports the target milestone, e.g. linting.pl needs \+
# which lands in M3) — remove the tolerance as milestones complete.
build-examples: build
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building examples..."
    mkdir -p target/examples
    for file in examples/*.pl; do
        name=$(basename "$file" .pl)
        echo "  Compiling $file..."
        if ! out=$(target/release/plgc build "$file" -o "target/examples/$name" 2>&1); then
            if echo "$out" | grep -q "not yet supported"; then
                echo "  Skipping $file ($out)"
            else
                echo "$out"
                exit 1
            fi
        fi
    done
    echo "✅ Examples built in target/examples/"
    ls -lh target/examples/

# Run all Rust unit tests.
# The explicit debug-profile runtime build first matters: plg-compiler's
# build.rs embeds target/debug/libplg_runtime.a (the no-hash artifact),
# which only refreshes when plg-runtime is built DIRECTLY — a build-dep
# rebuild updates only deps/libplg_runtime-<hash>.a and leaves the
# embedded copy stale (same trap as the release `build` ordering).
test:
    @echo "Running Rust unit tests..."
    cargo build --locked -p patch-prolog-runtime
    cargo test --locked --workspace --all-targets

# Run integration tests (compile .pl programs and query the binaries)
test-integration: build
    @echo "Running compiled-binary integration tests..."
    cargo test --locked --release -p patch-prolog-compiler --test integration
    @echo "✅ Integration tests passed!"

# Differential tests: same (program, goal) corpus through the old
# patch-prolog interpreter (oracle) and the new compiled binaries.
# Requires ../patch-prolog/target/release/prlg; SKIPS silently in CI
# (the runner image has no oracle). Deliberate divergences are pinned
# as direct tests instead — see docs/ISO_COMPLIANCE.md.
diff-test: build
    @echo "Running differential tests vs ../patch-prolog oracle..."
    cargo test --locked --release -p patch-prolog-compiler --test differential -- --nocapture

# Binary hygiene: size ceiling + standalone (system-libs-only) contract
check-binary-contents:
    cargo test --locked --release -p patch-prolog-compiler --test binary_size -- --nocapture

# Lint all Prolog example sources with the compiler's static checks
lint-pl: build
    @echo "Checking Prolog sources..."
    ./target/release/plgc check examples/*.pl
    @echo "✅ Prolog check passed!"

# Run clippy on all workspace members
lint:
    @echo "Running clippy..."
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Format all code
fmt:
    @echo "Formatting code..."
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    @echo "Checking code formatting..."
    cargo fmt --all -- --check

# Measure hello-world binary footprint and print it (record in docs)
footprint: build
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    printf 'hello(world).\n' > "$tmp/hello.pl"
    target/release/plgc build "$tmp/hello.pl" -o "$tmp/hello"
    ls -lh "$tmp/hello"
    size=$(du -h "$tmp/hello" | cut -f1)
    echo "hello-world binary footprint: $size"
    rm -rf "$tmp"

# Run all CI checks (same as Forgejo Actions!)
# (diff-test is intentionally NOT here: it needs the local v1 oracle.
# Run it manually when ../patch-prolog is present.)
ci: fmt-check lint test build build-examples test-integration lint-pl check-binary-contents
    @echo ""
    @echo "✅ All CI checks passed!"

# Clean all build artifacts
clean:
    @echo "Cleaning build artifacts..."
    cargo clean
    rm -f examples/*.ll
    rm -rf target/examples
    @echo "✅ Clean complete"

# Development: quick format + build + test
dev: fmt build test

# Build the mdbook documentation site into ./book/ (what CI publishes)
docs:
    @echo "Building documentation..."
    mdbook build
    @echo "✅ Documentation built in book/"

# Serve the docs locally with live reload
docs-serve:
    mdbook serve --open
