//! The C-ABI surface generated code calls into (`plg_rt_*`).
//!
//! Every function here matches a declaration emitted by plgc codegen;
//! the pairing is documented in docs/design/RUNTIME_ABI.md and protected
//! by the compiler↔runtime version sync in crates/compiler/build.rs.
//!
//! Safety: all `m` pointers originate from `plg_rt_init` and live for
//! the whole process; generated code is single-threaded.

use crate::cell;
use crate::machine::{ContFn, Machine};

#[inline]
fn mref<'a>(m: *mut Machine) -> &'a mut Machine {
    unsafe { &mut *m }
}

/// Bump the step counter. Returns 0 when the limit is exceeded (the
/// uncatchable resource error is set; generated code returns 0).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_step(m: *mut Machine) -> i32 {
    mref(m).step() as i32
}

/// Fresh unbound variable; returns its REF word.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_new_var(m: *mut Machine) -> u64 {
    mref(m).new_var()
}

/// Allocate an `n`-cell continuation frame; returns the base index.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_frame_alloc(m: *mut Machine, n: u32) -> u64 {
    mref(m).frame_alloc(n as usize) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_frame_set(m: *mut Machine, base: u64, i: u32, w: u64) {
    mref(m).heap[base as usize + i as usize] = w;
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_frame_get(m: *mut Machine, base: u64, i: u32) -> u64 {
    mref(m).heap[base as usize + i as usize]
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_areg_get(m: *mut Machine, i: u32) -> u64 {
    mref(m).areg[i as usize]
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_areg_set(m: *mut Machine, i: u32, w: u64) {
    mref(m).areg[i as usize] = w;
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_breg_set(m: *mut Machine, i: u32, w: u64) {
    mref(m).breg[i as usize] = w;
}

/// Build a compound from breg[0..arity]; returns its STR word.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_put_struct(m: *mut Machine, functor: u32, arity: u32) -> u64 {
    let m = mref(m);
    let idx = m.heap.len();
    m.heap.push(cell::pack_functor(functor, arity));
    for i in 0..arity as usize {
        let w = m.breg[i];
        m.heap.push(w);
    }
    cell::make(cell::TAG_STR, idx as u64)
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_put_list(m: *mut Machine, head: u64, tail: u64) -> u64 {
    let m = mref(m);
    let idx = m.heap.len();
    m.heap.push(head);
    m.heap.push(tail);
    cell::make(cell::TAG_LST, idx as u64)
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_put_float(m: *mut Machine, bits: u64) -> u64 {
    let m = mref(m);
    let idx = m.heap.len();
    m.heap.push(bits);
    cell::make(cell::TAG_FLT, idx as u64)
}

/// Box an i64 outside the i61 immediate range (BIG cell).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_put_big(m: *mut Machine, value: i64) -> u64 {
    let m = mref(m);
    let idx = m.heap.len();
    m.heap.push(value as u64);
    cell::make(cell::TAG_BIG, idx as u64)
}

/// Generic unification; returns 1 on success.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_unify(m: *mut Machine, a: u64, b: u64) -> i32 {
    crate::unify::unify(mref(m), a, b) as i32
}

/// Set the success continuation (k_fn passed as a raw pointer-sized
/// integer in IR; the transmute re-types it).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_set_k(m: *mut Machine, k_fn: u64, k_env: u64) {
    let m = mref(m);
    m.k_fn = unsafe { std::mem::transmute::<usize, ContFn>(k_fn as usize) };
    m.k_env = k_env;
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_k_fn(m: *mut Machine) -> u64 {
    mref(m).k_fn as usize as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_k_env(m: *mut Machine) -> u64 {
    mref(m).k_env
}

/// Push a choice point whose retry re-enters generated code.
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_push_cp(m: *mut Machine, retry: u64, env: u64) {
    let m = mref(m);
    let retry = unsafe { std::mem::transmute::<usize, ContFn>(retry as usize) };
    m.push_cp(retry, env);
}

/// Always-fail predicate body: the target for `:- dynamic` predicates
/// with no clauses (silent-fail linter contract).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_pred_fail(_m: *mut Machine, _env: u64) -> i32 {
    0
}

/// Raise existence_error(procedure, F/A) — the compiled stub for body
/// goals that reference a predicate with no clauses (and the runtime
/// path for unknown query goals shares the message shape).
#[unsafe(no_mangle)]
pub extern "C" fn plg_rt_existence_error(
    m: *mut Machine,
    functor: u32,
    arity: u32,
    site_id: u32,
) -> i32 {
    let m = mref(m);
    let name = m.atoms.resolve(functor).to_string();
    m.error_site = site_id;
    crate::errors::existence_procedure(m, &name, arity);
    m.error_site = crate::machine::NO_SITE;
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    #[test]
    fn put_struct_consumes_breg() {
        let mut m = Machine::new(StringInterner::new(), Vec::new());
        let mp = &mut *m as *mut Machine;
        plg_rt_breg_set(mp, 0, cell::make_atom(1));
        plg_rt_breg_set(mp, 1, cell::make_int(7));
        let w = plg_rt_put_struct(mp, 9, 2);
        assert_eq!(cell::tag_of(w), cell::TAG_STR);
        let idx = cell::payload(w) as usize;
        assert_eq!(cell::unpack_functor(m.heap[idx]), (9, 2));
        assert_eq!(m.heap[idx + 1], cell::make_atom(1));
        assert_eq!(m.heap[idx + 2], cell::make_int(7));
    }

    #[test]
    fn frames_roundtrip() {
        let mut m = Machine::new(StringInterner::new(), Vec::new());
        let mp = &mut *m as *mut Machine;
        let f = plg_rt_frame_alloc(mp, 3);
        plg_rt_frame_set(mp, f, 2, 99);
        assert_eq!(plg_rt_frame_get(mp, f, 2), 99);
    }

    #[test]
    fn k_roundtrips_through_u64() {
        let mut m = Machine::new(StringInterner::new(), Vec::new());
        let mp = &mut *m as *mut Machine;
        let k = plg_rt_pred_fail as *const () as usize as u64;
        plg_rt_set_k(mp, k, 42);
        assert_eq!(plg_rt_k_fn(mp), k);
        assert_eq!(plg_rt_k_env(mp), 42);
    }
}
