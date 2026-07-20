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

# Build the runtime for wasm32-wasip1 (Tier 1). The
# archive (target/wasm32-wasip1/release/libplg_runtime.a) is embedded into a
# wasm-enabled plgc by build.rs under `--features wasm`. Needs the wasm target:
#   rustup target add wasm32-wasip1
build-runtime-wasm:
    @echo "Building wasm runtime (wasm32-wasip1)..."
    cargo build --locked --release -p patch-prolog-runtime --target wasm32-wasip1
    @echo "✅ Wasm runtime built: target/wasm32-wasip1/release/libplg_runtime.a"

# Build the runtime for wasm32-unknown-unknown (Tier 2 reactor). The archive
# (target/wasm32-unknown-unknown/release/libplg_runtime.a) is embedded into a
# wasm-enabled plgc by build.rs under `--features wasm`. Needs the target:
#   rustup target add wasm32-unknown-unknown
build-runtime-wasm-reactor:
    @echo "Building reactor runtime (wasm32-unknown-unknown)..."
    cargo build --locked --release -p patch-prolog-runtime --target wasm32-unknown-unknown
    @echo "✅ Reactor runtime built: target/wasm32-unknown-unknown/release/libplg_runtime.a"

# Build BOTH wasm archives. build.rs embeds both whenever `--features wasm` is
# set, panicking if either is missing — so EVERY `--features wasm` consumer
# (lint, both smokes) must depend on this, not just the archive it happens to
# use. `just` dedups the shared prereq, so each archive still builds once.
build-runtime-wasm-all: build-runtime-wasm build-runtime-wasm-reactor

# Install a wasm-capable plgc: builds BOTH wasm runtimes, then installs plgc
# with the `wasm` feature so it can emit `--target wasm32-wasi` (Tier 1) and
# `--target worker` (Tier 2 reactor) modules. Also needs the rustup llvm-tools
# (llc/wasm-ld): rustup component add llvm-tools-preview
install-wasm: build-runtime-wasm-all
    @echo "Installing wasm-capable compiler (plgc --features wasm)..."
    cargo install --path crates/compiler --features wasm --force
    @echo "✅ Installed plgc with wasm support"

# Tier 1 wasm smoke gate (run in the separate wasm workflow, not main `just
# ci`). Needs: the wasm32-wasip1 target, llvm-tools-preview, and wasmtime on
# PATH. Compiles an example to wasm and asserts it answers --query
# byte-identically to native.
wasm-smoke: build-runtime-wasm-all
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
        native=$("$work/deps-native" --query "$q" --format text || true)
        wasm=$(wasmtime run "$work/deps.wasm" --query "$q" --format text || true)
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

# Tier 2 reactor smoke gate (run in the separate wasm workflow, not main `just
# ci`). Needs: the wasm32-unknown-unknown target, llvm-tools-preview, and node
# on PATH. Compiles an example to a reactor module, instantiates it under Node's
# V8 (the Workers engine), asserts the four host exports exist, and round-trips
# queries byte-identically to native — modulo the reactor's always-present
# "output" field (D4), which is stripped before the compare. Then proves the
# musttail→return_call lowering holds on V8 at 1,000,000-deep recursion.
# Tier 2 reactor smoke (run in the separate wasm workflow, not main `just ci`).
# Needs: wasm32-unknown-unknown, llvm-tools-preview, and node on PATH. Compiles
# examples/deps.pl to a reactor module, instantiates it under Node's V8 (the
# Workers engine), and asserts the bson→JSON host glue against fixtures
# (scripts/reactor-smoke.mjs). Then proves the musttail→return_call lowering
# holds on V8 at 1,000,000-deep recursion.
wasm-reactor-smoke: build-runtime-wasm-all
    #!/usr/bin/env bash
    set -euo pipefail
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    echo "Compiling examples/deps.pl → reactor..."
    cargo run -q --features wasm -p patch-prolog-compiler --bin plgc -- \
        build examples/deps.pl -o "$work/deps.worker.wasm" --target worker
    # Fixture mode: the driver asserts the bson→JSON decode for known queries.
    node scripts/reactor-smoke.mjs "$work/deps.worker.wasm"
    # Constant-stack proof on V8 (the headline gate finding): 1,000,000-deep
    # call/1 recursion returns in a V8 isolate via return_call. A high
    # per-request step_limit (3rd arg) keeps the step ceiling from tripping.
    printf 'count(0).\ncount(N) :- N > 0, N1 is N - 1, call(count(N1)).\n' > "$work/rec.pl"
    cargo run -q --features wasm -p patch-prolog-compiler --bin plgc -- \
        build "$work/rec.pl" -o "$work/rec.worker.wasm" --target worker
    deep=$(node scripts/reactor-smoke.mjs "$work/rec.worker.wasm" "count(1000000)" 100000000)
    if [ "$deep" = '{"count":1,"exhausted":true,"output":"","solutions":[{}]}' ]; then
        echo "✅ constant stack: 1,000,000-deep call/1 in a V8 isolate (return_call)"
    else
        echo "❌ deep recursion unexpected: $deep"; exit 1
    fi

# Compile a .pl to a reactor module and serve it on local workerd (Tier 2,
# WASM_TIER2_PLAN.md D2g). Needs: wasm32-unknown-unknown, llvm-tools-preview,
# and `workerd` on PATH (npm i -g workerd). Emits the glue (worker.js /
# wrangler.toml / config.capnp) into target/worker/<stem>/ and serves from
# there. Query it with:  curl 'http://localhost:8080/?query=<goal>'
# Example:  just wasm-worker-serve examples/deps.pl
wasm-worker-serve prog: build-runtime-wasm-reactor
    #!/usr/bin/env bash
    set -euo pipefail
    stem=$(basename "{{prog}}" .pl)
    out="target/worker/$stem"
    mkdir -p "$out"
    echo "Compiling {{prog}} → $out/$stem.worker.wasm ..."
    cargo run -q --features wasm -p patch-prolog-compiler --bin plgc -- \
        build "{{prog}}" -o "$out/$stem.worker.wasm" --target worker
    echo "Serving $stem on http://localhost:8080"
    echo "  try:  curl 'http://localhost:8080/?query=<goal>'"
    cd "$out"
    exec workerd serve config.capnp

# All wasm gates, both tiers (Tier 1 wasi + Tier 2 reactor). The separate wasm
# CI workflow (.forgejo/workflows/wasm.yml) calls this; it is NOT part of the
# main `just ci` because it needs a wasm toolchain (wasm rustup targets,
# llvm-tools-preview, wasmtime, node) the base CI image may lack — so a missing
# piece fails THIS gate without breaking the core build/test/lint.
wasm-ci: wasm-lint wasm-smoke wasm-reactor-smoke
    @echo "✅ wasm gates passed (Tier 1 wasi + Tier 2 reactor)"

# Clippy over the wasm-feature-gated compiler code (worker glue, reactor link,
# embedded archives) — the default `just lint` doesn't enable the feature, so
# this is the only thing that type-checks/lints those paths.
wasm-lint: build-runtime-wasm-all
    @echo "Linting wasm-feature code..."
    cargo clippy --locked --features wasm -p patch-prolog-compiler --all-targets -- -D warnings

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

# Audit dependency licenses against deny.toml (requires cargo-deny)
license-audit:
    @echo "Auditing dependency licenses..."
    cargo deny check licenses
    @echo "✅ License audit passed!"

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
ci: fmt-check lint license-audit test build build-examples test-integration lint-pl check-binary-contents
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

# Regenerate generated docs (docs/README.md) from the root README
gen-docs:
    ./scripts/generate-docs.sh

# Build the mdbook documentation site into ./book/ (what CI publishes)
docs: gen-docs
    @echo "Building documentation..."
    mdbook build
    @echo "✅ Documentation built in book/"

# Serve the docs locally with live reload
docs-serve: gen-docs
    mdbook serve --open
