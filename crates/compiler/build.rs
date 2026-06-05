//! Build script for plg-compiler
//!
//! Locates the plg-runtime static library so it can be embedded into the
//! compiler binary via `include_bytes!`. Ported from patch-seq's proven
//! pattern (crates/compiler/build.rs there).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Verify that plg-runtime version matches compiler version
    verify_runtime_version();

    // Rerun verification if Cargo.toml changes
    println!("cargo:rerun-if-changed=Cargo.toml");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    // We need libplg_runtime.a in:
    // target/<profile>/libplg_runtime.a or target/<profile>/deps/libplg_runtime-*.a
    let target_dir = out_dir
        .parent() // build/<pkg>-<hash>/out -> build/<pkg>-<hash>
        .and_then(|p| p.parent()) // -> build
        .and_then(|p| p.parent()) // -> <profile> (release/debug)
        .expect("Could not find target directory");

    let direct_lib = target_dir.join("libplg_runtime.a");

    let runtime_lib = if direct_lib.exists() {
        direct_lib
    } else {
        let deps_dir = target_dir.join("deps");
        find_runtime_in_deps(&deps_dir).unwrap_or_else(|| {
            panic!(
                "Runtime library not found.\n\
                 Looked in: {}\n\
                 And deps: {}\n\
                 OUT_DIR was: {}",
                direct_lib.display(),
                deps_dir.display(),
                out_dir.display()
            )
        })
    };

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

fn find_runtime_in_deps(deps_dir: &PathBuf) -> Option<PathBuf> {
    if !deps_dir.exists() {
        return None;
    }

    fs::read_dir(deps_dir).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("libplg_runtime") && name_str.ends_with(".a") {
            Some(entry.path())
        } else {
            None
        }
    })
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
