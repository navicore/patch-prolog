//! The wire layer: the fixed envelope *shape* as a typed value, plus a
//! plural `Encoder` trait whose impls are the *encodings* (text/JSON today;
//! bson to follow). Splits what was collapsed in `core.rs` — where the JSON
//! byte-emitters implicitly defined the shape — so the shape is owned once
//! (here) and the encodings vary independently. See docs/design/IO.md.
//!
//! Both the CLI/WASI shell (`entry.rs`) and the Tier-2 reactor (`reactor.rs`)
//! build an `Envelope` and hand it to a chosen `Encoder`, so the envelope
//! shape has a single source and can't drift between transports.

use crate::machine::Machine;
use crate::render::{RenderedSolution, json_escape};
use std::io::{self, Write};

/// The fixed engine-output shape — the contract, made a type. Every
/// `Encoder` reads the same fields; only their byte encoding varies.
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],
    /// Captured `write/1` bytes, when the sink is in capture mode. `None` when
    /// output streamed to stdout (the CLI text path, v1 byte-identical).
    /// Encoders that can't stream (`can_stream() == false`) require the caller
    /// to have run in capture mode so this is `Some`.
    pub program_output: Option<&'a str>,
}

impl<'a> Envelope<'a> {
    /// Build from the machine's solved state. `program_output` follows the
    /// machine's output sink: `None` when streaming to stdout, `Some(..)` when
    /// capturing — so one constructor serves both the CLI and the reactor.
    pub fn from_machine(m: &'a Machine, exhausted: bool) -> Self {
        Self {
            count: m.solutions.len(),
            exhausted,
            solutions: &m.solutions,
            program_output: m.captured_output(),
        }
    }
}

/// The two failure classes the engine distinguishes, carrying the message the
/// wire contract puts on the wire. Callers map these to their surface (exit
/// codes 2/3 for the CLI); the message bytes come from `core::run_query`.
pub enum WireError {
    Parse(String),
    Runtime(String),
}

/// The plural encoding. One impl per wire format.
pub trait Encoder {
    /// Serialise a solved envelope.
    fn write_envelope(&self, w: &mut dyn Write, e: &Envelope) -> io::Result<()>;
    /// Serialise an error.
    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()>;
    /// Whether this encoding can coexist with streamed `write/1` bytes on the
    /// same stdout. text/JSON: yes (v1-preserved). bson: no — a binary format
    /// can't tolerate interleaved foreign bytes, so bson forces capture mode.
    fn can_stream(&self) -> bool;
}

/// Look up an encoder by the `--format` name. Returns `None` for unknown names
/// (the CLI reports a usage error → exit 2). Today only `"json"` (the text wire
/// encoding) is registered; bson and the capability table land in later PRs.
pub fn encoder_for(name: &str) -> Option<Box<dyn Encoder>> {
    match name {
        "json" => Some(Box::new(Json)),
        _ => None,
    }
}

/// The text wire encoding: the v1 JSON envelope, byte-identical to the
/// pre-refactor output (`core::write_solutions_json` / `write_error_json`).
/// Registered as `"json"` to preserve the current CLI contract; the rename to
/// `text` (and the demotion of the human `X = foo` form to `--pretty`) comes
/// with the capability-table work.
pub struct Json;

impl Encoder for Json {
    fn write_envelope(&self, w: &mut dyn Write, e: &Envelope) -> io::Result<()> {
        write!(w, "{{\"count\":{},\"exhausted\":{}", e.count, e.exhausted)?;
        if let Some(out) = e.program_output {
            write!(w, ",\"output\":\"{}\"", json_escape(out))?;
        }
        w.write_all(b",\"solutions\":[")?;
        for (i, sol) in e.solutions.iter().enumerate() {
            if i > 0 {
                w.write_all(b",")?;
            }
            w.write_all(b"{")?;
            for (j, (name, json, _)) in sol.bindings.iter().enumerate() {
                if j > 0 {
                    w.write_all(b",")?;
                }
                write!(w, "\"{}\":{}", json_escape(name), json)?;
            }
            w.write_all(b"}")?;
        }
        w.write_all(b"]}")
    }

    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()> {
        let msg = match e {
            WireError::Parse(m) | WireError::Runtime(m) => m,
        };
        write!(w, "{{\"error\":\"{}\"}}", json_escape(msg))
    }

    fn can_stream(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(count: usize, exhausted: bool, output: Option<&'a str>) -> Envelope<'a> {
        Envelope {
            count,
            exhausted,
            solutions: &[],
            program_output: output,
        }
    }

    fn bytes(f: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut buf = Vec::new();
        f(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn json_empty_success_matches_v1_shape() {
        let e = env(0, true, None);
        assert_eq!(
            bytes(|w| Json.write_envelope(w, &e)),
            "{\"count\":0,\"exhausted\":true,\"solutions\":[]}"
        );
    }

    #[test]
    fn json_output_field_sorts_between_exhausted_and_solutions() {
        let e = env(0, false, Some("hi\n"));
        assert_eq!(
            bytes(|w| Json.write_envelope(w, &e)),
            "{\"count\":0,\"exhausted\":false,\"output\":\"hi\\n\",\"solutions\":[]}"
        );
    }

    #[test]
    fn json_error_object_is_escaped() {
        assert_eq!(
            bytes(|w| Json.write_error(w, &WireError::Parse("a\"b".into()))),
            "{\"error\":\"a\\\"b\"}"
        );
    }

    #[test]
    fn encoder_for_known_and_unknown() {
        assert!(encoder_for("json").is_some());
        assert!(encoder_for("bson").is_none(), "bson lands in a later PR");
        assert!(encoder_for("garbage").is_none());
    }

    #[test]
    fn json_can_stream() {
        assert!(Json.can_stream());
    }
}
