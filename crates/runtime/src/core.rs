//! I/O-free query core: parse + solve + the v1 JSON wire shape, with no
//! commitment to *where* the bytes go. Both the WASI/CLI shell (`entry.rs`,
//! sink = stdout) and the Tier-2 reactor (`reactor.rs`, sink = a linear-memory
//! buffer) call into here, so the JSON shape and the `exhausted` rule have a
//! single source and can't drift between the two transports
//! (docs/design/done/WASM_TIER2_PLAN.md A1 / WASM.md finding #6). The shared core
//! INVOCATION.md's resident mode also wants is the same one.

use crate::machine::Machine;
use crate::{query, render, solve};
use std::io::{self, Write};

/// Outcome of running one query, with the prefixed message the v1 contract
/// puts on the wire for the two failure classes. The caller maps these to its
/// own surface — exit codes 2/3 for the CLI, an `{"error":...}` object for the
/// reactor — but the message bytes are produced once, here.
pub enum QueryResult {
    /// Solved; solutions live in `m.solutions`.
    Solutions,
    /// Query failed to parse — `"Parse error: …"` (CLI exit 2).
    ParseError(String),
    /// A runtime error was raised — `"Runtime error: …"` (CLI exit 3).
    RuntimeError(String),
}

/// Parse `q` against the program in `m`, then solve it. The caller must have
/// already reset per-query state and set the per-query limits; this consumes
/// `m.error` on the error path so the message can be returned.
pub fn run_query(m: &mut Machine, q: &str) -> QueryResult {
    let goal = match query::parse_query(m, q) {
        Ok(g) => g,
        Err(e) => return QueryResult::ParseError(format!("Parse error: {e}")),
    };
    match solve::solve(m, goal) {
        solve::Outcome::Error => {
            let msg = m.error.take().map(|e| e.message).unwrap_or_default();
            QueryResult::RuntimeError(format!("Runtime error: {msg}"))
        }
        solve::Outcome::Done => QueryResult::Solutions,
    }
}

/// The v1 `exhausted` flag: the search ran to completion unless a `--limit`
/// stopped it exactly at the cap. Single-sourced so the CLI and the reactor
/// compute it identically (finding #4 — the spike hard-coded `true`).
pub fn exhausted(m: &Machine) -> bool {
    m.solution_limit.is_none_or(|l| m.solutions.len() < l)
}

/// v1 error object: `{"error":"<escaped message>"}`. No trailing newline —
/// the CLI appends one for stdout, the reactor returns the bytes as-is.
pub fn write_error_json<W: Write>(w: &mut W, message: &str) -> io::Result<()> {
    write!(w, "{{\"error\":\"{}\"}}", render::json_escape(message))
}

/// v1 success object: `{"count":N,"exhausted":B,"solutions":[…]}`, keys in
/// serde_json sorted order. `output`, when `Some`, inserts an `"output"` field
/// (sorts between `exhausted` and `solutions`) carrying captured `write/1`
/// bytes — the reactor uses it (no stdout in an isolate, D4); the CLI passes
/// `None` because its output already streamed to stdout, keeping native bytes
/// byte-identical to v1.
pub fn write_solutions_json<W: Write>(
    w: &mut W,
    m: &Machine,
    exhausted: bool,
    output: Option<&str>,
) -> io::Result<()> {
    write!(
        w,
        "{{\"count\":{},\"exhausted\":{}",
        m.solutions.len(),
        exhausted
    )?;
    if let Some(out) = output {
        write!(w, ",\"output\":\"{}\"", render::json_escape(out))?;
    }
    w.write_all(b",\"solutions\":[")?;
    for (i, sol) in m.solutions.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        w.write_all(b"{")?;
        for (j, (name, json, _)) in sol.bindings.iter().enumerate() {
            if j > 0 {
                w.write_all(b",")?;
            }
            write!(w, "\"{}\":{}", render::json_escape(name), json)?;
        }
        w.write_all(b"}")?;
    }
    w.write_all(b"]}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    fn bytes(f: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut buf = Vec::new();
        f(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn empty_success_matches_v1_shape() {
        let m = machine();
        assert_eq!(
            bytes(|w| write_solutions_json(w, &m, true, None)),
            "{\"count\":0,\"exhausted\":true,\"solutions\":[]}"
        );
    }

    #[test]
    fn output_field_sorts_between_exhausted_and_solutions() {
        let m = machine();
        assert_eq!(
            bytes(|w| write_solutions_json(w, &m, false, Some("hi\n"))),
            "{\"count\":0,\"exhausted\":false,\"output\":\"hi\\n\",\"solutions\":[]}"
        );
    }

    #[test]
    fn error_object_is_escaped() {
        assert_eq!(
            bytes(|w| write_error_json(w, "a\"b")),
            "{\"error\":\"a\\\"b\"}"
        );
    }

    #[test]
    fn exhausted_follows_the_limit() {
        let mut m = machine();
        assert!(exhausted(&m), "no limit => exhausted");
        m.solution_limit = Some(2);
        assert!(exhausted(&m), "under the limit => exhausted");
        m.solutions
            .push(render::RenderedSolution { bindings: vec![] });
        m.solutions
            .push(render::RenderedSolution { bindings: vec![] });
        assert!(!exhausted(&m), "limit hit exactly => not exhausted");
    }
}
