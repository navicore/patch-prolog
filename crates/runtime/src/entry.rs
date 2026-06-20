//! Process entry: `plg_rt_init` + `plg_rt_main`, called from the thin
//! generated `main`. Owns argv parsing (hand-rolled — no clap inside
//! compiled binaries), output, and the v1 exit-code contract:
//!
//!   0 = no solutions, 1 = solutions found,
//!   2 = query parse error, 3 = runtime error

use crate::core::{self, QueryResult};
use crate::machine::{Machine, RegistryEntry, SrcLoc};
use plg_shared::StringInterner;
use std::ffi::CStr;
use std::io::{self, Write};
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
    Box::into_raw(m)
}

struct Args {
    query: String,
    limit: Option<usize>,
    format: String,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut query = None;
    let mut limit = None;
    let mut format = "json".to_string(); // v1 default
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
            "-h" | "--help" => {
                return Err("usage: --query <goal> [--limit N] [--format json|text]".to_string());
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    let query = query.ok_or("missing required argument: --query <goal>".to_string())?;
    Ok(Args {
        query,
        limit,
        format,
    })
}

/// v1's output_error: JSON errors go to stdout, text errors to stderr.
/// The JSON shape is the shared core's; only the routing is CLI-specific.
fn output_error(format: &str, message: &str) {
    if format == "json" {
        let mut out = io::stdout().lock();
        let _ = core::write_error_json(&mut out, message);
        let _ = out.write_all(b"\n");
    } else {
        eprintln!("Error: {message}");
    }
}

fn output_json(m: &Machine, exhausted: bool) {
    let mut out = io::stdout().lock();
    // `None` output: the CLI streamed any `write/1` bytes to stdout already, so
    // its JSON stays byte-identical to v1 (no `output` field).
    let _ = core::write_solutions_json(&mut out, m, exhausted, None);
    let _ = out.write_all(b"\n");
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
            for (name, _, text) in &sol.bindings {
                println!("{name} = {text}");
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
    if args.format != "json" && args.format != "text" {
        output_error("text", &format!("Unknown format: {}", args.format));
        return 2;
    }
    m.solution_limit = args.limit;
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

    match core::run_query(m, &args.query) {
        QueryResult::ParseError(msg) => {
            output_error(&args.format, &msg);
            2
        }
        QueryResult::RuntimeError(msg) => {
            output_error(&args.format, &msg);
            3
        }
        QueryResult::Solutions => {
            let count = m.solutions.len();
            let exhausted = core::exhausted(m);
            match args.format.as_str() {
                "json" => output_json(m, exhausted),
                _ => output_text(m),
            }
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
        assert_eq!(a.query, "p(X)");
        assert_eq!(a.limit, Some(3));
        assert_eq!(a.format, "text");

        let a = args(&["--query=p(X)", "-l", "1"]).unwrap();
        assert_eq!(a.query, "p(X)");
        assert_eq!(a.limit, Some(1));
        assert_eq!(a.format, "json", "default format is json (v1)");
    }

    #[test]
    fn missing_query_is_an_error() {
        assert!(args(&["--format", "json"]).is_err());
        assert!(args(&["--query"]).is_err());
        assert!(args(&["--bogus", "x"]).is_err());
    }
}
