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
    /// Run `--query` and return (stdout, exit code). Default output is the
    /// readable `text` format.
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

    /// Run `--query --format bson` and decode the envelope's scalar fields
    /// (`count`, `exhausted`, `error`). Solution *values* are opaque TermBuf
    /// cell words (atom ids) — a test has no atom table to resolve them, so
    /// value/order assertions stay in `text`; this is for `count`/`exhausted`
    /// checks that the readable text format doesn't carry.
    #[allow(dead_code)] // each test crate compiles the harness alone
    pub fn query_bson(&self, goal: &str, extra: &[&str]) -> (BsonEnvelope, i32) {
        let mut all = vec!["--format", "bson"];
        all.extend_from_slice(extra);
        let out = Command::new(&self.bin)
            .args(["--query", goal])
            .args(&all)
            .output()
            .expect("run compiled binary");
        (
            bson_decode(&out.stdout).expect("valid bson envelope or error doc"),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Run with arbitrary argv (no forced `--query`) and stdin bytes; return
    /// raw stdout bytes + exit code. Used for bson-input mode.
    #[allow(dead_code)]
    pub fn run_with_stdin(&self, args: &[&str], stdin: &[u8]) -> (Vec<u8>, i32) {
        use std::io::Write;
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn compiled binary");
        if !stdin.is_empty() {
            child.stdin.take().unwrap().write_all(stdin).unwrap();
        }
        let out = child.wait_with_output().expect("wait compiled binary");
        (out.stdout, out.status.code().unwrap_or(-1))
    }

    /// Like `run_with_stdin` but decode the bson envelope from stdout.
    #[allow(dead_code)]
    pub fn run_with_stdin_bson(&self, args: &[&str], stdin: &[u8]) -> (BsonEnvelope, i32) {
        let (bytes, code) = self.run_with_stdin(args, stdin);
        (
            bson_decode(&bytes).expect("valid bson envelope or error doc"),
            code,
        )
    }
}

/// The scalar fields of a bson envelope / error document, decoded for tests.
/// `count`/`exhausted` come from a success envelope; `error` from an error doc.
#[allow(dead_code)]
#[derive(Debug)]
pub struct BsonEnvelope {
    pub count: Option<i64>,
    pub exhausted: Option<bool>,
    pub error: Option<String>,
    pub atoms: Option<Vec<String>>,
}

/// Minimal bson document decoder extracting the top-level scalar fields tests
/// need (`count` int32, `exhausted` bool, `error` string). Not a general bson
/// parser — it sizes-and-skips compound/binary values it doesn't read.
#[allow(dead_code)]
pub fn bson_decode(buf: &[u8]) -> Option<BsonEnvelope> {
    if buf.len() < 5 {
        return None;
    }
    let total = i32::from_le_bytes(buf[0..4].try_into().ok()?) as usize;
    if total > buf.len() {
        return None;
    }
    let body = &buf[..total];
    let mut off = 4;
    let end = total - 1;
    let mut count = None;
    let mut exhausted = None;
    let mut error = None;
    let mut atoms = None;
    while off < end {
        let ty = body[off];
        off += 1;
        let (key, after) = read_cstring(body, off)?;
        off = after;
        match (ty, key.as_str()) {
            (0x10, "count") => {
                count = Some(i32::from_le_bytes(body[off..off + 4].try_into().ok()?) as i64);
                off += 4;
            }
            (0x08, "exhausted") => {
                exhausted = Some(body[off] != 0);
                off += 1;
            }
            (0x02, "error") => {
                let n = i32::from_le_bytes(body[off..off + 4].try_into().ok()?) as usize;
                error = Some(String::from_utf8_lossy(&body[off + 4..off + 4 + n - 1]).into_owned());
                off += 4 + n;
            }
            (0x04, "atoms") => {
                // self-describing mode (--atoms): an array of name strings.
                let arr_total = i32::from_le_bytes(body[off..off + 4].try_into().ok()?) as usize;
                let arr_end = off + arr_total;
                let mut names = Vec::new();
                let mut ao = off + 4;
                while ao < arr_end - 1 {
                    let aty = body[ao];
                    let (_, after_k) = read_cstring(body, ao + 1)?;
                    ao = after_k;
                    if aty == 0x02 {
                        let n = i32::from_le_bytes(body[ao..ao + 4].try_into().ok()?) as usize;
                        names.push(
                            String::from_utf8_lossy(&body[ao + 4..ao + 4 + n - 1]).into_owned(),
                        );
                        ao += 4 + n;
                    } else {
                        break;
                    }
                }
                atoms = Some(names);
                off = arr_end;
            }
            _ => {
                off = skip_value(body, off, ty)?;
            }
        }
    }
    Some(BsonEnvelope {
        count,
        exhausted,
        error,
        atoms,
    })
}

#[allow(dead_code)]
fn read_cstring(buf: &[u8], mut off: usize) -> Option<(String, usize)> {
    let end = buf[off..].iter().position(|&b| b == 0)?;
    let s = String::from_utf8_lossy(&buf[off..off + end]).into_owned();
    off += end + 1;
    Some((s, off))
}

#[allow(dead_code)]
fn skip_value(buf: &[u8], off: usize, ty: u8) -> Option<usize> {
    Some(match ty {
        0x01 => off + 8, // double
        0x02 => {
            // string
            let n = i32::from_le_bytes(buf[off..off + 4].try_into().ok()?) as usize;
            off + 4 + n
        }
        0x03 | 0x04 => {
            // document / array
            let n = i32::from_le_bytes(buf[off..off + 4].try_into().ok()?) as usize;
            off + n
        }
        0x05 => {
            // binary
            let n = i32::from_le_bytes(buf[off..off + 4].try_into().ok()?) as usize;
            off + 4 + 1 + n
        }
        0x08 => off + 1, // bool
        0x0A => off,     // null
        0x10 => off + 4, // int32
        0x12 => off + 8, // int64
        _ => return None,
    })
}
