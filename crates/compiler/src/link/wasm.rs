//! wasm linking (Tier 1 `wasm32-wasi` CLI module + Tier 2
//! `wasm32-unknown-unknown` reactor) using the Rust-bundled `llc` /
//! `wasm-ld` and — for Tier 1 — the wasm target's self-contained
//! wasi-libc. No wasi-sdk required.

use crate::OptLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// IR → wasm object via `llc`, tail calls enabled: the musttail chains that
/// keep recursion in constant stack require `+tail-call` to lower them to
/// `return_call`. Without it `llc` errors out — it never silently emits a
/// non-tail call — so a misconfigured toolchain fails loudly at build time,
/// not as a runtime stack overflow. `label` names the tier in errors.
fn llc_to_obj(
    llc: &Path,
    mtriple: &str,
    label: &str,
    ir_path: &Path,
    obj: &Path,
    opt_flag: &str,
) -> Result<(), String> {
    let mut cmd = Command::new(llc);
    cmd.arg(format!("-mtriple={mtriple}"))
        .arg("-mattr=+tail-call")
        .arg("-filetype=obj")
        .arg(opt_flag)
        .arg(ir_path)
        .arg("-o")
        .arg(obj);
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to run llc: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "llc ({label}) failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Link an LLVM IR file into a standalone `wasm32-wasi` module (Tier 1),
/// using the Rust-bundled `llc` / `wasm-ld` and the wasm
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
    llc_to_obj(&llc, "wasm32-wasi", "wasm", ir_path, &obj, opt_flag)?;

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
    "plg_rt_atom_name",
];

/// Link an LLVM IR file into a `wasm32-unknown-unknown` *reactor* module
/// (Tier 2): no WASI, no crt, no libc — the
/// module exports `plg_init` + the buffer ABI a JS host (Cloudflare Workers /
/// V8) drives. Reuses the same Rust-bundled `llc`/`wasm-ld` as Tier 1; only the
/// archive and the link flags differ (`--no-entry` + the explicit exports).
///
/// As with Tier 1, `-mattr=+tail-call` is what lowers the musttail chains to
/// `return_call`; without it `llc` errors at build time rather than emitting
/// a stack-overflowing module. This was the load-bearing finding the gate proved
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
    llc_to_obj(
        &llc,
        "wasm32-unknown-unknown",
        "wasm reactor",
        ir_path,
        &obj,
        opt_flag,
    )?;

    // 2) Link the object + reactor runtime into a reactor module. No crt/libc:
    // `--no-entry` means the module has no `_start`; the host drives the
    // exports directly. `--allow-undefined` defers any runtime import to
    // instantiation (mirrors Tier 1) — a genuinely missing import then surfaces
    // when the host instantiates the module, which `just wasm-reactor-smoke`
    // does (it also asserts the four exports are present, since
    // `--allow-undefined` would otherwise degrade a missing export to a silent
    // import).
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

/// Locate the Rust-bundled LLVM tools (`llc`, `rust-lld`) under
/// `<sysroot>/lib/rustlib/<host>/bin/`. Shared by both wasm link paths —
/// Tier 1 (`wasm_toolchain`, which also needs wasi-libc) and the Tier-2
/// reactor (which needs no libc at all). Errors with the exact rustup
/// command to run when a piece is missing.
///
/// STRATEGY: this tracks rustup's layout, which has been stable for years;
/// if Rust ever moves it, the failure here is a "file not found" that reads
/// like a broken install. The fallback, should that happen, is to discover
/// the tools via `llvm-config` or switch the wasm link path to the
/// wasi-sdk distribution.
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
