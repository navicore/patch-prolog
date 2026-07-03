//! Shared integration-test harness: compile a fixture program ONCE per
//! program (clang dominates test time), then run many `--query` calls
//! against the produced binary.

use std::path::PathBuf;
use std::process::Command;

pub struct Compiled {
    pub _dir: tempfile::TempDir,
    pub bin: PathBuf,
}

pub fn compile(source: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("prog.pl");
    std::fs::write(&src, source).expect("write source");
    let bin = dir.path().join("prog");
    let out = Command::new(env!("CARGO_BIN_EXE_plgc"))
        .args(["build"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run plgc");
    assert!(
        out.status.success(),
        "plgc build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Compiled { _dir: dir, bin }
}

impl Compiled {
    /// Run `--query` and return (stdout, exit code).
    pub fn query(&self, goal: &str, extra: &[&str]) -> (String, i32) {
        let out = Command::new(&self.bin)
            .args(["--query", goal])
            .args(extra)
            .output()
            .expect("run compiled binary");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Run `--query` and return raw stdout bytes + exit code. Use this for
    /// binary encodings (bson) where lossy UTF-8 decoding would corrupt bytes.
    #[allow(dead_code)] // used only by bson-capable tests; each test crate compiles the harness alone
    pub fn query_bytes(&self, goal: &str, extra: &[&str]) -> (Vec<u8>, i32) {
        let out = Command::new(&self.bin)
            .args(["--query", goal])
            .args(extra)
            .output()
            .expect("run compiled binary");
        (out.stdout, out.status.code().unwrap_or(-1))
    }
}
