//! Process entry: `plg_rt_init` + `plg_rt_main`, called from the thin
//! generated `main`. Owns argv parsing (hand-rolled — no clap inside compiled
//! binaries), output, and the exit-code contract:
//!
//!   0 = no solutions, 1 = solutions found,
//!   2 = query parse / usage error, 3 = runtime error
//!
//! Wire encodings are `text` (readable, default) and `bson` (binary), gated by
//! the codegen-baked capability table (`io_format/1`, default `[text]`). No
//! JSON — a host wanting JSON derives it from bson. See docs/design/IO.md.

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
    caps: *const *const crate::wire::EncoderDesc,
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
    atoms: bool,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut query = None;
    let mut limit = None;
    let mut format = "text".to_string(); // default wire encoding
    let mut input_format = "text".to_string(); // default: argv `--query`
    let mut atoms = false; // bson self-describing mode (--atoms)
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
            "--atoms" => atoms = true,
            "-h" | "--help" => {
                return Err(
                    "usage: --query <goal> [--limit N] [--format text|bson] [--input-format text|bson] [--atoms]"
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
        atoms,
    })
}

/// Emit solutions for a solved query through the chosen encoder. `can_stream()`
/// drives the line discipline: streamed encodings (text) get a trailing flush
/// via the line buffer; non-streamable encodings (bson) get an explicit flush
/// (Rust's stdout is a `LineWriter`, and bson's no-newline bytes would never
/// flush on exit without it).
fn output_solutions(enc: &EncoderDesc, m: &Machine, exhausted: bool, atoms: bool) {
    let mut out = io::stdout().lock();
    let mut env = Envelope::from_machine(m, exhausted);
    // `--atoms`: bson self-describing mode — materialize the post-query atom
    // map (program + query atoms) so the host can decode term values from this
    // document. No-op for text (can_stream renders names directly).
    let atom_names: Vec<String> = if atoms && !(enc.can_stream)() {
        (0..m.atoms.len())
            .map(|i| {
                m.atoms
                    .try_resolve(i as u32)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    } else {
        Vec::new()
    };
    if !atom_names.is_empty() {
        env.atoms = Some(&atom_names);
    }
    let _ = (enc.write_envelope)(&mut out, m, &env);
    if !(enc.can_stream)() {
        let _ = out.flush();
    }
}

/// Emit an error through the chosen encoder. The caller constructs the
/// `WireError` with the correct class (`Parse` vs `Runtime`) so the distinction
/// is honest on the wire.
fn output_result(enc: Option<&EncoderDesc>, err: WireError) {
    let message = match &err {
        WireError::Parse(m) | WireError::Runtime(m) => m.as_str(),
    };
    match enc {
        Some(e) => {
            let mut out = io::stdout().lock();
            let _ = (e.write_error)(&mut out, &err);
            if !(e.can_stream)() {
                let _ = out.flush();
            }
        }
        None => eprintln!("Error: {message}"),
    }
}

/// Run the query named in argv (or a bson request on stdin) against the
/// compiled program. Returns the process exit code.
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
    // this binary advertises via `io_format/1`, default `[text]`). An unknown
    // or undeclared wire format is a usage error (exit 2); there is no valid
    // encoder to serialize the error, so it goes to stderr.
    let enc: Option<&'static EncoderDesc> = match unsafe {
        EncoderDesc::find(m.capabilities.as_ptr(), m.capabilities.len(), &args.format)
    } {
        Some(e) => Some(e),
        None => {
            eprintln!("Unknown or undeclared format: {}", args.format);
            return 2;
        }
    };
    // Standalone `--atoms` (no query): emit the program atom map and exit.
    // No query runs, so the map is program-atoms-only (the boundary: a query
    // that introduces atoms needs the with-query `--atoms` form instead).
    if args.atoms && args.query.is_none() && args.input_format == "text" {
        let mut out = io::stdout().lock();
        let e = enc.unwrap();
        if (e.can_stream)() {
            let _ = crate::wire::write_atom_map_text(&mut out, m);
        } else {
            let _ = crate::wire::write_atom_map_bson(&mut out, m);
            let _ = out.flush();
        }
        return 0;
    }
    // Resolve the query string and limit by input mode. Default (`text`) takes
    // the query from argv `--query` (required). `--input-format bson` reads a
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
                output_result(
                    enc,
                    WireError::Parse(format!("bson request read error: {e}")),
                );
                return 2;
            }
            match crate::wire::parse_bson_request(&stdin_buf) {
                Ok(req) => (req.query, args.limit.or(req.limit)),
                Err(e) => {
                    // A malformed bson request is a parse error at the framing
                    // layer; encode it like any other parse error rather than
                    // breaking the "I only speak bson" contract.
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
    if let Ok(s) = std::env::var("PLG_MAX_STEPS")
        && let Ok(n) = s.parse::<u64>()
    {
        m.step_limit = n;
    }
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
            output_solutions(enc.unwrap(), m, exhausted, args.atoms);
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
        let a = args(&["--query", "p(X)", "--limit", "3", "--format", "bson"]).unwrap();
        assert_eq!(a.query.as_deref(), Some("p(X)"));
        assert_eq!(a.limit, Some(3));
        assert_eq!(a.format, "bson");
        assert_eq!(a.input_format, "text", "default input-format is text");

        let a = args(&["--query=p(X)", "-l", "1"]).unwrap();
        assert_eq!(a.query.as_deref(), Some("p(X)"));
        assert_eq!(a.limit, Some(1));
        assert_eq!(a.format, "text", "default format is text");
    }

    #[test]
    fn parses_input_format_flag() {
        let a = args(&["--query", "p(X)", "--input-format", "bson"]).unwrap();
        assert_eq!(a.input_format, "bson");
        // --query is optional at the parse layer (required only for text-input
        // mode, enforced in plg_rt_main).
        assert!(args(&["--input-format", "bson"]).is_ok());
    }

    #[test]
    fn missing_value_flags_are_errors() {
        assert!(args(&["--query"]).is_err());
        assert!(args(&["--bogus", "x"]).is_err());
        assert!(args(&["--input-format"]).is_err());
    }
}
