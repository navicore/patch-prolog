//! Fact-table lookup (FACT_TABLE.md, Stages A–B): the generic enumerator for a
//! predicate compiled to a `.rodata` immediate-word table instead of one
//! function per clause. Same observable behavior as the per-clause facts —
//! solution order = program order, choice-point backtracking — mirroring
//! `between/3`'s nondeterministic shape.
//!
//! Stage B adds a first-argument index: a `.rodata` array of row indices
//! sorted by column 0. When the first query argument is a bound atom/int, a
//! binary search narrows the scan to the matching row range (O(log n) instead
//! of O(n)); otherwise (unbound, or a non-immediate first arg) the scan covers
//! all rows, exactly as Stage A. Either way the matching rows are visited in
//! program order, so output is identical to the per-clause path.
//!
//! Delivery to the continuation is a `musttail` in the GENERATED entry/retry
//! functions, not here: these helpers only find/bind a row and push the
//! choice point, then RETURN. That return pops their frame before the
//! generated code tail-calls the continuation, so recursion *through* a fact
//! predicate (e.g. `edge` in a recursive `path/2`) keeps a constant C stack.

use crate::cell::{TAG_ATOM, TAG_INT, Word, tag_of};
use crate::machine::{ContFn, Machine};
use crate::unify::unify;

// Control-frame layout (heap cells): a fixed prefix, then the arg snapshots.
const TBL: usize = 0; // table pointer (ptrtoint of the .rodata global)
const NROWS: usize = 1;
const ARITY: usize = 2;
const IDX: usize = 3; // first-arg index pointer, or 0 for a full scan (no
// real `.rodata` global lives at address 0, so 0 is a safe sentinel — see
// `fact_scan` for how a position maps to a row through this index, or to the
// row directly when 0)
const CURSOR: usize = 4; // next position in [.., END) to try (see fact_scan
// for the position→row mapping); mutated in place, untrailed
const END: usize = 5; // exclusive upper bound of the cursor's range
const RETRY: usize = 6; // the predicate's generated `@..._ftr` (a ContFn)
const KFN: usize = 7;
const KENV: usize = 8;
const QBAR: usize = 9;
const ARGS: usize = 10; // arg snapshots start here

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

/// Borrow the `.rodata` table as `nrows * arity` immediate words.
///
/// # Safety
/// `ptr` is a generated `.rodata` global of exactly that length (FACT_TABLE.md)
/// — the same kind of codegen-emitted, read-only table the runtime already
/// reads for the atom table and predicate registry.
unsafe fn table_slice<'a>(ptr: u64, len: usize) -> &'a [Word] {
    unsafe { std::slice::from_raw_parts(ptr as usize as *const Word, len) }
}

/// Compiled entry: snapshot the args + continuation into a control frame, pick
/// the row range to scan (binary search on the index when the first arg is a
/// bound atom/int, else all rows), then find the first matching row. Returns 1
/// if a solution was set up (the generated entry then musttails the
/// continuation), 0 if no row matches.
///
/// # Safety
/// Called from generated code. `table_ptr` addresses a `.rodata` array of
/// exactly `nrows * arity` immediate words; `idx_ptr` is either 0 or a
/// `.rodata` array of `nrows` row indices sorted by column 0; `retry_ptr` is
/// the predicate's `@..._ftr` function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plg_rt_fact_first(
    m: *mut Machine,
    table_ptr: i64,
    idx_ptr: i64,
    nrows: i64,
    arity: i64,
    retry_ptr: i64,
) -> i32 {
    let m = unsafe { &mut *m };
    let nrows = nrows as usize;
    let arity = arity as usize;

    // Choose the iteration space. The index is consulted only when the first
    // argument is a bound atom/int — the exact domain of column 0 — so that
    // Word-equality (what the binary search tests) coincides with what `unify`
    // would accept. Unbound or non-immediate first args fall back to a full
    // scan, identical to Stage A.
    let (idx_for_frame, cursor0, end) = if arity == 0 {
        (0u64, 0usize, nrows)
    } else {
        let a0 = m.deref(m.areg[0]);
        if idx_ptr != 0 && matches!(tag_of(a0), TAG_ATOM | TAG_INT) {
            let table = unsafe { table_slice(table_ptr as u64, nrows * arity) };
            let idx = unsafe { table_slice(idx_ptr as u64, nrows) };
            let (lo, hi) = equal_range(idx, table, arity, a0);
            (idx_ptr as u64, lo, hi)
        } else {
            (0u64, 0usize, nrows)
        }
    };

    let frame = m.frame_alloc(ARGS + arity);
    m.heap[frame + TBL] = table_ptr as u64;
    m.heap[frame + NROWS] = nrows as u64;
    m.heap[frame + ARITY] = arity as u64;
    m.heap[frame + IDX] = idx_for_frame;
    m.heap[frame + CURSOR] = cursor0 as u64;
    m.heap[frame + END] = end as u64;
    m.heap[frame + RETRY] = retry_ptr as u64;
    m.heap[frame + KFN] = m.k_fn as usize as u64;
    m.heap[frame + KENV] = m.k_env;
    m.heap[frame + QBAR] = m.qbarrier as u64;
    for c in 0..arity {
        m.heap[frame + ARGS + c] = m.areg[c];
    }
    // `m.k_fn`/`k_env` are unchanged (the caller's continuation), so the
    // generated `deliver:` reads the right continuation — no restore needed
    // here (unlike `plg_rt_fact_next`, which runs after the driver may have
    // overwritten them).
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

/// `[lower, upper)` positions in `idx` whose column-0 word equals `key`. `idx`
/// is sorted by `table[idx[k] * arity]` as a `u64`; equality search is correct
/// on any consistent total order, so the raw `u64` comparator suffices (we
/// never order-compare across keys). Requires `arity >= 1`.
fn equal_range(idx: &[Word], table: &[Word], arity: usize, key: Word) -> (usize, usize) {
    let col0 = |k: usize| table[idx[k] as usize * arity];
    // lower bound: first k with col0(k) >= key
    let (mut lo, mut hi) = (0usize, idx.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if col0(mid) < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let lower = lo;
    // upper bound: first k with col0(k) > key (search the [lower, len) tail)
    hi = idx.len();
    let mut lo2 = lower;
    while lo2 < hi {
        let mid = lo2 + (hi - lo2) / 2;
        if col0(mid) <= key {
            lo2 = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lower, lo2)
}

/// Scan positions `[cursor, END)` for the first row that unifies with the
/// snapshot args. A row's actual index is `IDX[cursor]` when an index is set,
/// else `cursor` itself. On a match: advance the cursor, push a choice point
/// for the rest of the range whose restore-point is the *pre-binding* state
/// (so backtracking undoes exactly this row), and return 1 — binding the row
/// only once. Returns 0 when no remaining position matches.
fn fact_scan(m: &mut Machine, frame: usize) -> i32 {
    let nrows = m.heap[frame + NROWS] as usize;
    let arity = m.heap[frame + ARITY] as usize;
    let idx_ptr = m.heap[frame + IDX];
    let end = m.heap[frame + END] as usize;
    let retry = unsafe { read_contfn(m.heap[frame + RETRY]) };
    let table = unsafe { table_slice(m.heap[frame + TBL], nrows * arity) };
    let idx: Option<&[Word]> = (idx_ptr != 0).then(|| unsafe { table_slice(idx_ptr, nrows) });

    // Restore-point for the next alternative: the state before any row here
    // binds. A failed row rewinds to it and retries; a matched row leaves its
    // bindings in place and hands this mark to the choice point.
    let clean_t = m.trail.len();
    let clean_h = m.heap.len();
    loop {
        let pos = m.heap[frame + CURSOR] as usize;
        if pos >= end {
            return 0;
        }
        let row_idx = match idx {
            Some(ix) => ix[pos] as usize,
            None => pos,
        };
        let row = &table[row_idx * arity..row_idx * arity + arity];
        let mut matched = true;
        for (c, &col) in row.iter().enumerate() {
            let a = m.heap[frame + ARGS + c];
            if !unify(m, a, col) {
                matched = false;
                break;
            }
        }
        // Advance both branches. The write is intentionally untrailed: the
        // frame cell survives the choice point's heap truncation, so the
        // cursor advances monotonically across a CP resume (the next solution
        // continues from here, never re-tries `pos`).
        m.heap[frame + CURSOR] = (pos + 1) as u64;
        if matched {
            if pos + 1 < end {
                m.push_cp_at(retry, frame as u64, clean_t, clean_h);
            }
            return 1;
        }
        m.rewind_to(clean_t, clean_h);
    }
}
