//! Run a compiled session binary as a bounded subprocess.
//!
//! Ported in spirit from patch-seq's `seqr`: spawn with stdin nulled (so
//! keystrokes can't leak into the child's stdin and freeze the REPL) and
//! kill on a timeout, so a divergent query can't hang the loop — a
//! belt-and-suspenders alongside the runtime step limit.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_SECS: u64 = 10;

pub enum RunResult {
    /// Exit 0/1 — the program ran; `stdout` holds the wire-format output.
    Ok(String),
    /// Non-zero exit (query parse error 2, runtime error 3, …).
    Failed(String),
    /// Killed after exceeding the timeout.
    Timeout(u64),
    /// Could not spawn the binary.
    Error(String),
}

fn timeout() -> Duration {
    let secs = std::env::var("PLG_REPL_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Run `?- goal` against `binary` via the wire contract (`--query`).
pub fn query(binary: &Path, goal: &str, limit: usize) -> RunResult {
    let args = [
        "--query".to_string(),
        goal.to_string(),
        "--limit".to_string(),
        limit.to_string(),
        "--format".to_string(),
        "text".to_string(),
    ];
    run(binary, &args)
}

fn run(path: &Path, args: &[String]) -> RunResult {
    let mut child = match Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return RunResult::Error(format!("failed to start: {e}")),
    };

    let limit = timeout();
    let start = Instant::now();
    let poll = Duration::from_millis(50);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    RunResult::Ok(drain(child.stdout.take()))
                } else {
                    // exit 1 ("solutions found") is success per the wire
                    // contract — solutions still land on stdout.
                    let stdout = drain(child.stdout.take());
                    if status.code() == Some(1) && !stdout.trim().is_empty() {
                        RunResult::Ok(stdout)
                    } else {
                        RunResult::Failed(drain(child.stderr.take()))
                    }
                };
            }
            Ok(None) => {
                if start.elapsed() >= limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return RunResult::Timeout(limit.as_secs());
                }
                std::thread::sleep(poll);
            }
            Err(e) => return RunResult::Error(format!("wait error: {e}")),
        }
    }
}

fn drain<R: Read>(pipe: Option<R>) -> String {
    pipe.map(|mut r| {
        let mut buf = String::new();
        let _ = r.read_to_string(&mut buf);
        buf
    })
    .unwrap_or_default()
}
