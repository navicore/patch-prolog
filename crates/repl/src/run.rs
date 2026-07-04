//! Run a compiled session binary as a bounded subprocess and turn its
//! `--format text` output into pageable solutions.
//!
//! Stateless binaries can't stream solutions across `;` presses, so we
//! fetch a batch and page client-side, re-fetching with a bigger `--limit`
//! when the user runs past it. Each fetch is a **single** exec (`--format
//! text`); the engine's text writer is the single source of truth for how
//! terms render, so the REPL never duplicates the renderer.
//!
//! The engine's exit code is the authoritative outcome signal
//! (`entry.rs`):
//!   0 = no solutions, 1 = solutions found,
//!   2 = query parse / usage error, 3 = runtime error
//! and the text body carries the bindings (one `Var = Value` line per
//! binding; `true.` for an all-anonymous solution; `false.` for none).
//! `exhausted` is inferred from the batch size: a query that yielded fewer
//! solutions than the `--limit` is fully explored; one that filled the
//! limit may have more.
//!
//! Paging past a batch re-fetches at a doubled limit (capped by the caller),
//! so a full enumeration of N solutions re-spawns ~log(N) times. Spawns nul
//! stdin (keystrokes can't leak into the child) and kills on
//! `PLG_REPL_TIMEOUT` so a divergent query can't hang the REPL.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Outcome of one query fetch.
#[derive(Debug)]
pub enum Fetch {
    /// Rendered solutions (each a display string) and whether the engine
    /// reported the search exhausted at this limit (`false` ⇒ there may be
    /// more beyond the batch).
    Found {
        solutions: Vec<String>,
        exhausted: bool,
    },
    /// The goal has no solutions.
    NoSolutions,
    /// A runtime/parse error; message for display.
    Failed(String),
    /// Killed after exceeding the timeout.
    Timeout(u64),
    /// Could not spawn the binary.
    Error(String),
}

/// Raw subprocess outcome — stdout/stderr captured regardless of exit code
/// (the text error reply lands on stdout; stderr is kept as a fallback for
/// errors the engine couldn't serialize, e.g. an undeclared `--format`).
enum Raw {
    Done {
        stdout: String,
        stderr: String,
        code: i32,
    },
    Timeout(u64),
    SpawnError(String),
}

fn timeout() -> Duration {
    let secs = std::env::var("PLG_REPL_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

fn query_args(goal: &str, limit: usize) -> [String; 6] {
    [
        "--query".to_string(),
        goal.to_string(),
        "--limit".to_string(),
        limit.to_string(),
        "--format".to_string(),
        "text".to_string(),
    ]
}

/// Fetch up to `limit` solutions for `goal` from `binary`.
pub fn fetch(binary: &Path, goal: &str, limit: usize) -> Fetch {
    let raw = match run_raw(binary, &query_args(goal, limit)) {
        Raw::Done {
            stdout,
            stderr,
            code,
        } => (stdout, stderr, code),
        Raw::Timeout(t) => return Fetch::Timeout(t),
        Raw::SpawnError(e) => return Fetch::Error(e),
    };

    // Dispatch on the engine's authoritative exit code (entry.rs contract).
    match raw.2 {
        // 0 = no solutions.
        0 => Fetch::NoSolutions,
        // 1 = solutions found; split the text body into per-solution groups.
        1 => match split_text_solutions(&raw.0) {
            Some(solutions) if solutions.is_empty() => Fetch::NoSolutions,
            Some(solutions) => Fetch::Found {
                exhausted: solutions.len() < limit,
                solutions,
            },
            None => Fetch::Failed("malformed query output".to_string()),
        },
        // 2 = parse/usage error, 3 = runtime error — body is `error: <msg>`.
        2 | 3 => Fetch::Failed(error_message(&raw.0, &raw.1)),
        other => Fetch::Failed(format!("unexpected exit code {other}")),
    }
}

/// The engine's error message: text errors land on stdout as `error: <msg>`;
/// fall back to stderr for errors that had no encoder to serialize through.
fn error_message(stdout: &str, stderr: &str) -> String {
    if let Some(rest) = stdout.trim().strip_prefix("error: ") {
        return rest.to_string();
    }
    if !stderr.trim().is_empty() {
        return stderr.trim().to_string();
    }
    stdout.trim().to_string()
}

/// Split canonical `--format text` output into one display string per
/// solution. The text writer guarantees a uniform shape per query. With no
/// solutions it emits `false.` (the caller dispatches on exit code, but we
/// defend against it here too); an all-anonymous solution emits one `true.`
/// line; otherwise every solution is `Var = Value` lines for the *same*
/// variables in the *same* order.
///
/// That uniformity lets us recover group boundaries without an external
/// count: the first solution's leading variable name marks each new group.
/// Returns `None` on a non-uniform shape (shouldn't happen for valid output)
/// so the caller reports it rather than mis-group.
fn split_text_solutions(text: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() || lines == ["false."] {
        return Some(Vec::new());
    }
    if lines.iter().all(|l| *l == "true.") {
        return Some(lines.iter().map(|l| l.to_string()).collect());
    }
    // The first solution's first variable name is the group delimiter: it
    // recurs at the start of every subsequent solution.
    let first_lhs = lhs_of(lines.first()?)?;
    let per = (1..lines.len())
        .find(|&i| lhs_of(lines[i]) == Some(first_lhs))
        .unwrap_or(lines.len());
    if !lines.len().is_multiple_of(per) {
        return None;
    }
    Some(lines.chunks(per).map(|c| c.join(", ")).collect())
}

/// The left-hand side of a `Var = Value` line, or `None` if the line isn't
/// in that shape. Splits on the first ` = ` so a value containing ` = `
/// (e.g. `X = a = b`) is handled correctly.
fn lhs_of(line: &str) -> Option<&str> {
    line.split_once(" = ").map(|(lhs, _)| lhs)
}

fn run_raw(path: &Path, args: &[String]) -> Raw {
    let mut child = match Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Raw::SpawnError(format!("failed to start: {e}")),
    };

    let limit = timeout();
    let start = Instant::now();
    let poll = Duration::from_millis(50);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = drain(child.stdout.take());
                let stderr = drain(child.stderr.take());
                return Raw::Done {
                    stdout,
                    stderr,
                    code: status.code().unwrap_or(-1),
                };
            }
            Ok(None) => {
                if start.elapsed() >= limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Raw::Timeout(limit.as_secs());
                }
                std::thread::sleep(poll);
            }
            Err(e) => return Raw::SpawnError(format!("wait error: {e}")),
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

#[cfg(test)]
mod tests {
    use super::{error_message, lhs_of, split_text_solutions};
    use crate::engine;
    use crate::run::{Fetch, fetch};
    use std::path::{Path, PathBuf};

    #[test]
    fn splits_multi_var_solutions_into_groups() {
        let text = "X = tom\nY = bob\nX = bob\nY = ann\nX = ann\nY = sue";
        assert_eq!(
            split_text_solutions(text).unwrap(),
            ["X = tom, Y = bob", "X = bob, Y = ann", "X = ann, Y = sue"]
        );
    }

    #[test]
    fn splits_single_var_one_per_line() {
        assert_eq!(
            split_text_solutions("X = 1\nX = 2").unwrap(),
            ["X = 1", "X = 2"]
        );
    }

    #[test]
    fn all_anonymous_solutions_are_true() {
        assert_eq!(
            split_text_solutions("true.\ntrue.").unwrap(),
            ["true.", "true."]
        );
    }

    #[test]
    fn no_solutions_is_empty_not_none() {
        assert_eq!(
            split_text_solutions("false.").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(split_text_solutions("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn value_containing_equals_is_kept_intact() {
        // `a = b` is a legal value; the lhs is still just `X`.
        assert_eq!(
            split_text_solutions("X = a = b\nX = c").unwrap(),
            ["X = a = b", "X = c"]
        );
    }

    #[test]
    fn partial_trailing_group_is_none() {
        // Two-var solutions (per = 2, delimited by the recurring `X`) but a
        // stray trailing line ⇒ 5 isn't divisible by 2 ⇒ surfaced, not
        // mis-grouped.
        assert_eq!(
            split_text_solutions("X = 1\nY = 2\nX = 3\nY = 4\nX = 5"),
            None
        );
    }

    #[test]
    fn lhs_of_splits_on_first_equals() {
        assert_eq!(lhs_of("X = a"), Some("X"));
        assert_eq!(lhs_of("X = a = b"), Some("X"));
        assert_eq!(lhs_of("true."), None);
    }

    #[test]
    fn error_message_prefers_stdout_then_stderr() {
        assert_eq!(
            error_message("error: Parse error: boom\n", ""),
            "Parse error: boom"
        );
        assert_eq!(
            error_message("", "Unknown or undeclared format: bson"),
            "Unknown or undeclared format: bson"
        );
        assert_eq!(error_message("  spaced  ", ""), "spaced");
    }

    /// Locate `plgc`: `$PLGC` wins, else the workspace `target/<profile>/plgc`
    /// that `cargo test --workspace` is guaranteed to have built.
    fn locate_plgc() -> PathBuf {
        if let Ok(p) = std::env::var("PLGC") {
            return PathBuf::from(p);
        }
        for profile in ["debug", "release"] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .join(profile)
                .join("plgc");
            if p.exists() {
                return p;
            }
        }
        panic!("plgc not found: run `cargo build -p patch-prolog-compiler` or set $PLGC");
    }

    /// End-to-end: compile a real program via `plgc` and exercise `fetch()`
    /// through the live `--format text` wire path. This is the test that
    /// would have caught the json/bson regression — every other test here
    /// operates on strings, so a broken wire contract could slip past them.
    #[test]
    fn fetch_runs_a_real_compiled_binary() {
        // SAFETY: this is the only test in the crate that touches $PLGC,
        // so there's no concurrent reader/writer to race.
        unsafe {
            std::env::set_var("PLGC", locate_plgc());
        }

        // The bug report: a cut-based `membercheck` whose bound-variable
        // query once printed `false` because fetch() asked for `--format json`.
        let src = "membercheck(X, [X|_]) :- !.\n\
                   membercheck(X, [_|L]) :- membercheck(X, L).\n";
        let compiled = engine::compile(src).expect("compile session");

        match fetch(&compiled.binary, "X = a, membercheck(X, [a, b, c])", 16) {
            Fetch::Found {
                solutions,
                exhausted,
            } => {
                assert_eq!(solutions, ["X = a"]);
                assert!(exhausted, "1 < 16 ⇒ fully explored");
            }
            other => panic!("bound-var query: expected Found, got {other:?}"),
        }

        // No solutions ⇒ exit 0 ⇒ NoSolutions (not Failed, not empty Found).
        assert!(matches!(
            fetch(&compiled.binary, "membercheck(z, [a, b, c])", 16),
            Fetch::NoSolutions,
        ));

        // Multi-solution enumeration + the exhaustion flag, including at the
        // limit boundary (the signal paging relies on).
        let multi = engine::compile("f(1). f(2). f(3).").unwrap();
        match fetch(&multi.binary, "f(X)", 16) {
            Fetch::Found {
                solutions,
                exhausted,
            } => {
                assert_eq!(solutions, ["X = 1", "X = 2", "X = 3"]);
                assert!(exhausted, "3 < 16 ⇒ fully explored");
            }
            other => panic!("multi: expected Found, got {other:?}"),
        }
        match fetch(&multi.binary, "f(X)", 2) {
            Fetch::Found {
                solutions,
                exhausted,
            } => {
                assert_eq!(solutions, ["X = 1", "X = 2"]);
                assert!(!exhausted, "filled the limit ⇒ maybe more");
            }
            other => panic!("truncated: expected Found, got {other:?}"),
        }

        // Parse error ⇒ exit 2 ⇒ Failed carrying the engine's message.
        match fetch(&multi.binary, "f(X,,", 16) {
            Fetch::Failed(msg) => assert!(msg.contains("Parse"), "got: {msg}"),
            other => panic!("parse error: expected Failed, got {other:?}"),
        }
    }
}
