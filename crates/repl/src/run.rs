//! Run a compiled session binary as a bounded subprocess and turn its
//! `--query` output into pageable solutions.
//!
//! Stateless binaries can't stream solutions across `;` presses, so we
//! fetch a batch and page client-side, re-fetching with a bigger `--limit`
//! when the user runs past it. Each fetch makes two execs: `--format json`
//! for the authoritative `count`/`exhausted` (and error detection), and
//! `--format text` for canonical term rendering — then splits the (uniform)
//! text lines into `count` solutions. Re-rendering the JSON term objects
//! ourselves would duplicate the engine's writer, so we don't.
//!
//! Paging past a batch re-fetches at a doubled limit (capped by the caller),
//! so a full enumeration of N solutions re-renders ~2–3N total — geometric,
//! i.e. linear in N, across ~log(N) spawns. Both passes assume the engine is
//! deterministic across runs (pure Prolog — no random/assert/time), so the
//! JSON `count` and the text lines describe the same solution set.
//!
//! Spawns nul stdin (keystrokes can't leak into the child) and kill on a
//! `PLG_REPL_TIMEOUT` so a divergent query can't hang the REPL.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Outcome of one query fetch.
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

/// Raw subprocess outcome — stdout captured regardless of exit code (the
/// JSON error reply lands on stdout with a non-zero exit; stderr is unused
/// but still drained so the child can't block on a full pipe).
enum Raw {
    Done { stdout: String },
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

fn query_args(goal: &str, limit: usize, format: &str) -> [String; 6] {
    [
        "--query".to_string(),
        goal.to_string(),
        "--limit".to_string(),
        limit.to_string(),
        "--format".to_string(),
        format.to_string(),
    ]
}

/// Fetch up to `limit` solutions for `goal` from `binary`.
pub fn fetch(binary: &Path, goal: &str, limit: usize) -> Fetch {
    // JSON first: authoritative count / exhausted, and the error reply.
    let json = match run_raw(binary, &query_args(goal, limit, "json")) {
        Raw::Done { stdout, .. } => stdout,
        Raw::Timeout(t) => return Fetch::Timeout(t),
        Raw::SpawnError(e) => return Fetch::Error(e),
    };
    if let Some(msg) = json_error(&json) {
        return Fetch::Failed(msg);
    }
    let count = match json_count(&json) {
        Some(0) | None => return Fetch::NoSolutions,
        Some(n) => n,
    };
    let exhausted = json_exhausted(&json);

    // TEXT for canonical rendering; split into `count` solutions.
    let text = match run_raw(binary, &query_args(goal, limit, "text")) {
        Raw::Done { stdout, .. } => stdout,
        Raw::Timeout(t) => return Fetch::Timeout(t),
        Raw::SpawnError(e) => return Fetch::Error(e),
    };
    match split_solutions(&text, count) {
        Some(solutions) => Fetch::Found {
            solutions,
            exhausted,
        },
        // The JSON count and text lines disagreed — surface it instead of
        // silently paging through a wrong grouping.
        None => Fetch::Failed("malformed query output (lines vs. count)".to_string()),
    }
}

/// Value slice of a *top-level* JSON field `"key":` — one preceded by `{` or
/// `,` — so a same-named key *nested* inside a solution term can't be matched
/// by accident if the header ever grows fields.
fn top_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let bytes = json.as_bytes();
    let mut from = 0;
    while let Some(rel) = json[from..].find(&needle) {
        let at = from + rel;
        if matches!(at.checked_sub(1).map(|i| bytes[i]), Some(b'{') | Some(b',')) {
            return Some(&json[at + needle.len()..]);
        }
        from = at + needle.len();
    }
    None
}

/// Top-level `"error":"..."` message, if the reply is an error. The engine's
/// error strings contain no embedded `"`, so the first `"}` closes it.
fn json_error(json: &str) -> Option<String> {
    let rest = top_field(json, "error")?.strip_prefix('"')?;
    let end = rest
        .find("\"}")
        .or_else(|| rest.find('"'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Top-level `"count":N`.
fn json_count(json: &str) -> Option<usize> {
    top_field(json, "count")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// Top-level `"exhausted":true`.
fn json_exhausted(json: &str) -> bool {
    top_field(json, "exhausted").is_some_and(|v| v.starts_with("true"))
}

/// Split canonical `--format text` output into `count` solution strings.
/// Every solution emits the same number of `Var = Value` lines (anonymous
/// vars aren't reported; an all-anonymous solution emits `true.`), so the
/// non-blank lines divide evenly into `count` groups. Returns `None` on a
/// non-uniform shape (shouldn't happen for valid output) so the caller can
/// report it rather than mis-group.
fn split_solutions(text: &str, count: usize) -> Option<Vec<String>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if count == 0 {
        return Some(Vec::new());
    }
    if lines.is_empty() || !lines.len().is_multiple_of(count) {
        return None;
    }
    let per = lines.len() / count;
    Some(lines.chunks(per).map(|c| c.join(", ")).collect())
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
            Ok(Some(_)) => {
                let stdout = drain(child.stdout.take());
                let _ = drain(child.stderr.take());
                return Raw::Done { stdout };
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
    use super::{json_count, json_error, json_exhausted, split_solutions};

    #[test]
    fn splits_multi_var_solutions_into_groups() {
        let text = "X = tom\nY = bob\nX = bob\nY = ann\nX = ann\nY = sue";
        assert_eq!(
            split_solutions(text, 3).unwrap(),
            ["X = tom, Y = bob", "X = bob, Y = ann", "X = ann, Y = sue"]
        );
    }

    #[test]
    fn splits_single_var_one_per_line() {
        assert_eq!(
            split_solutions("X = 1\nX = 2", 2).unwrap(),
            ["X = 1", "X = 2"]
        );
    }

    #[test]
    fn all_anonymous_solutions_are_true() {
        assert_eq!(
            split_solutions("true.\ntrue.", 2).unwrap(),
            ["true.", "true."]
        );
    }

    #[test]
    fn non_uniform_lines_is_none() {
        // 3 lines can't divide into 2 solutions — surfaced, not mis-grouped.
        assert_eq!(split_solutions("X = 1\nX = 2\nY = 3", 2), None);
    }

    #[test]
    fn parses_top_level_count_error_and_exhausted() {
        assert_eq!(
            json_count(r#"{"count":3,"exhausted":false,"solutions":[]}"#),
            Some(3)
        );
        assert!(json_exhausted(
            r#"{"count":1,"exhausted":true,"solutions":[{}]}"#
        ));
        assert!(!json_exhausted(
            r#"{"count":3,"exhausted":false,"solutions":[]}"#
        ));
        assert_eq!(json_error(r#"{"count":1}"#), None);
        assert_eq!(
            json_error(r#"{"error":"Runtime error: boom(a, b)"}"#).as_deref(),
            Some("Runtime error: boom(a, b)")
        );
        // A solution binding a var to a compound with a `count` functor must
        // NOT be mistaken for the top-level count.
        assert_eq!(
            json_count(r#"{"count":1,"solutions":[{"X":{"functor":"count","args":[9]}}]}"#),
            Some(1)
        );
    }
}
