//! Compile the session buffer to a temporary native binary.
//!
//! `plgr` shells out to the `plgc` binary (`$PLGC` or PATH) rather than
//! linking `plg-compiler` in-process: linking would pull plgc's ~22M
//! embedded runtime archive into the `plgr` binary, so shelling keeps it a
//! thin driver (the LSP stays a frontend-only consumer for the same
//! reason). Either way the REPL only ever *compiles and execs*, never
//! interprets.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// A freshly compiled session binary. Holds its `TempDir` so the binary
/// stays on disk until this value is dropped (then it's cleaned up).
pub struct Compiled {
    pub binary: PathBuf,
    _dir: TempDir,
}

/// Compile `source` to a temp binary. `Err` carries the compiler's
/// stderr (parse/codegen/link failure) for display in the REPL.
pub fn compile(source: &str) -> Result<Compiled, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let src = dir.path().join("session.pl");
    std::fs::write(&src, source).map_err(|e| e.to_string())?;
    let binary = dir.path().join("session");

    let plgc = std::env::var("PLGC").unwrap_or_else(|_| "plgc".to_string());
    let output = Command::new(&plgc)
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .output()
        .map_err(|e| format!("could not run `{plgc}` (is it on PATH? set $PLGC): {e}"))?;

    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(msg.trim().to_string());
    }
    Ok(Compiled { binary, _dir: dir })
}
