//! Tier-2 reactor ABI for `wasm32-unknown-unknown` (Cloudflare Workers / V8
//! isolates). No WASI, no stdio/argv — the module *exports* functions a JS
//! host calls over linear memory (docs/design/done/WASM_TIER2_PLAN.md A3):
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
use crate::wire::{Envelope, PLG_ENC_JSON, WireError};
use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

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

/// Module-default limits, captured from the Machine at init. A per-request `0`
/// means "use the module default" — and because the reactor reuses ONE Machine
/// across every request (and `reset_per_query` deliberately leaves the limit
/// fields alone, correct for the CLI's set-once use), we must restore these
/// explicitly each request. Otherwise `0` would inherit the *previous*
/// request's value — a cross-request latch in exactly the reuse scenario this
/// module exists for.
static DEFAULT_STEP_LIMIT: AtomicU64 = AtomicU64::new(0);
static DEFAULT_DEPTH_LIMIT: AtomicU64 = AtomicU64::new(0);

/// # Safety
/// Called once from the generated `plg_init` with the `plg_rt_init` result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_set_machine(m: *mut Machine) {
    // No stdout in a V8 isolate: capture `write/1` output into the result JSON
    // (D4) instead of streaming it nowhere.
    unsafe { (*m).output = OutputSink::Capture(String::new()) };
    // Snapshot the limits codegen/`plg_init` baked in, so a per-request `0`
    // restores them rather than latching the prior request's override.
    DEFAULT_STEP_LIMIT.store(unsafe { (*m).step_limit }, Ordering::Relaxed);
    DEFAULT_DEPTH_LIMIT.store(
        unsafe { (*m).metacall_depth_limit } as u64,
        Ordering::Relaxed,
    );
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
    // Assign all three limits UNCONDITIONALLY: `reset_per_query` doesn't touch
    // the limit fields (set-once for the CLI), so on the reused reactor Machine
    // a non-zero arg overrides and `0` restores the module default — never the
    // previous request's value.
    m.solution_limit = if limit == 0 {
        None
    } else {
        Some(limit as usize)
    };
    m.step_limit = if step_limit != 0 {
        step_limit
    } else {
        DEFAULT_STEP_LIMIT.load(Ordering::Relaxed)
    };
    m.metacall_depth_limit = if depth_limit != 0 {
        depth_limit as usize
    } else {
        DEFAULT_DEPTH_LIMIT.load(Ordering::Relaxed) as usize
    };

    let q = std::str::from_utf8(unsafe { std::slice::from_raw_parts(qptr, qlen as usize) })
        .unwrap_or("");

    let mut buf = Vec::new();
    // Writes never fail (a `Vec` sink), so the `io::Result`s are infallible.
    match core::run_query(m, q) {
        QueryResult::ParseError(msg) => {
            let _ = (PLG_ENC_JSON.write_error)(&mut buf, &WireError::Parse(msg));
        }
        QueryResult::RuntimeError(msg) => {
            let _ = (PLG_ENC_JSON.write_error)(&mut buf, &WireError::Runtime(msg));
        }
        QueryResult::Solutions => {
            let exhausted = core::exhausted(m);
            // Capture mode → `captured_output()` is always `Some` (`""` when
            // nothing was written), so the result always carries an `output`
            // field. Intended D4 contract: a stable shape for hosts, present
            // even when empty.
            let env = Envelope::from_machine(m, exhausted);
            let _ = (PLG_ENC_JSON.write_envelope)(&mut buf, m, &env);
        }
    }

    // Copy into an exact-Layout buffer so the host frees it with just `len`.
    let out = raw_alloc(buf.len());
    unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, buf.len()) };
    ((buf.len() as u64) << 32) | (out as u32 as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    /// Build and register a Machine the way the generated `plg_init` does.
    /// Leaks the Machine and any result buffers — fine for a test, and only
    /// this test touches the `MACHINE`/`DEFAULT_*` statics, so no race.
    fn install() -> *mut Machine {
        let m = Box::into_raw(Machine::new(StringInterner::new(), Vec::new()));
        // SAFETY: mirrors `plg_init` handing us a freshly built Machine.
        unsafe { plg_rt_set_machine(m) };
        m
    }

    fn run(q: &str, limit: u32, step: u64, depth: u32) {
        let b = q.as_bytes();
        // SAFETY: `b` is a valid UTF-8 buffer; a Machine is installed above.
        let _ = unsafe { plg_rt_run_query(b.as_ptr(), b.len() as u32, limit, step, depth) };
    }

    #[test]
    fn zero_limits_restore_module_default_not_previous_request() {
        let m = install();
        // The module defaults `plg_init`/`Machine::new` baked in.
        let (def_step, def_depth) = unsafe { ((*m).step_limit, (*m).metacall_depth_limit) };

        // Request 1: explicit non-default per-request limits.
        run("x", 0, 5_000, 50);
        unsafe {
            assert_eq!((*m).step_limit, 5_000);
            assert_eq!((*m).metacall_depth_limit, 50);
        }

        // Request 2: `0` => module default, NOT request 1's latched values.
        run("x", 0, 0, 0);
        unsafe {
            assert_eq!(
                (*m).step_limit,
                def_step,
                "step_limit must revert to the module default"
            );
            assert_eq!(
                (*m).metacall_depth_limit,
                def_depth,
                "metacall_depth_limit must revert to the module default"
            );
        }
    }
}
