//! I/O-free query core: parse + solve, producing the message bytes for the
//! two failure classes. The envelope *shape* and its plural *encodings* live
//! in [`crate::wire`] — this module is the solve side plus the shared
//! `exhausted` rule, with no commitment to *where* output bytes go or how
//! they're encoded. Both the CLI shell (`entry.rs`) and the Tier-2 reactor
//! (`reactor.rs`) call `run_query` here, then build a [`crate::wire::Envelope`]
//! and hand it to a chosen [`crate::wire::EncoderDesc`] — so the shape has one
//! source and can't drift between transports.

use crate::machine::Machine;
use crate::{query, solve};

/// Outcome of running one query, with the prefixed message the wire contract
/// puts on the wire for the two failure classes. The caller maps these to its
/// own surface — exit codes 2/3 for the CLI, an error document for the reactor
/// — but the message bytes are produced once, here.
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

/// The `exhausted` flag: the search ran to completion unless a `--limit`
/// stopped it exactly at the cap. Single-sourced so the CLI and the reactor
/// compute it identically.
pub fn exhausted(m: &Machine) -> bool {
    m.solution_limit.is_none_or(|l| m.solutions.len() < l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RenderedSolution;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    #[test]
    fn exhausted_follows_the_limit() {
        let mut m = machine();
        assert!(exhausted(&m), "no limit => exhausted");
        m.solution_limit = Some(2);
        assert!(exhausted(&m), "under the limit => exhausted");
        m.solutions.push(RenderedSolution { bindings: vec![] });
        m.solutions.push(RenderedSolution { bindings: vec![] });
        assert!(!exhausted(&m), "limit hit exactly => not exhausted");
    }
}
