//! Process entry: `plg_rt_init` + `plg_rt_main`, called from the thin
//! generated `main`. Owns argv parsing (hand-rolled — no clap inside
//! compiled binaries), output, and the v1 exit-code contract:
//!
//!   0 = no solutions, 1 = solutions found,
//!   2 = query parse error, 3 = runtime error

use crate::core::{self, QueryResult};
use crate::machine::{Machine, RegistryEntry, SrcLoc};
use crate::wire::{EncoderDesc, Envelope, WireError};
use plg_shared::StringInterner;
use std::ffi::CStr;
use std::io::{self, Read, Write};
use std::os::raw::c_char;

/// Build the Machine from the tables codegen baked into the binary.
/// Re-interning the emitted atom names in id order reproduces the
/// compiler's exact id space (the interner pre-seeds the same
/// well-known atoms the compiler's did).
///
/// # Safety
/// Called once from generated `main` with codegen-emitted tables.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_init(
    atom_strs: *const *const c_char,
    atom_count: u32,
    registry: *const RegistryEntry,
    registry_len: u32,
    srcmap: *const SrcLoc,
    srcmap_len: u32,
    files: *const *const c_char,
    files_len: u32,
    caps: *const *const EncoderDesc,
    caps_len: u32,
) -> *mut Machine {
    let mut atoms = StringInterner::new();
    for i in 0..atom_count as usize {
        let s = unsafe { CStr::from_ptr(*atom_strs.add(i)) };
        let expected = i as u32;
        let id = atoms.intern(&s.to_string_lossy());
        debug_assert_eq!(id, expected, "atom table out of sync with interner");
    }
    let registry: Vec<RegistryEntry> =
        unsafe { std::slice::from_raw_parts(registry, registry_len as usize) }.to_vec();
    debug_assert!(
        registry.is_sorted_by_key(|e| (e.functor, e.arity)),
        "registry must be sorted for binary search"
    );
    // Source-location side-table (SPANS.md Layer 3). Both tables are empty
    // (`len == 0`) for binaries built without provenance.
    let srcmap: Vec<SrcLoc> = if srcmap_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(srcmap, srcmap_len as usize) }.to_vec()
    };
    let files: Vec<String> = (0..files_len as usize)
        .map(|i| {
            unsafe { CStr::from_ptr(*files.add(i)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let mut m = Machine::new(atoms, registry);
    m.set_provenance(srcmap, files);
    m.capabilities = (0..caps_len as usize)
        .map(|i| unsafe { *caps.add(i) })
        .collect();
    Box::into_raw(m)
}

struct Args {
    query: Option<String>,
    limit: Option<usize>,
    format: String,
    input_format: String,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut query = None;
    let mut limit = None;
    let mut format = "json".to_string(); // v1 default
    let mut input_format = "text".to_string(); // default: argv `--query`
    let mut it = argv.into_iter().peekable();
    while let Some(arg) = it.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg, None),
        };
        let value = |it: &mut std::iter::Peekable<std::vec::IntoIter<String>>| {
            inline_value
                .clone()
                .or_else(|| it.next())
                .ok_or(format!("missing value for {flag}"))
        };
        match flag.as_str() {
            "-q" | "--query" => query = Some(value(&mut it)?),
            "-l" | "--limit" => {
                limit = Some(
                    value(&mut it)?
                        .parse::<usize>()
                        .map_err(|_| "invalid --limit value".to_string())?,
                )
            }
            "-f" | "--format" => format = value(&mut it)?,
            "--input-format" => input_format = value(&mut it)?,
            "-h" | "--help" => {
                return Err(
                    "usage: --query <goal> [--limit N] [--format json|text|bson] [--input-format text|bson]"
                        .to_string(),
                );
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Args {
        query,
        limit,
        format,
        input_format,
    })
}

/// Emit solutions for a solved query. `enc == None` is the human display form
/// (`--format text`); otherwise the wire encoder serializes the envelope.
/// The `can_stream()` flag drives both the line discipline (streamed json gets
/// a trailing newline; binary bson gets an explicit flush instead) and the
/// capture-mode decision made earlier in `plg_rt_main`.
fn output_solutions(enc: Option<&EncoderDesc>, m: &Machine, exhausted: bool) {
    let Some(e) = enc else {
        output_text(m);
        return;
    };
    let mut out = io::stdout().lock();
    let env = Envelope::from_machine(m, exhausted);
    let _ = (e.write_envelope)(&mut out, m, &env);
    if (e.can_stream)() {
        let _ = out.write_all(b"\n");
    } else {
        let _ = out.flush();
    }
}

/// Emit an error. Wire encodings go to stdout (the encoding's error object);
/// the human form (`enc == None`) goes to stderr (v1 behavior). The caller
/// constructs the `WireError` with the correct class (`Parse` vs `Runtime`) so
/// the distinction is honest on the wire even though today's encoders collapse
/// both to the same message — a future bson `kind` field will surface it.
fn output_result(enc: Option<&EncoderDesc>, err: WireError) {
    let message = match &err {
        WireError::Parse(m) | WireError::Runtime(m) => m.as_str(),
    };
    match enc {
        Some(e) => {
            let mut out = io::stdout().lock();
            let _ = (e.write_error)(&mut out, &err);
            if (e.can_stream)() {
                let _ = out.write_all(b"\n");
            } else {
                let _ = out.flush();
            }
        }
        None => eprintln!("Error: {message}"),
    }
}

fn output_text(m: &Machine) {
    if m.solutions.is_empty() {
        println!("false.");
        return;
    }
    for sol in &m.solutions {
        if sol.bindings.is_empty() {
            println!("true.");
        } else {
            for b in &sol.bindings {
                println!("{} = {}", b.name, b.text);
            }
        }
    }
}

/// Run the query named in argv against the compiled program. Returns
/// the process exit code.
///
/// # Safety
/// Called once from generated `main` with the process argc/argv.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_main(
    m: *mut Machine,
    argc: i32,
    argv: *const *const c_char,
) -> i32 {
    let m = unsafe { &mut *m };
    let raw_args: Vec<String> = (1..argc as usize)
        .map(|i| {
            unsafe { CStr::from_ptr(*argv.add(i)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    let args = match parse_args(raw_args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // Resolve the output encoding against the capability table (the encoders
    // this binary advertises via `io_format/1`, default `[json]`). `text` is
    // the human display form — always available, not a wire encoding. An
    // unknown or undeclared wire format is a usage error (exit 2).
    let enc: Option<&'static EncoderDesc> = if args.format == "text" {
        None
    } else {
        match unsafe {
            EncoderDesc::find(m.capabilities.as_ptr(), m.capabilities.len(), &args.format)
        } {
            Some(e) => Some(e),
            None => {
                eprintln!("Unknown or undeclared format: {}", args.format);
                return 2;
            }
        }
    };
    // Resolve the query string and limit by input mode (IO.md). Default
    // (`text`) takes the query from argv `--query` (required) — always
    // available, like the human display form. `--input-format bson` reads a
    // one-field request document `{query, limit?}` from stdin; it requires the
    // binary to advertise `bson` (capability gates both directions). argv
    // `--limit`, when present, overrides any limit from the bson document.
    let (query, argv_limit) = match args.input_format.as_str() {
        "text" => {
            let q = match args.query {
                Some(q) => q,
                None => {
                    eprintln!("missing required argument: --query <goal>");
                    return 2;
                }
            };
            (q, args.limit)
        }
        "bson" => {
            if unsafe {
                EncoderDesc::find(m.capabilities.as_ptr(), m.capabilities.len(), "bson").is_none()
            } {
                eprintln!("Unknown or undeclared input format: bson");
                return 2;
            }
            let mut stdin_buf = Vec::new();
            if let Err(e) = std::io::stdin().read_to_end(&mut stdin_buf) {
                // Route through the output encoder: a bson-speaking caller
                // expects a bson response, not plaintext stderr. (IO.md #2.)
                output_result(
                    enc,
                    WireError::Parse(format!("bson request read error: {e}")),
                );
                return 2;
            }
            match crate::wire::parse_bson_request(&stdin_buf) {
                Ok(req) => {
                    // argv --limit overrides the document's limit when present.
                    let lim = args.limit.or(req.limit);
                    (req.query, lim)
                }
                Err(e) => {
                    // A malformed bson request is a parse error at the framing
                    // layer; encode it like any other parse error rather than
                    // breaking the "I only speak bson" contract. (IO.md #2.)
                    output_result(
                        enc,
                        WireError::Parse(format!("bson request parse error: {e}")),
                    );
                    return 2;
                }
            }
        }
        other => {
            eprintln!("Unknown --input-format: {other} (expected text|bson)");
            return 2;
        }
    };
    // A non-streamable encoding (binary) can't coexist with raw `write/1` text
    // bytes on stdout, so run in capture mode: `write/1` bytes land in the
    // envelope's `output` field. (IO.md — the encoding dictates the sink.)
    if let Some(e) = enc
        && !(e.can_stream)()
    {
        m.output = crate::machine::OutputSink::Capture(String::new());
    }
    m.solution_limit = argv_limit;
    // Documented extension over v1 (which hardcoded 10_000): the step
    // ceiling is tunable via environment so big-but-legitimate queries
    // can raise it without changing the CLI contract.
    if let Ok(s) = std::env::var("PLG_MAX_STEPS")
        && let Ok(n) = s.parse::<u64>()
    {
        m.step_limit = n;
    }
    // Same knob for the metacall depth bound (#23): tune it to the stack the
    // binary will run under — lower it for a small `ulimit -s`, raise it on a
    // generous stack. Mirrors `PLG_MAX_STEPS`; the default (1000) is set in
    // `Machine::new`.
    if let Ok(s) = std::env::var("PLG_METACALL_DEPTH")
        && let Ok(n) = s.parse::<usize>()
    {
        m.metacall_depth_limit = n;
    }

    match core::run_query(m, &query) {
        QueryResult::ParseError(msg) => {
            output_result(enc, WireError::Parse(msg));
            2
        }
        QueryResult::RuntimeError(msg) => {
            output_result(enc, WireError::Runtime(msg));
            3
        }
        QueryResult::Solutions => {
            let count = m.solutions.len();
            let exhausted = core::exhausted(m);
            output_solutions(enc, m, exhausted);
            if count > 0 { 1 } else { 0 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_args(v.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn parses_flags_with_space_and_equals() {
        let a = args(&["--query", "p(X)", "--limit", "3", "--format", "text"]).unwrap();
        assert_eq!(a.query.as_deref(), Some("p(X)"));
        assert_eq!(a.limit, Some(3));
        assert_eq!(a.format, "text");
        assert_eq!(a.input_format, "text", "default input-format is text");

        let a = args(&["--query=p(X)", "-l", "1"]).unwrap();
        assert_eq!(a.query.as_deref(), Some("p(X)"));
        assert_eq!(a.limit, Some(1));
        assert_eq!(a.format, "json", "default format is json (v1)");
    }

    #[test]
    fn parses_input_format_flag() {
        let a = args(&["--query", "p(X)", "--input-format", "bson"]).unwrap();
        assert_eq!(a.input_format, "bson");
        // --query is now optional at the parse layer (required only for
        // text-input mode, enforced in plg_rt_main); missing it parses fine.
        assert!(args(&["--input-format", "bson"]).is_ok());
    }

    #[test]
    fn missing_value_flags_are_errors() {
        assert!(args(&["--query"]).is_err());
        assert!(args(&["--bogus", "x"]).is_err());
        assert!(args(&["--input-format"]).is_err());
    }
}
