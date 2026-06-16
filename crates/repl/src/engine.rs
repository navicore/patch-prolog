//! Compile the session buffer to a temporary native binary.
//!
//! Scaffold strategy: shell out to the `plgc` binary (`$PLGC` or PATH).
//! The design target (docs/design/REPL.md) is to link `plg-compiler`
//! in-process — for instant parse/codegen errors and the phase-2 IR
//! panel — but that couples the build to the embedded runtime archive.
//! Shelling keeps the scaffold simple and is a drop-in to replace later;
//! either way the REPL only ever *compiles and execs*, never interprets.

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
