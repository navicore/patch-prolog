//! The Machine: the single runtime context every compiled predicate
//! receives (`%M` in generated IR). Owns the term heap, trail,
//! choice-point stack, argument registers, the current success
//! continuation, step accounting, and the runtime atom table.

use crate::cell::{self, Word};
use plg_shared::StringInterner;

/// Uniform signature shared by every compiled predicate, continuation,
/// and retry function — and by runtime-provided continuations. The
/// uniform C-ABI prototype is what makes `musttail` transfers valid.
/// Returns 0 = fail/exhausted (driver backtracks), 1 = stop (limit hit
/// or final success).
pub type ContFn = unsafe extern "C" fn(*mut Machine, u64) -> i32;

/// Maximum predicate arity passed via argument registers (v1 had no
/// practical limit; raise alongside a frame-passing scheme if ever hit).
pub const MAX_ARGS: usize = 16;

/// Registry row emitted by codegen as `{ i32, i32, ptr }`, sorted by
/// (functor, arity) for binary search.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RegistryEntry {
    pub functor: u32,
    pub arity: u32,
    pub f: ContFn,
}

/// One source-location row of the codegen-emitted `@plg_srcmap` side-table
/// (SPANS.md Layer 3). A raising call site passes its index (`site_id`) and
/// the error path resolves it to `file:line:col`. Layout mirrors the IR's
/// `{i32, i32, i32}`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SrcLoc {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

/// `site_id` sentinel meaning "no source location" — runtime-internal
/// raises (query-side undefined goals) and any binary built without
/// provenance. The error message gets no `at file:line:col` suffix.
///
/// ABI contract: MUST equal `plg_compiler::codegen::NO_SITE` (codegen emits
/// this value as the `site_id` arg). Separate consts in separate crates;
/// each pins `== u32::MAX` in a unit test to flag a one-sided renumber.
pub const NO_SITE: u32 = u32::MAX;

/// RAII guard for the in-flight raise's site (SPANS.md Layer 3). A raising
/// compiled builtin creates one at its ABI boundary; `Drop` **restores the
/// previous value** (not `NO_SITE`), so the set/clear can't be forgotten and
/// nested raises — an outer builtin whose work reaches an inner raising
/// builtin — each see their own site without a save/restore stack. Use this
/// instead of hand-writing `m.error_site = site; ...; m.error_site = NO_SITE`.
///
/// FOOTGUN: bind to a **named** variable — `let _site = ...;`. `let _ = ...`
/// drops the guard immediately, so the body runs with the site already
/// restored (no provenance, or the caller's site). `#[must_use]` does not
/// catch the `let _` form.
#[must_use = "binding to `let _` drops the guard immediately; use `let _site = ...`"]
pub(crate) struct ErrorSiteGuard {
    m: *mut Machine,
    saved: u32,
}

impl ErrorSiteGuard {
    pub(crate) fn enter(m: *mut Machine, site_id: u32) -> Self {
        // SAFETY: `m` is the valid Machine pointer the builtin was called
        // with. We touch only `error_site`, and the `Drop` write runs after
        // the builtin's `&mut Machine` borrows have ended.
        let saved = unsafe { (*m).error_site };
        unsafe { (*m).error_site = site_id };
        ErrorSiteGuard { m, saved }
    }
}

impl Drop for ErrorSiteGuard {
    fn drop(&mut self) {
        unsafe { (*self.m).error_site = self.saved };
    }
}

/// Catch frames participate in error unwinding (drive() in solve.rs)
/// and stop cut truncation (v1 rule: catch is opaque to cut).
#[derive(Clone, Copy, PartialEq)]
pub enum CpKind {
    Normal,
    Catch,
}

pub struct ChoicePoint {
    pub trail_mark: usize,
    pub heap_mark: usize,
    pub retry: ContFn,
    pub env: u64,
    pub kind: CpKind,
}

/// A runtime error in flight. The ball is a relocatable copy (it must
/// survive heap rewinding on the way to a catch frame); the message is
/// its v1-format rendering for top-level output (exit code 3).
pub struct RtError {
    pub ball: crate::copyterm::TermBuf,
    pub message: String,
    pub uncatchable: bool,
}

pub struct Machine {
    pub heap: Vec<Word>,
    pub trail: Vec<u64>, // heap indices of bound REF cells
    pub cps: Vec<ChoicePoint>,
    pub areg: [Word; MAX_ARGS],
    /// Build registers for `plg_rt_put_struct` (separate from areg so
    /// argument setup and term construction never clobber each other).
    pub breg: [Word; MAX_ARGS],
    pub k_fn: ContFn,
    pub k_env: u64,
    pub steps: u64,
    pub step_limit: u64,
    /// Live nesting depth of the runtime goal-walker (`call_goal`). Compiled
    /// predicate-to-predicate transfers `musttail` and never touch this; only
    /// runtime-walked goals (queries, `call/N`, `findall/3`, `catch/3`
    /// recovery) recurse the C stack here. Bounded by `metacall_depth_limit`
    /// so a deep non-trampolined metacall fails gracefully instead of
    /// overflowing the native stack (#23).
    pub metacall_depth: usize,
    pub metacall_depth_limit: usize,
    pub error: Option<RtError>,
    pub atoms: StringInterner,
    pub registry: Vec<RegistryEntry>,
    /// Source-location side-table (SPANS.md Layer 3), handed over by codegen
    /// at init. Empty for binaries built without provenance.
    pub srcmap: Vec<SrcLoc>,
    /// `file_id` → filename, parallel to `srcmap`'s `file` field.
    pub files: Vec<String>,
    /// `site_id` of the raise currently in flight (SPANS.md Layer 3).
    /// `set_formal` appends ` at file:line:col` from it. `NO_SITE` (the
    /// default, and the value for runtime-internal/query-side raises) means no
    /// suffix — keeping those messages byte-identical to v1.
    ///
    /// INVARIANT: a raising compiled builtin must set this to its site only
    /// around its own work and restore it after — always via `ErrorSiteGuard`,
    /// which makes the set/restore impossible to forget and keeps nested
    /// raises correct (each restores the caller's site, not `NO_SITE`).
    pub error_site: u32,
    /// Query variables in source order: (name, heap index of the cell).
    pub query_vars: Vec<(String, usize)>,
    /// findall/3 collector stack (a stack because findall can nest):
    /// each level accumulates relocatable copies of template instances.
    pub findall_stack: Vec<Vec<crate::copyterm::TermBuf>>,
    /// Cut barrier for `!` in RUNTIME-WALKED goals (queries, metacalls).
    /// Call-like constructs set it for their inner goal; every walker
    /// continuation frame snapshots and restores it (the runtime mirror
    /// of the compiled cut_slot). Compiled `!` never reads this.
    pub qbarrier: usize,
    /// Solutions captured by the print continuation, already rendered.
    pub solutions: Vec<crate::render::RenderedSolution>,
    pub solution_limit: Option<usize>,
}

unsafe extern "C" fn no_continuation(_m: *mut Machine, _env: u64) -> i32 {
    // Reaching the continuation with none installed is a codegen bug.
    debug_assert!(false, "no continuation installed");
    0
}

impl Machine {
    pub fn new(atoms: StringInterner, registry: Vec<RegistryEntry>) -> Box<Machine> {
        Box::new(Machine {
            heap: Vec::with_capacity(4096),
            trail: Vec::with_capacity(256),
            cps: Vec::with_capacity(64),
            areg: [0; MAX_ARGS],
            breg: [0; MAX_ARGS],
            k_fn: no_continuation,
            k_env: 0,
            steps: 0,
            step_limit: 10_000, // v1 default
            metacall_depth: 0,
            // Conservative: well below the native C-stack capacity (~5-6k
            // walker frames overflow an 8MB stack). The trampoline keeps the
            // common `call(pred)` tail recursion off this path entirely, so
            // this only bounds rare control-construct / findall recursion.
            metacall_depth_limit: 1000,
            error: None,
            atoms,
            registry,
            srcmap: Vec::new(),
            files: Vec::new(),
            error_site: NO_SITE,
            query_vars: Vec::new(),
            findall_stack: Vec::new(),
            qbarrier: 0,
            solutions: Vec::new(),
            solution_limit: None,
        })
    }

    /// Allocate a fresh unbound variable cell; returns its REF word.
    pub fn new_var(&mut self) -> Word {
        let idx = self.heap.len();
        self.heap.push(cell::make_ref(idx)); // unbound = self-reference
        cell::make_ref(idx)
    }

    /// Allocate `n` raw cells (a frame); returns the base index.
    /// Frames hold continuation state — they are not terms and must
    /// never be unified.
    pub fn frame_alloc(&mut self, n: usize) -> usize {
        let idx = self.heap.len();
        self.heap.resize(idx + n, 0);
        idx
    }

    /// Bind an unbound REF cell to a value, recording it on the trail.
    pub fn bind(&mut self, ref_idx: usize, value: Word) {
        debug_assert_eq!(
            self.heap[ref_idx],
            cell::make_ref(ref_idx),
            "bind target must be unbound"
        );
        self.heap[ref_idx] = value;
        self.trail.push(ref_idx as u64);
    }

    /// Follow REF chains to the representative word. Bound chains are
    /// acyclic (we only ever bind unbound cells), so this terminates.
    pub fn deref(&self, mut w: Word) -> Word {
        while cell::tag_of(w) == cell::TAG_REF {
            let idx = cell::payload(w) as usize;
            let c = self.heap[idx];
            if c == w {
                return w; // unbound
            }
            w = c;
        }
        w
    }

    pub fn push_cp(&mut self, retry: ContFn, env: u64) {
        self.cps.push(ChoicePoint {
            trail_mark: self.trail.len(),
            heap_mark: self.heap.len(),
            retry,
            env,
            kind: CpKind::Normal,
        });
    }

    /// Push a choice point whose backtrack restore-point is an EXPLICIT mark,
    /// captured before the current alternative bound anything — rather than
    /// the live heap/trail top. Lets a nondeterministic builtin bind a
    /// solution and still record the pre-binding state for the next
    /// alternative, without a rewind-then-rebind.
    ///
    /// **CALLER MUST GUARANTEE** `trail_mark <= self.trail.len()` and
    /// `heap_mark <= self.heap.len()`. An out-of-range mark is caught only by
    /// the debug assertion below; in release it silently no-ops the backtrack
    /// truncation (`Vec::truncate(n)` with `n > len` does nothing), leaking
    /// bindings from a supposedly-undone alternative into the next.
    pub fn push_cp_at(&mut self, retry: ContFn, env: u64, trail_mark: usize, heap_mark: usize) {
        debug_assert!(trail_mark <= self.trail.len() && heap_mark <= self.heap.len());
        self.cps.push(ChoicePoint {
            trail_mark,
            heap_mark,
            retry,
            env,
            kind: CpKind::Normal,
        });
    }

    pub fn push_catch_cp(&mut self, retry: ContFn, env: u64) {
        self.cps.push(ChoicePoint {
            trail_mark: self.trail.len(),
            heap_mark: self.heap.len(),
            retry,
            env,
            kind: CpKind::Catch,
        });
    }

    /// Cut: truncate the CP stack to `height`, but stop at a catch
    /// frame (v1 rule: catch/3 is opaque to cut — `!` inside catch's
    /// goal cannot prune the catch frame or anything below it).
    pub fn cut_to(&mut self, height: usize) {
        while self.cps.len() > height {
            if self.cps.last().is_some_and(|cp| cp.kind == CpKind::Catch) {
                break;
            }
            self.cps.pop();
        }
    }

    /// Rewind bindings and heap to a popped choice point's marks.
    pub fn rewind_to(&mut self, trail_mark: usize, heap_mark: usize) {
        while self.trail.len() > trail_mark {
            let idx = self.trail.pop().unwrap() as usize;
            self.heap[idx] = cell::make_ref(idx);
        }
        self.heap.truncate(heap_mark);
    }

    /// Bump the step counter; on exceeding the limit set the uncatchable
    /// resource error (v1: step limit cannot be trapped by catch/3).
    /// v1 wording, byte-for-byte (solver.rs step_limit_thrown).
    pub fn step(&mut self) -> bool {
        self.steps += 1;
        if self.steps > self.step_limit {
            let context = format!("Maximum step limit exceeded ({})", self.step_limit);
            crate::errors::resource(self, "steps", &context, true);
            return false;
        }
        true
    }

    pub fn registry_lookup(&self, functor: u32, arity: u32) -> Option<ContFn> {
        self.registry
            .binary_search_by_key(&(functor, arity), |e| (e.functor, e.arity))
            .ok()
            .map(|i| self.registry[i].f)
    }

    /// Install the codegen-emitted source-location side-table (SPANS.md
    /// Layer 3). Called once from `plg_rt_init`.
    pub fn set_provenance(&mut self, srcmap: Vec<SrcLoc>, files: Vec<String>) {
        self.srcmap = srcmap;
        self.files = files;
    }

    /// Resolve a `site_id` to `(filename, line, col)`, or `None` for the
    /// `NO_SITE` sentinel / a binary built without provenance. Owned so the
    /// (cold) error path can mutate `self.error` without a borrow conflict.
    pub fn site_location(&self, site_id: u32) -> Option<(String, u32, u32)> {
        if site_id == NO_SITE {
            return None;
        }
        let loc = self.srcmap.get(site_id as usize)?;
        let file = self.files.get(loc.file as usize)?;
        Some((file.clone(), loc.line, loc.col))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::*;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    #[test]
    fn new_var_is_unbound_self_ref() {
        let mut m = machine();
        let v = m.new_var();
        assert_eq!(tag_of(v), TAG_REF);
        assert_eq!(m.deref(v), v);
    }

    #[test]
    fn no_site_sentinel_value_is_pinned() {
        // ABI contract with plg_compiler::codegen::NO_SITE (see its docs).
        assert_eq!(NO_SITE, u32::MAX);
    }

    #[test]
    fn bind_and_rewind() {
        let mut m = machine();
        let v = m.new_var();
        let tmark = m.trail.len();
        let hmark = m.heap.len();
        m.bind(payload(v) as usize, make_atom(7));
        assert_eq!(m.deref(v), make_atom(7));
        m.rewind_to(tmark, hmark);
        assert_eq!(m.deref(v), v, "binding undone");
    }

    #[test]
    fn deref_follows_chains() {
        let mut m = machine();
        let a = m.new_var();
        let b = m.new_var();
        m.bind(payload(a) as usize, b);
        m.bind(payload(b) as usize, make_int(-5));
        assert_eq!(int_value(m.deref(a)), -5);
    }

    #[test]
    fn step_limit_sets_uncatchable_error() {
        let mut m = machine();
        m.step_limit = 2;
        assert!(m.step());
        assert!(m.step());
        assert!(!m.step());
        assert!(m.error.as_ref().unwrap().uncatchable);
    }
}
