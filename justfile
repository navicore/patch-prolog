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

# Build the runtime for wasm32-wasip1 (Tier 1, docs/design/WASM.md). The
# archive (target/wasm32-wasip1/release/libplg_runtime.a) is embedded into a
# wasm-enabled plgc by build.rs under `--features wasm`. Needs the wasm target:
#   rustup target add wasm32-wasip1
build-runtime-wasm:
    @echo "Building wasm runtime (wasm32-wasip1)..."
    cargo build --locked --release -p patch-prolog-runtime --target wasm32-wasip1
    @echo "✅ Wasm runtime built: target/wasm32-wasip1/release/libplg_runtime.a"

# Install a wasm-capable plgc: builds the wasm runtime, then installs plgc with
# the `wasm` feature so it can emit `--target wasm32-wasi` modules. Also needs
# the rustup llvm-tools (llc/wasm-ld): rustup component add llvm-tools-preview
install-wasm: build-runtime-wasm
    @echo "Installing wasm-capable compiler (plgc --features wasm)..."
    cargo install --path crates/compiler --features wasm --force
    @echo "✅ Installed plgc with wasm support"

# Tier 1 wasm smoke test (LOCAL only — not in CI yet). Needs: the
# wasm32-wasip1 target, llvm-tools-preview, and wasmtime on PATH. Compiles an
# example to wasm and asserts it answers --query byte-identically to native.
wasm-smoke: build-runtime-wasm
    #!/usr/bin/env bash
    set -euo pipefail
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    echo "Compiling examples/deps.pl (native + wasm)..."
    cargo run -q -p patch-prolog-compiler --bin plgc -- \
        build examples/deps.pl -o "$work/deps-native"
    cargo run -q --features wasm -p patch-prolog-compiler --bin plgc -- \
        build examples/deps.pl -o "$work/deps.wasm" --target wasm32-wasi
    fail=0
    # `--query` exits 1 when solutions are found (the wire contract), so the
    # captures must not trip `set -e`; the byte comparison is the real check.
    for q in "needs(app, X)" "depends_on(app, D)" "shared_deps(auth, render, Ds)"; do
        native=$("$work/deps-native" --query "$q" --format json || true)
        wasm=$(wasmtime run "$work/deps.wasm" --query "$q" --format json || true)
        if [ "$native" = "$wasm" ]; then
            echo "✅ $q"
        else
            echo "❌ $q"; echo "   native: $native"; echo "   wasm:   $wasm"; fail=1
        fi
    done
    # Constant-stack proof (the wasm analog of PR #24's native ulimit test):
    # deep call/1 recursion must run under a *small* wasm stack via return_call
    # — without the musttail lowering it would overflow. PLG_MAX_STEPS must be
    # passed with --env because WASI does not inherit the host environment.
    printf 'count(0).\ncount(N) :- N > 0, N1 is N - 1, call(count(N1)).\n' > "$work/rec.pl"
    cargo run -q --features wasm -p patch-prolog-compiler --bin plgc -- \
        build "$work/rec.pl" -o "$work/rec.wasm" --target wasm32-wasi
    deep=$(wasmtime run --env PLG_MAX_STEPS=100000000 -W max-wasm-stack=1048576 \
        "$work/rec.wasm" --query "count(1000000)" --format text || true)
    if [ "$deep" = "true." ]; then
        echo "✅ constant stack: 1,000,000-deep call/1 under a 1MB wasm stack"
    else
        echo "❌ deep recursion expected 'true.', got '$deep'"; fail=1
    fi
    exit $fail

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
