//! Fact-table lookup (FACT_TABLE.md, Stage A): the generic enumerator for a
//! predicate compiled to a `.rodata` immediate-word table instead of one
//! function per clause. Same observable behavior as the per-clause facts —
//! solution order = program order, choice-point backtracking — mirroring
//! `between/3`'s nondeterministic shape.
//!
//! Delivery to the continuation is a `musttail` in the GENERATED entry/retry
//! functions, not here: these helpers only find/bind a row and push the
//! choice point, then RETURN. That return pops their frame before the
//! generated code tail-calls the continuation, so recursion *through* a fact
//! predicate (e.g. `edge` in a recursive `path/2`) keeps a constant C stack.

use crate::cell::Word;
use crate::machine::{ContFn, Machine};
use crate::unify::unify;

// Control-frame layout (heap cells): a fixed prefix, then the arg snapshots.
const TBL: usize = 0; // table pointer (ptrtoint of the .rodata global)
const NROWS: usize = 1;
const ARITY: usize = 2;
const CURSOR: usize = 3; // next row to try — mutated in place, untrailed
const RETRY: usize = 4; // the predicate's generated `@..._ftr` (a ContFn)
const KFN: usize = 5;
const KENV: usize = 6;
const QBAR: usize = 7;
const ARGS: usize = 8; // arg snapshots start here

/// Read a `ContFn` from a frame cell that the generated IR wrote via
/// `ptrtoint` — the retry pointer (`@..._ftr`) or the saved continuation
/// (`k_fn`). Centralizes the one invariant both sites share: the cell holds a
/// function pointer to an `i32 (ptr, i64)` we ourselves emitted.
///
/// # Safety
/// `word` must be such a `ptrtoint`-encoded function pointer; nothing else is
/// ever stored in these cells.
unsafe fn read_contfn(word: u64) -> ContFn {
    unsafe { std::mem::transmute::<usize, ContFn>(word as usize) }
}

/// Compiled entry: snapshot the args + continuation into a control frame,
/// then find the first matching row. Returns 1 if a solution was set up (the
/// generated entry then musttails the continuation), 0 if no row matches.
///
/// # Safety
/// Called from generated code. `table_ptr` addresses a `.rodata` array of
/// exactly `nrows * arity` immediate words; `retry_ptr` is the predicate's
/// `@..._ftr` function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_fact_first(
    m: *mut Machine,
    table_ptr: i64,
    nrows: i64,
    arity: i64,
    retry_ptr: i64,
) -> i32 {
    let m = unsafe { &mut *m };
    let arity = arity as usize;
    let frame = m.frame_alloc(ARGS + arity);
    m.heap[frame + TBL] = table_ptr as u64;
    m.heap[frame + NROWS] = nrows as u64;
    m.heap[frame + ARITY] = arity as u64;
    m.heap[frame + CURSOR] = 0;
    m.heap[frame + RETRY] = retry_ptr as u64;
    m.heap[frame + KFN] = m.k_fn as usize as u64;
    m.heap[frame + KENV] = m.k_env;
    m.heap[frame + QBAR] = m.qbarrier as u64;
    for c in 0..arity {
        m.heap[frame + ARGS + c] = m.areg[c];
    }
    // `m.k_fn`/`k_env` are unchanged (the caller's continuation), so the
    // generated `deliver:` reads the right continuation.
    fact_scan(m, frame)
}

/// Choice-point retry: restore the saved continuation (the driver may have
/// overwritten `m.k_fn`), then resume the scan from the frame's cursor.
///
/// # Safety
/// Called by the solve driver with a frame built by `plg_rt_fact_first`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_fact_next(m: *mut Machine, frame: u64) -> i32 {
    let m = unsafe { &mut *m };
    let frame = frame as usize;
    m.k_fn = unsafe { read_contfn(m.heap[frame + KFN]) };
    m.k_env = m.heap[frame + KENV];
    m.qbarrier = m.heap[frame + QBAR] as usize;
    fact_scan(m, frame)
}

/// Scan rows from the cursor for the first that unifies with the snapshot
/// args. On a match: advance the cursor, push a choice point for the rest
/// (whose restore-point is the pre-binding state, so backtracking undoes this
/// row), bind the row, and return 1. Returns 0 when no remaining row matches.
fn fact_scan(m: &mut Machine, frame: usize) -> i32 {
    let nrows = m.heap[frame + NROWS] as usize;
    let arity = m.heap[frame + ARITY] as usize;
    let retry = unsafe { read_contfn(m.heap[frame + RETRY]) };
    // SAFETY: the generated code passed a `.rodata` global of exactly
    // nrows*arity immediate words (FACT_TABLE.md) — the same kind of
    // codegen-emitted, read-only table the runtime already reads for the
    // atom table and predicate registry.
    let table: &[Word] = unsafe {
        std::slice::from_raw_parts(m.heap[frame + TBL] as usize as *const Word, nrows * arity)
    };

    // Pre-attempt state: row tries bind the snapshot args; a failed or
    // committed row rewinds to here.
    let clean_t = m.trail.len();
    let clean_h = m.heap.len();
    let mut i = m.heap[frame + CURSOR] as usize;
    while i < nrows {
        let row = &table[i * arity..i * arity + arity];
        let mut matched = true;
        for (c, &col) in row.iter().enumerate() {
            let a = m.heap[frame + ARGS + c];
            if !unify(m, a, col) {
                matched = false;
                break;
            }
        }
        if matched {
            m.heap[frame + CURSOR] = (i + 1) as u64;
            // Undo the binding so the choice point captures the pre-binding
            // marks (backtracking must undo THIS row), then push and rebind.
            m.rewind_to(clean_t, clean_h);
            if i + 1 < nrows {
                m.push_cp(retry, frame as u64);
            }
            for (c, &col) in row.iter().enumerate() {
                let a = m.heap[frame + ARGS + c];
                unify(m, a, col);
            }
            return 1;
        }
        m.rewind_to(clean_t, clean_h);
        i += 1;
    }
    0
}
