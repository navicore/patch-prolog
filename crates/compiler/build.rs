//! Build script for plg-compiler
//!
//! Locates the plg-runtime static library so it can be embedded into the
//! compiler binary via `include_bytes!`. Ported from patch-seq's proven
//! pattern (crates/compiler/build.rs there).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn main() {
    // Verify that plg-runtime version matches compiler version
    verify_runtime_version();

    // Rerun verification if Cargo.toml changes. Also watch the runtime
    // crate's sources so this script re-embeds when the runtime changes,
    // independent of the (reliable) build-dependency relink trigger.
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../runtime/src");
    println!("cargo:rerun-if-changed=../runtime/Cargo.toml");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let target_dir = out_dir
        .parent() // build/<pkg>-<hash>/out -> build/<pkg>-<hash>
        .and_then(|p| p.parent()) // -> build
        .and_then(|p| p.parent()) // -> <profile> (release/debug)
        .expect("Could not find target directory");

    // Pick the runtime staticlib cargo just built. We MUST NOT prefer the
    // uplifted top-level `target/<profile>/libplg_runtime.a`: that name is
    // only refreshed by a *primary* build of plg-runtime, so when plgc is
    // built alone (plg-runtime compiled as its build-dependency) it lands in
    // `deps/libplg_runtime-<hash>.a` and the top-level copy goes stale —
    // embedding old runtime bytes. cargo never GCs old hashed copies, so we
    // take the newest by mtime (the one written for this build), with a
    // deterministic filename tie-break. The uplifted copy is only a
    // last-resort fallback for layouts where deps/ has none.
    let deps_dir = target_dir.join("deps");
    let runtime_lib = newest_runtime_lib(&deps_dir)
        .or_else(|| {
            let direct = target_dir.join("libplg_runtime.a");
            direct.exists().then_some(direct)
        })
        .unwrap_or_else(|| {
            panic!(
                "Runtime library not found.\n\
                 Looked in deps: {}\n\
                 And: {}\n\
                 OUT_DIR was: {}",
                deps_dir.display(),
                target_dir.join("libplg_runtime.a").display(),
                out_dir.display()
            )
        });

    // Set environment variable for include_bytes! in lib.rs
    println!(
        "cargo:rustc-env=PLG_RUNTIME_LIB_PATH={}",
        runtime_lib.display()
    );

    // Content-hash the archive and bake the digest in. link.rs keys its
    // shared extraction cache (cache_base()/runtime-<hash>/) on it: the key
    // changes exactly when the embedded bytes change, and identical rebuilds
    // keep reusing the same extraction. (The cargo version is NOT a valid
    // key — dev rebuilds embed different bytes under the same version.)
    let runtime_bytes = fs::read(&runtime_lib).expect("Failed to read runtime lib for hashing");
    println!(
        "cargo:rustc-env=PLG_RUNTIME_HASH={:016x}",
        fnv1a64(&runtime_bytes)
    );

    // Rerun if the runtime library changes
    println!("cargo:rerun-if-changed={}", runtime_lib.display());

    // Wasm Tier 1 (docs/design/WASM.md): when built `--features wasm`, also
    // embed the `wasm32-wasip1` runtime archive, built out-of-band by
    // `just build-runtime-wasm`. Behind the feature so the default
    // `cargo install plgc` is byte-for-byte unchanged and needs no wasm
    // toolchain. The wasm archive is a *primary* build of plg-runtime, so its
    // uplifted top-level name is reliable (unlike the native deps/ case above).
    if env::var_os("CARGO_FEATURE_WASM").is_some() {
        let target_root = target_dir.parent().expect("target root");
        let wasm_lib = target_root.join("wasm32-wasip1/release/libplg_runtime.a");
        if !wasm_lib.exists() {
            panic!(
                "feature `wasm` is enabled but the wasm runtime archive is missing:\n  {}\n\
                 Build it first:  just build-runtime-wasm",
                wasm_lib.display()
            );
        }
        println!(
            "cargo:rustc-env=PLG_WASM_RUNTIME_LIB_PATH={}",
            wasm_lib.display()
        );
        println!("cargo:rerun-if-changed={}", wasm_lib.display());
    }
}

/// FNV-1a, 64-bit: tiny, dependency-free, and deterministic across builds,
/// platforms, and Rust versions — which is all a cache key needs (this is
/// invalidation, not cryptography).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Newest `libplg_runtime-*.a` in `deps/` by modification time.
///
/// Guarantee this provides: cargo compiles build-dependencies before running
/// this script, so it has just (re)written *this build's* runtime artifact —
/// it is therefore the newest among the artifacts cargo touched, and that is
/// what we embed. Guarantee it does NOT provide: a positive identity. We do
/// not confirm this is the exact artifact cargo's metadata pinned for this
/// build (the principled key is the build-dep's metadata hash in the
/// filename). In a workspace where a sibling unit recompiles `plg-runtime`
/// under a different hash, mtime could in principle prefer that copy. The
/// airtight fix is artifact dependencies (`artifact = "staticlib"`), still
/// nightly-only; mtime is the right shape for stable today.
///
/// Ties (identical mtime) break on the lexically-LARGER path — arbitrary but
/// deterministic; direction chosen only so the rule is total.
fn newest_runtime_lib(deps_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(deps_dir).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("libplg_runtime") && name.ends_with(".a")) {
            continue;
        }
        let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let path = entry.path();
        let better = match &best {
            None => true,
            Some((t, p)) => (mtime, &path) > (*t, p),
        };
        if better {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Verify that the plg-runtime version matches the plg-compiler version
/// by parsing this crate's Cargo.toml. The embedded runtime MUST match
/// the compiler version so published packages are trustworthy.
fn verify_runtime_version() {
    let compiler_version = env!("CARGO_PKG_VERSION");

    let cargo_toml_content =
        fs::read_to_string("Cargo.toml").expect("Failed to read compiler/Cargo.toml");

    let cargo_toml: toml::Value =
        toml::from_str(&cargo_toml_content).expect("Failed to parse Cargo.toml");

    let runtime_version = cargo_toml
        .get("build-dependencies")
        .and_then(|deps| deps.get("plg-runtime"))
        .and_then(|dep| match dep {
            toml::Value::Table(t) => t.get("version").and_then(|v| v.as_str()),
            toml::Value::String(s) => Some(s.as_str()),
            _ => None,
        })
        .expect("Could not find plg-runtime version in Cargo.toml");

    let runtime_version = runtime_version.trim_start_matches('=');

    if compiler_version != runtime_version {
        panic!(
            "\n\nVERSION MISMATCH: plg-compiler is {compiler_version} but \
             build-dependencies pin plg-runtime to {runtime_version}.\n\
             The embedded runtime MUST match the compiler version.\n\
             Update crates/compiler/Cargo.toml to: version = \"={compiler_version}\"\n"
        );
    }
}
