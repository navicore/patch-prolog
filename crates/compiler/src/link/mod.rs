//! clang driver: turns generated LLVM IR text into a standalone native
//! binary linked against the embedded `libplg_runtime.a`. The runtime
//! archive's cache lives in `archive.rs`; the wasm link paths in `wasm.rs`.
//!
//! Ported from patch-seq crates/compiler/src/lib.rs (the success
//! pattern): materialize the embedded archive at a content-addressed cache
//! path shared by every run of this build, invoke clang, dead-strip
//! unreachable runtime code.

mod archive;
mod wasm;

pub use wasm::{link_wasm, link_wasm_reactor};

use crate::OptLevel;
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

/// Link an LLVM IR file into a standalone executable against the
/// embedded runtime archive.
pub fn link_ir(ir_path: &Path, output_path: &Path, opt: OptLevel) -> Result<(), String> {
    check_clang_version()?;
    // Bound to a local so an ephemeral extraction outlives the clang call.
    let runtime = archive::extracted_runtime()?;

    let opt_flag = match opt {
        OptLevel::O0 => "-O0",
        OptLevel::O3 => "-O3",
    };

    let mut clang = Command::new("clang");
    clang.arg(opt_flag);
    // DWARF only in --debug builds: it multiplies binary size ~8x
    // (4.4M vs ~550K for hello-world) and the line info resolves into
    // the Rust runtime, not the user's .pl. Release binaries stay lean.
    if opt == OptLevel::O0 {
        clang.arg("-g");
    }
    clang
        .arg(ir_path)
        .arg("-o")
        .arg(output_path)
        .arg("-L")
        .arg(
            runtime
                .lib_path()
                .parent()
                .expect("archive path always has a parent"),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_clang_version() {
        assert_eq!(
            parse_clang_version("Debian clang version 15.0.6\nTarget: x86_64-pc-linux-gnu\n"),
            Some(15)
        );
        assert_eq!(
            parse_clang_version("Ubuntu clang version 18.1.3 (1ubuntu1)\n"),
            Some(18)
        );
    }

    #[test]
    fn parses_apple_clang_version() {
        // Apple clang numbers its own line; the LLVM major still follows
        // "version " (and check_clang_version applies Apple's lower floor).
        assert_eq!(
            parse_clang_version("Apple clang version 15.0.0 (clang-1500.1.0.2.5)\n"),
            Some(15)
        );
    }

    #[test]
    fn rejects_non_clang_and_malformed_output() {
        assert_eq!(parse_clang_version("gcc (Debian) 12.2.0\n"), None);
        assert_eq!(parse_clang_version("clang version\n"), None);
        assert_eq!(parse_clang_version(""), None);
    }
}
