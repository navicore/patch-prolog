//! THROWAWAY Tier 2 gate spike (docs/design/WASM.md): a reactor-module ABI for
//! `wasm32-unknown-unknown` (Cloudflare Workers / V8 isolates). No WASI, no
//! stdio/argv — the module exports functions a JS host calls over linear
//! memory:
//!
//!   plg_init            (emitted by the generated module) → hands us the Machine
//!   plg_rt_alloc(len)   → ptr        host writes the query bytes here
//!   plg_rt_run_query(ptr,len) → u64  packed (len<<32 | ptr) of a JSON buffer
//!   plg_rt_free(ptr,len)             host frees the result
//!
//! This proves the query path runs in an isolate with no WASI. It duplicates
//! the JSON formatting from `entry.rs` on purpose — productization extracts a
//! single I/O-free core shared by the WASI shell and this one.

use crate::machine::Machine;
use crate::{query, render, solve};
use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::{AtomicPtr, Ordering};

/// Exact-`Layout` allocation keyed by byte length, so the host can free a
/// buffer with just its length — `Vec::with_capacity` may over-allocate, and
/// freeing with the requested (not actual) size corrupts the allocator.
fn raw_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    // SAFETY: len > 0; align 1 is always valid for bytes.
    unsafe { alloc(Layout::from_size_align_unchecked(len, 1)) }
}

/// The program Machine, built once by the generated `plg_init`. wasm is
/// single-threaded, so `Relaxed` is sufficient.
static MACHINE: AtomicPtr<Machine> = AtomicPtr::new(std::ptr::null_mut());

/// # Safety
/// Called once from the generated `plg_init` with the `plg_rt_init` result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_set_machine(m: *mut Machine) {
    // Spike: the reactor doesn't wire a per-request step ceiling yet, so lift
    // it for the session (productization passes it alongside the query).
    unsafe { (*m).step_limit = 1_000_000_000 };
    MACHINE.store(m, Ordering::Relaxed);
}

/// Allocate a host-writable buffer in linear memory (query in / result out).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_alloc(len: u32) -> *mut u8 {
    raw_alloc(len as usize)
}

/// # Safety
/// `ptr`/`len` must be exactly a prior `plg_rt_alloc`/`plg_rt_run_query` pair.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_free(ptr: *mut u8, len: u32) {
    if len == 0 {
        return;
    }
    unsafe { dealloc(ptr, Layout::from_size_align_unchecked(len as usize, 1)) };
}

/// Run one query (UTF-8 at `qptr..qptr+qlen`); return packed `(len << 32) | ptr`
/// of a JSON byte buffer the host reads then frees via `plg_rt_free`.
///
/// # Safety
/// Requires `plg_init` to have run first; `qptr`/`qlen` a valid buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_run_query(qptr: *const u8, qlen: u32) -> u64 {
    let m = unsafe { &mut *MACHINE.load(Ordering::Relaxed) };
    reset(m);
    let q = std::str::from_utf8(unsafe { std::slice::from_raw_parts(qptr, qlen as usize) })
        .unwrap_or("");
    let json = run_core(m, q);
    // Copy into an exact-Layout buffer so the host frees it with just `len`.
    let bytes = json.as_bytes();
    let out = raw_alloc(bytes.len());
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    ((bytes.len() as u64) << 32) | (out as u32 as u64)
}

/// Clear per-query state, keeping the program (atoms/registry/srcmap/limits).
/// The spike rebuilds nothing — a fresh `Machine::new` has exactly this state.
fn reset(m: &mut Machine) {
    m.heap.clear();
    m.trail.clear();
    m.cps.clear();
    m.steps = 0;
    m.error = None;
    m.error_site = crate::machine::NO_SITE;
    m.query_vars.clear();
    m.findall_stack.clear();
    m.qbarrier = 0;
    m.metacall_depth = 0;
    m.solutions.clear();
    m.solution_limit = None;
}

fn run_core(m: &mut Machine, q: &str) -> String {
    let goal = match query::parse_query(m, q) {
        Ok(g) => g,
        Err(e) => return error_json(&format!("Parse error: {e}")),
    };
    match solve::solve(m, goal) {
        solve::Outcome::Error => {
            let msg = m.error.take().map(|e| e.message).unwrap_or_default();
            error_json(&format!("Runtime error: {msg}"))
        }
        solve::Outcome::Done => solutions_json(m),
    }
}

fn error_json(message: &str) -> String {
    format!("{{\"error\":\"{}\"}}", render::json_escape(message))
}

fn solutions_json(m: &Machine) -> String {
    let solutions: Vec<String> = m
        .solutions
        .iter()
        .map(|sol| {
            let fields: Vec<String> = sol
                .bindings
                .iter()
                .map(|(name, json, _)| format!("\"{}\":{}", render::json_escape(name), json))
                .collect();
            format!("{{{}}}", fields.join(","))
        })
        .collect();
    format!(
        "{{\"count\":{},\"exhausted\":true,\"solutions\":[{}]}}",
        m.solutions.len(),
        solutions.join(",")
    )
}
