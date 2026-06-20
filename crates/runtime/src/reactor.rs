//! Tier-2 reactor ABI for `wasm32-unknown-unknown` (Cloudflare Workers / V8
//! isolates). No WASI, no stdio/argv — the module *exports* functions a JS
//! host calls over linear memory (docs/design/WASM_TIER2_PLAN.md A3):
//!
//!   plg_init                       (emitted by the generated module) → builds
//!                                  the Machine, hands it to `plg_rt_set_machine`
//!   plg_rt_alloc(len) → ptr        host writes the query bytes here
//!   plg_rt_run_query(ptr,len,…) → u64   packed (len<<32 | ptr) of a JSON buffer
//!   plg_rt_free(ptr,len)           host frees the result (or the query buffer)
//!
//! JSON formatting and the query path are NOT duplicated here — both go
//! through `crate::core`, the single I/O-free core the WASI shell shares.
//!
//! ## Concurrency contract (D3 / WASM.md finding #2)
//!
//! **One in-flight query per isolate.** The program Machine is a single
//! `static`; a V8 isolate is single-threaded, but one Worker can interleave
//! async tasks, so the host must not call `plg_rt_run_query` again before the
//! prior call returns. This matches typical Worker use (a request maps to a
//! query) and avoids threading per-request state through the ABI.

use crate::core::{self, QueryResult};
use crate::machine::{Machine, OutputSink};
use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::{AtomicPtr, Ordering};

/// Exact-`Layout` allocation keyed by byte length, so the host can free a
/// buffer with just its length. NEVER `Vec::with_capacity`: a `Vec` may
/// over-allocate, and the host frees by *requested* length, so an actual
/// capacity > requested length corrupts the allocator (WASM.md finding #1 —
/// this is the bug that aborted the spike's deep query; the reflexive reach
/// for `Vec` is the trap).
fn raw_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    // SAFETY: len > 0; align 1 is always valid for bytes.
    unsafe { alloc(Layout::from_size_align_unchecked(len, 1)) }
}

/// The program Machine, built once by the generated `plg_init` and reused for
/// every query (cold-start-per-isolate; never freed — a teardown entry point
/// would only be needed to swap a live isolate's program, WASM.md finding #8).
/// wasm is single-threaded, so `Relaxed` is sufficient.
static MACHINE: AtomicPtr<Machine> = AtomicPtr::new(std::ptr::null_mut());

/// # Safety
/// Called once from the generated `plg_init` with the `plg_rt_init` result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_set_machine(m: *mut Machine) {
    // No stdout in a V8 isolate: capture `write/1` output into the result JSON
    // (D4) instead of streaming it nowhere. Per-query limits arrive with each
    // `plg_rt_run_query`, so nothing else is configured here.
    unsafe { (*m).output = OutputSink::Capture(String::new()) };
    MACHINE.store(m, Ordering::Relaxed);
}

/// Allocate a host-writable buffer in linear memory (query in / result out).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_alloc(len: u32) -> *mut u8 {
    raw_alloc(len as usize)
}

/// # Safety
/// `ptr`/`len` must be exactly a prior `plg_rt_alloc`/`plg_rt_run_query` pair.
/// `len == 0` no-ops to pair with `raw_alloc(0)`'s dangling sentinel (which was
/// never really allocated); the two halves agree by convention, not by API.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_free(ptr: *mut u8, len: u32) {
    if len == 0 {
        return;
    }
    unsafe { dealloc(ptr, Layout::from_size_align_unchecked(len as usize, 1)) };
}

/// Run one query (UTF-8 at `qptr..qptr+qlen`) and return packed
/// `(len << 32) | ptr` of a JSON byte buffer the host reads then frees via
/// `plg_rt_free`. The packed return assumes **wasm32** (the pointer fits in the
/// low 32 bits); wasm64 would need a wider/two-value result (WASM.md finding #7).
///
/// Per-request limits bound the query before the platform's CPU/wall limit does
/// (WASM.md finding #5). All three mirror the CLI's knobs:
/// - `limit`: max solutions; `0` = unbounded.
/// - `step_limit`: step ceiling (`PLG_MAX_STEPS`); `0` = keep the module default.
/// - `depth_limit`: metacall depth bound (`PLG_METACALL_DEPTH`); `0` = keep the
///   default. Depth matters more on wasm: its ~1 MB stack is far smaller than
///   native's ~8 MB.
///
/// # Safety
/// Requires `plg_init` to have run first; `qptr`/`qlen` a valid buffer. See the
/// module's single-in-flight concurrency contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_run_query(
    qptr: *const u8,
    qlen: u32,
    limit: u32,
    step_limit: u64,
    depth_limit: u32,
) -> u64 {
    let m = unsafe { &mut *MACHINE.load(Ordering::Relaxed) };
    m.reset_per_query();
    m.solution_limit = if limit == 0 {
        None
    } else {
        Some(limit as usize)
    };
    if step_limit != 0 {
        m.step_limit = step_limit;
    }
    if depth_limit != 0 {
        m.metacall_depth_limit = depth_limit as usize;
    }

    let q = std::str::from_utf8(unsafe { std::slice::from_raw_parts(qptr, qlen as usize) })
        .unwrap_or("");

    let mut buf = Vec::new();
    // Writes never fail (a `Vec` sink), so the `io::Result`s are infallible.
    match core::run_query(m, q) {
        QueryResult::ParseError(msg) | QueryResult::RuntimeError(msg) => {
            let _ = core::write_error_json(&mut buf, &msg);
        }
        QueryResult::Solutions => {
            let exhausted = core::exhausted(m);
            let _ = core::write_solutions_json(&mut buf, m, exhausted, m.captured_output());
        }
    }

    // Copy into an exact-Layout buffer so the host frees it with just `len`.
    let out = raw_alloc(buf.len());
    unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, buf.len()) };
    ((buf.len() as u64) << 32) | (out as u32 as u64)
}
