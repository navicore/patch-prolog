//! clang driver: turns generated LLVM IR text into a standalone native
//! binary linked against the embedded `libplg_runtime.a`.
//!
//! Ported from patch-seq crates/compiler/src/lib.rs (the success
//! pattern): materialize the embedded archive at a content-addressed cache
//! path shared by every run of this build, invoke clang, dead-strip
//! unreachable runtime code.

use crate::{OptLevel, RUNTIME_LIB};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// Minimum clang/LLVM version required.
/// Generated IR uses opaque pointers (`ptr`), which requires LLVM 15+.
const MIN_CLANG_VERSION: u32 = 15;

static CLANG_VERSION_CHECKED: OnceLock<Result<u32, String>> = OnceLock::new();

/// Check that clang is available and meets the minimum version.
/// Cached — runs once per process.
pub fn check_clang_version() -> Result<u32, String> {
    CLANG_VERSION_CHECKED
        .get_or_init(|| {
            let output = Command::new("clang")
                .arg("--version")
                .output()
                .map_err(|e| {
                    format!(
                        "Failed to run clang: {e}.\n\
                         plgc needs clang {MIN_CLANG_VERSION}+ to link compiled binaries \
                         (the binaries themselves need nothing).\n\
                         Install it with:\n\
                         \x20 debian/ubuntu:  sudo apt install clang\n\
                         \x20 fedora:         sudo dnf install clang\n\
                         \x20 macOS:          xcode-select --install"
                    )
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "clang --version failed with exit code {:?}: {stderr}",
                    output.status.code(),
                ));
            }

            let version_str = String::from_utf8_lossy(&output.stdout);
            let version = parse_clang_version(&version_str).ok_or_else(|| {
                format!(
                    "Could not parse clang version from: {}\n\
                     plgc requires clang {MIN_CLANG_VERSION} or later (opaque pointer support).",
                    version_str.lines().next().unwrap_or(&version_str),
                )
            })?;

            // Apple clang versioning differs: Apple clang 14 is LLVM-15-based.
            let is_apple = version_str.contains("Apple clang");
            let effective_min = if is_apple { 14 } else { MIN_CLANG_VERSION };

            if version < effective_min {
                return Err(format!(
                    "clang version {version} detected, but plgc requires {} {effective_min} or later.\n\
                     The generated LLVM IR uses opaque pointers (requires LLVM 15+).",
                    if is_apple { "Apple clang" } else { "clang" },
                ));
            }

            Ok(version)
        })
        .clone()
}

/// Parse major version number from `clang --version` output.
fn parse_clang_version(output: &str) -> Option<u32> {
    for line in output.lines() {
        if line.contains("clang version")
            && let Some(idx) = line.find("version ")
        {
            let after_version = &line[idx + 8..];
            let major: String = after_version
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !major.is_empty() {
                return major.parse().ok();
            }
        }
    }
    None
}

static RUNTIME_EXTRACTED: OnceLock<Result<std::path::PathBuf, String>> = OnceLock::new();

/// Materialize the embedded runtime archive at a stable, content-addressed
/// cache path, reusing a previous extraction when the bytes already match.
///
/// The directory is keyed by a build-time hash of the archive bytes
/// (`PLG_RUNTIME_HASH`, emitted by build.rs), so every process of the same
/// plgc build shares ONE extraction — the pid-keyed scheme this replaces
/// left one ~21MB dir behind per invocation (issue #4). A build with a
/// different runtime gets a different directory, and stale siblings are
/// swept the first time the new build links. The archive keeps its
/// canonical `libplg_runtime.a` name (clang finds it via `-L <dir>
/// -lplg_runtime`); the hash lives in the directory name. The `OnceLock`
/// still memoizes within a process for parallel in-process compiles.
fn extracted_runtime() -> Result<std::path::PathBuf, String> {
    RUNTIME_EXTRACTED
        .get_or_init(|| {
            let base = cache_base();
            let dir = base.join(concat!("runtime-", env!("PLG_RUNTIME_HASH")));
            let path = dir.join("libplg_runtime.a");

            // Fast path: already extracted by an earlier run. The dir name
            // encodes the content hash; the length check guards against
            // exotic corruption (installs below are atomic renames, so a
            // partial file can never appear under the final name).
            if let Ok(meta) = fs::metadata(&path)
                && meta.len() == RUNTIME_LIB.len() as u64
            {
                return Ok(path);
            }

            fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to create runtime cache dir: {e}"))?;

            // Write under a process-unique temp name, then rename into
            // place. Renames are atomic, so concurrent first runs of the
            // same build race benignly: last one wins with identical bytes.
            let tmp = dir.join(format!(".libplg_runtime.a.{}", std::process::id()));
            fs::write(&tmp, RUNTIME_LIB)
                .map_err(|e| format!("Failed to write runtime lib: {e}"))?;
            fs::rename(&tmp, &path).map_err(|e| format!("Failed to install runtime lib: {e}"))?;

            sweep_stale_runtimes(&base, &dir);
            Ok(path)
        })
        .clone()
}

/// `$XDG_CACHE_HOME/plgc`, else `$HOME/.cache/plgc`, else a temp-dir
/// fallback for HOME-less environments.
fn cache_base() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME")
        && !xdg.is_empty()
    {
        return std::path::PathBuf::from(xdg).join("plgc");
    }
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return std::path::PathBuf::from(home).join(".cache").join("plgc");
    }
    std::env::temp_dir().join("plgc-cache")
}

/// Remove extractions left behind by other plgc builds. `cache_base()` is
/// plgc's own namespace, so reclaiming siblings is safe; racing a concurrent
/// plgc of a *different* build here is theoretical and self-healing (its
/// next link simply re-extracts).
fn sweep_stale_runtimes(base: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != keep
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("runtime-"))
        {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Link an LLVM IR file into a standalone executable against the
/// embedded runtime archive.
pub fn link_ir(ir_path: &Path, output_path: &Path, opt: OptLevel) -> Result<(), String> {
    check_clang_version()?;
    let runtime_path = extracted_runtime()?;

    let opt_flag = match opt {
        OptLevel::O0 => "-O0",
        OptLevel::O3 => "-O3",
    };

    let mut clang = Command::new("clang");
    clang.arg(opt_flag);
    // DWARF only in --debug builds: it multiplies binary size ~8x
    // (4.4M vs ~550K for hello-world) and the line info resolves into
    // the Rust runtime, not the user's .pl. Release binaries stay lean
    // (v1 shipped without debug info too).
    if opt == OptLevel::O0 {
        clang.arg("-g");
    }
    clang
        .arg(ir_path)
        .arg("-o")
        .arg(output_path)
        .arg("-L")
        .arg(runtime_path.parent().unwrap())
        .arg("-lplg_runtime")
        // libm: arithmetic builtins reach libm symbols via the runtime
        // archive; the link must be explicit. Harmless on macOS where
        // libm is part of libSystem.
        .arg("-lm");

    // Strip runtime code unreachable from the entry point so binaries
    // contain only what the program could execute.
    if cfg!(target_os = "macos") {
        clang.arg("-Wl,-dead_strip");
    } else if cfg!(target_os = "linux") {
        clang.arg("-Wl,--gc-sections");
        // The runtime archive's members carry Rust std DWARF; without
        // this the linker copies it all in (~3.8M on a ~550K binary).
        // --debug builds keep it.
        if opt != OptLevel::O0 {
            clang.arg("-Wl,--strip-debug");
        }
    }

    let output = clang
        .output()
        .map_err(|e| format!("Failed to run clang: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Clang compilation failed:\n{stderr}"));
    }

    Ok(())
}
