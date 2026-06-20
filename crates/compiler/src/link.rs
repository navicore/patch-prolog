//! clang driver: turns generated LLVM IR text into a standalone native
//! binary linked against the embedded `libplg_runtime.a`.
//!
//! Ported from patch-seq crates/compiler/src/lib.rs (the success
//! pattern): materialize the embedded archive at a content-addressed cache
//! path shared by every run of this build, invoke clang, dead-strip
//! unreachable runtime code.

use crate::{OptLevel, RUNTIME_LIB};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

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

/// Orphaned `.libplg_runtime.a.<pid>` temps older than this are reclaimed: a
/// live extraction renames within milliseconds, so minutes of age means the
/// writer died mid-extraction (PR #5 review, finding 1).
const STALE_TEMP_AGE: Duration = Duration::from_secs(10 * 60);

/// Sibling `runtime-*` dirs (other plgc builds) older than this are
/// reclaimed. The week of grace keeps a concurrently *running* older build's
/// archive from being swept out from under its clang invocation (PR #5
/// review, finding 2); disk stays bounded at "builds actually used this
/// week".
const STALE_SIBLING_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The materialized runtime archive a link uses. Kept alive across the clang
/// invocation; the ephemeral variant removes its extraction on drop.
enum RuntimeArchive {
    /// Shared content-addressed cache entry — persists by design.
    Cached(std::path::PathBuf),
    /// HOME-less/XDG-less fallback: a private per-link extraction. A shared
    /// dir under the world-writable temp root would have a predictable,
    /// poisonable name — and a non-cryptographic content key can't make a
    /// reuse check adversary-proof — so we don't share there at all
    /// (PR #5 review, finding 3).
    Ephemeral(#[allow(dead_code)] tempfile::TempDir, std::path::PathBuf),
}

impl RuntimeArchive {
    fn lib_path(&self) -> &Path {
        match self {
            RuntimeArchive::Cached(path) => path,
            RuntimeArchive::Ephemeral(_, path) => path,
        }
    }
}

/// Materialize the embedded runtime archive; the handle stays valid for the
/// duration of one link.
///
/// When a per-user cache location exists, the archive lives at a stable,
/// content-addressed path keyed by a build-time hash of its bytes
/// (`PLG_RUNTIME_HASH`, emitted by build.rs): every process of the same plgc
/// build shares ONE extraction — the pid-keyed scheme this replaces left one
/// ~21MB dir behind per invocation (issue #4). The archive keeps its
/// canonical `libplg_runtime.a` name (clang finds it via `-L <dir>
/// -lplg_runtime`); the hash lives in the directory name. The `OnceLock`
/// memoizes within a process for parallel in-process compiles, and each
/// process performs one age-gated hygiene sweep of stale leftovers.
fn extracted_runtime() -> Result<RuntimeArchive, String> {
    let Some(base) = cache_base() else {
        // No private per-user location exists: extract for this link only
        // and let `TempDir` reclaim it on drop. ~21MB per link is acceptable
        // in this rare environment; a poisonable shared path is not.
        let dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
        let path = dir.path().join("libplg_runtime.a");
        fs::write(&path, RUNTIME_LIB).map_err(|e| format!("Failed to write runtime lib: {e}"))?;
        return Ok(RuntimeArchive::Ephemeral(dir, path));
    };

    RUNTIME_EXTRACTED
        .get_or_init(|| {
            let dir = base.join(concat!("runtime-", env!("PLG_RUNTIME_HASH")));
            let path = dir.join("libplg_runtime.a");

            // Hygiene before the fast path, so aged orphans are reclaimed on
            // warm runs too (the common case).
            sweep_stale(&base, &dir);

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

            Ok(path)
        })
        .clone()
        .map(RuntimeArchive::Cached)
}

/// `$XDG_CACHE_HOME/plgc`, else `$HOME/.cache/plgc`; `None` when neither is
/// available — there is no private per-user place to cache in (see
/// `RuntimeArchive::Ephemeral`).
fn cache_base() -> Option<std::path::PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME")
        && !xdg.is_empty()
    {
        return Some(std::path::PathBuf::from(xdg).join("plgc"));
    }
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return Some(std::path::PathBuf::from(home).join(".cache").join("plgc"));
    }
    None
}

/// Cache hygiene, once per process: reclaim sibling `runtime-*` dirs left by
/// other plgc builds, and orphaned `.libplg_runtime.a.*` temps in our own
/// dir (writers killed between write and rename). Both sweeps are age-gated
/// so nothing in active use is ever touched.
fn sweep_stale(base: &Path, keep: &Path) {
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path != keep
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("runtime-"))
                && older_than(&path, STALE_SIBLING_AGE)
            {
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(keep) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".libplg_runtime.a."))
                && older_than(&path, STALE_TEMP_AGE)
            {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn older_than(path: &Path, age: Duration) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|elapsed| elapsed > age)
}

/// Link an LLVM IR file into a standalone executable against the
/// embedded runtime archive.
pub fn link_ir(ir_path: &Path, output_path: &Path, opt: OptLevel) -> Result<(), String> {
    check_clang_version()?;
    // Bound to a local so an ephemeral extraction outlives the clang call.
    let runtime = extracted_runtime()?;

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
        .arg(runtime.lib_path().parent().unwrap())
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

/// Link an LLVM IR file into a standalone `wasm32-wasi` module (Tier 1 — see
/// docs/design/WASM.md), using the Rust-bundled `llc` / `wasm-ld` and the wasm
/// target's self-contained wasi-libc — no wasi-sdk.
///
/// The musttail chains that keep recursion in constant stack require the wasm
/// tail-call feature: `llc -mattr=+tail-call` lowers them to `return_call`.
/// Without it `llc` errors out — it never silently emits a non-tail call — so a
/// misconfigured toolchain fails loudly at build time, not as a runtime
/// stack overflow.
pub fn link_wasm(ir_path: &Path, output_path: &Path, opt: OptLevel) -> Result<(), String> {
    let wasm_runtime = crate::WASM_RUNTIME_LIB.ok_or_else(|| {
        "this plgc was built without wasm support.\n\
         Reinstall with the wasm runtime embedded:  just install-wasm\n\
         (or: cargo install --features wasm --path crates/compiler)"
            .to_string()
    })?;
    let (llc, lld, self_contained) = wasm_toolchain()?;

    // Work area: the intermediate object and a materialized copy of the
    // embedded wasm runtime archive (wasm-ld reads it from disk).
    let work = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let obj = work.path().join("prog.o");
    let runtime = work.path().join("libplg_runtime.a");
    fs::write(&runtime, wasm_runtime).map_err(|e| format!("Failed to write wasm runtime: {e}"))?;

    // llc's -O3 buys little for our IR and -O2 is what the gate proved; -O0
    // for --debug. Tail calls are honoured at every level (musttail is
    // mandatory), so opt level never affects stack safety.
    let opt_flag = match opt {
        OptLevel::O0 => "-O0",
        OptLevel::O3 => "-O2",
    };

    // 1) IR → wasm object, tail calls enabled.
    let llc_out = Command::new(&llc)
        .args(["-mtriple=wasm32-wasi", "-mattr=+tail-call", "-filetype=obj"])
        .arg(opt_flag)
        .arg(ir_path)
        .arg("-o")
        .arg(&obj)
        .output()
        .map_err(|e| format!("Failed to run llc: {e}"))?;
    if !llc_out.status.success() {
        return Err(format!(
            "llc (wasm) failed:\n{}",
            String::from_utf8_lossy(&llc_out.stderr)
        ));
    }

    // 2) Link crt + object + runtime + wasi-libc into a command module.
    // `crt1-command.o`'s `_start` → `__main_void` → our `__main_argc_argv`.
    let lld_out = Command::new(&lld)
        .args(["-flavor", "wasm"])
        .arg("-L")
        .arg(&self_contained)
        .arg(self_contained.join("crt1-command.o"))
        .arg(&obj)
        .arg(&runtime)
        .arg("-lc")
        // Runtime imports (the wasi syscalls the runtime calls) resolve at
        // instantiation, not link — so a genuinely missing import surfaces
        // when the module is *run*, not here. `just wasm-smoke` runs the
        // module, which is what verifies the imports end to end.
        .arg("--allow-undefined")
        .arg("-o")
        .arg(output_path)
        .output()
        .map_err(|e| format!("Failed to run wasm-ld (rust-lld): {e}"))?;
    if !lld_out.status.success() {
        return Err(format!(
            "wasm-ld failed:\n{}",
            String::from_utf8_lossy(&lld_out.stderr)
        ));
    }

    Ok(())
}

/// The reactor's host-facing exports. `plg_init` builds the Machine (the JS
/// host calls it once); `plg_rt_alloc`/`plg_rt_free` manage linear-memory
/// buffers; `plg_rt_run_query` answers a query. `plg_rt_set_machine` is NOT
/// here — it's internal to `plg_init`. wasm-ld treats these as the only roots
/// (`--no-entry`), GCing everything else they don't reach.
const REACTOR_EXPORTS: &[&str] = &[
    "plg_init",
    "plg_rt_run_query",
    "plg_rt_alloc",
    "plg_rt_free",
];

/// Link an LLVM IR file into a `wasm32-unknown-unknown` *reactor* module
/// (Tier 2 — docs/design/WASM_TIER2_PLAN.md C1): no WASI, no crt, no libc — the
/// module exports `plg_init` + the buffer ABI a JS host (Cloudflare Workers /
/// V8) drives. Reuses the same Rust-bundled `llc`/`wasm-ld` as Tier 1; only the
/// archive and the link flags differ (`--no-entry` + the explicit exports).
///
/// As with Tier 1, `-mattr=+tail-call` is what lowers the musttail chains to
/// `return_call`; without it `llc` errors at build time rather than emitting a
/// stack-overflowing module. This was the load-bearing finding the gate proved
/// on V8 at 1,000,000-deep recursion.
pub fn link_wasm_reactor(ir_path: &Path, output_path: &Path, opt: OptLevel) -> Result<(), String> {
    let worker_runtime = crate::WORKER_RUNTIME_LIB.ok_or_else(|| {
        "this plgc was built without wasm support.\n\
         Reinstall with the wasm runtimes embedded:  just install-wasm\n\
         (or: cargo install --features wasm --path crates/compiler)"
            .to_string()
    })?;
    let (llc, lld) = llvm_tools()?;

    let work = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let obj = work.path().join("prog.o");
    let runtime = work.path().join("libplg_runtime.a");
    fs::write(&runtime, worker_runtime)
        .map_err(|e| format!("Failed to write reactor runtime: {e}"))?;

    let opt_flag = match opt {
        OptLevel::O0 => "-O0",
        OptLevel::O3 => "-O2",
    };

    // 1) IR → wasm object, tail calls enabled.
    let llc_out = Command::new(&llc)
        .args([
            "-mtriple=wasm32-unknown-unknown",
            "-mattr=+tail-call",
            "-filetype=obj",
        ])
        .arg(opt_flag)
        .arg(ir_path)
        .arg("-o")
        .arg(&obj)
        .output()
        .map_err(|e| format!("Failed to run llc: {e}"))?;
    if !llc_out.status.success() {
        return Err(format!(
            "llc (wasm reactor) failed:\n{}",
            String::from_utf8_lossy(&llc_out.stderr)
        ));
    }

    // 2) Link the object + reactor runtime into a reactor module. No crt/libc:
    // `--no-entry` means the module has no `_start`; the host drives the
    // exports directly. `--allow-undefined` defers any runtime import to
    // instantiation (mirrors Tier 1) — a genuinely missing import then surfaces
    // when the host instantiates the module, which `just reactor-smoke` does
    // (it also asserts the four exports are present, since `--allow-undefined`
    // would otherwise degrade a missing export to a silent import).
    let mut lld_cmd = Command::new(&lld);
    lld_cmd.args(["-flavor", "wasm", "--no-entry", "--allow-undefined"]);
    for sym in REACTOR_EXPORTS {
        lld_cmd.arg(format!("--export={sym}"));
    }
    let lld_out = lld_cmd
        .arg(&obj)
        .arg(&runtime)
        .arg("-o")
        .arg(output_path)
        .output()
        .map_err(|e| format!("Failed to run wasm-ld (rust-lld): {e}"))?;
    if !lld_out.status.success() {
        return Err(format!(
            "wasm-ld (reactor) failed:\n{}",
            String::from_utf8_lossy(&lld_out.stderr)
        ));
    }

    Ok(())
}

/// Locate the Rust-bundled LLVM tools (`llc`, `rust-lld`) and the
/// `wasm32-wasip1` self-contained wasi-libc, all under `rustc --print sysroot`.
/// Returns `(llc, rust-lld, self-contained-dir)`. Errors with the exact rustup
/// command to run when a piece is missing.
///
/// STRATEGY: this tracks rustup's `<sysroot>/lib/rustlib/<host>/bin/` and
/// `.../<target>/lib/self-contained/` layout. That layout has been stable for
/// years and Rust is unlikely to move it without notice, but if it ever does,
/// the failure here is a "file not found" that reads like a broken install.
/// The fallback, should that happen, is to discover the tools via `llvm-config`
/// or switch the wasm link path to the wasi-sdk distribution.
/// Locate the Rust-bundled LLVM tools (`llc`, `rust-lld`) under
/// `<sysroot>/lib/rustlib/<host>/bin/`. Shared by both wasm link paths — Tier 1
/// (`wasm_toolchain`, which also needs wasi-libc) and the Tier-2 reactor
/// (`link_wasm_reactor`, which needs no libc at all).
fn llvm_tools() -> Result<(PathBuf, PathBuf), String> {
    let sysroot = rustc_print(&["--print", "sysroot"])?;
    let host = rustc_print(&["--print", "host-tuple"])?;
    let bin = Path::new(&sysroot)
        .join("lib/rustlib")
        .join(&host)
        .join("bin");
    let llc = bin.join("llc");
    let lld = bin.join("rust-lld");
    if !llc.exists() || !lld.exists() {
        return Err(format!(
            "wasm target needs the LLVM tools that ship with rustup:\n  \
             rustup component add llvm-tools-preview\n\
             (looked under {})",
            bin.display()
        ));
    }
    Ok((llc, lld))
}

fn wasm_toolchain() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let (llc, lld) = llvm_tools()?;
    let sysroot = rustc_print(&["--print", "sysroot"])?;
    let self_contained = Path::new(&sysroot).join("lib/rustlib/wasm32-wasip1/lib/self-contained");
    // Distinguish "target not added" from "target partially installed" so the
    // recovery hint is right for each.
    if !self_contained.exists() {
        return Err("wasm target not installed (wasm32-wasip1 std):\n  \
             rustup target add wasm32-wasip1"
            .to_string());
    }
    if !self_contained.join("libc.a").exists() {
        return Err(format!(
            "wasm32-wasip1 std looks partially installed — wasi-libc (libc.a) \
             missing under {}.\n  \
             Try: rustup target remove wasm32-wasip1 && rustup target add wasm32-wasip1",
            self_contained.display()
        ));
    }
    Ok((llc, lld, self_contained))
}

fn rustc_print(args: &[&str]) -> Result<String, String> {
    let out = Command::new("rustc")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run rustc (needed for the wasm target): {e}"))?;
    if !out.status.success() {
        return Err(format!("rustc {args:?} failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
