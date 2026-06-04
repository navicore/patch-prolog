# Runtime ABI (ADR)

**Status: pinned (M2). Frozen by M3.**

The C-ABI surface between `plgc`-generated LLVM IR and
`libplg_runtime.a`. Rust side: `crates/runtime/src/abi.rs` + `entry.rs`;
IR side: the `RUNTIME_DECLS` block in
`crates/compiler/src/codegen/program.rs`. Those two files ARE the
contract; this document explains it. The build.rs version-sync
guarantees an embedded runtime always matches the compiler that emits
calls into it.

## The uniform function signature

Every compiled predicate, clause, chain ("try next clause"),
continuation, and retry function — and every runtime-provided
continuation — shares one C-ABI prototype:

```llvm
i32 (ptr %machine, i64 %env)
```

Returns `0` = fail/exhausted (the driver loop backtracks), `1` = stop
(solution limit reached / enumeration finished early). The uniform
prototype is what makes `musttail` transfers guaranteed-TCO under the C
calling convention (same trick protobuf's parser uses), and it lets
Rust functions (the solve driver, the capture continuation, `,`/2 at
query level) participate at chain boundaries via plain function
pointers.

Function pointers cross the boundary as `i64` (`ptrtoint`/`inttoptr` in
IR, `transmute` in Rust); frames and environments are heap **indices**,
never raw pointers — the heap is a growable `Vec<u64>` and indices
survive reallocation and backtracking truncation.

## Control flow contract

- Forward execution: generated code installs a continuation with
  `plg_rt_set_k` and `musttail`s into the callee's entry function.
  Solution delivery is a `musttail` into the installed k.
- Failure: `ret i32 0` unwinds the (constant-depth) tail-call chain to
  the runtime **driver loop** (`solve.rs`), which pops a choice point,
  rewinds trail+heap to its marks, and calls its retry function. The
  driver is the trampoline; the C stack never grows with recursion or
  backtracking depth.
- Choice points: `plg_rt_push_cp(m, retry_fn, env)` snapshots
  trail/heap marks at push time. Predicate entries push one CP per
  remaining clause group lazily (entry pushes t1; t1 pushes t2; …).
- Frames (predicate: `[args…, k_fn, k_env]`; clause body:
  `[k_fn, k_env, vars…]`) are allocated with `plg_rt_frame_alloc`
  **before** any CP that must see them — heap rewind frees everything
  allocated after the CP, so ordering is the correctness rule.

## Generated global data

```llvm
@plg_atom_strs    = [N x ptr]              ; atom names, id order
@plg_registry     = [K x { i32, i32, ptr }] ; functor, arity, entry fn
```

`plg_rt_init` re-interns the atom names in order, reproducing the
compiler's exact id space (both sides pre-seed the same well-known
atoms); query atoms unseen at compile time get fresh ids ≥ N and
correctly unify with nothing. The registry is sorted by (functor,
arity) for binary search; `:- dynamic` predicates with no clauses point
at `@plg_rt_pred_fail` (silent-fail contract).

## The plg_rt_* surface (M2)

| symbol | signature (IR) | purpose |
|---|---|---|
| `plg_rt_init` | `ptr (ptr, i32, ptr, i32)` | build Machine from atom table + registry |
| `plg_rt_main` | `i32 (ptr, i32, ptr)` | argv parse, query parse, solve, output, exit code |
| `plg_rt_step` | `i32 (ptr)` | step counter; 0 ⇒ uncatchable resource_error set |
| `plg_rt_new_var` | `i64 (ptr)` | fresh unbound REF cell |
| `plg_rt_frame_alloc` | `i64 (ptr, i32)` | raw continuation-frame cells |
| `plg_rt_frame_set/get` | `void/i64 (ptr, i64, i32[, i64])` | frame slot access |
| `plg_rt_areg_get/set` | `i64/void (ptr, i32[, i64])` | argument registers |
| `plg_rt_breg_set` | `void (ptr, i32, i64)` | build registers for put_struct |
| `plg_rt_put_struct` | `i64 (ptr, i32, i32)` | compound from breg[0..arity] |
| `plg_rt_put_list` | `i64 (ptr, i64, i64)` | cons cell |
| `plg_rt_put_float` | `i64 (ptr, i64)` | boxed f64 (bits) |
| `plg_rt_unify` | `i32 (ptr, i64, i64)` | generic trail-recording unify |
| `plg_rt_set_k` / `plg_rt_k_fn` / `plg_rt_k_env` | — | continuation register |
| `plg_rt_push_cp` | `void (ptr, i64, i64)` | choice point (retry fn + env) |
| `plg_rt_pred_fail` | `i32 (ptr, i64)` | always-fail body (dynamic stubs) |
| `plg_rt_existence_error` | `i32 (ptr, i32, i32)` | sets v1-format error, returns 0 |

Atoms and immediate integers never cross the ABI: codegen emits their
tagged words as compile-time constants (`(id<<3)|1`, `(n<<3)|2` —
mirrored in `cell.rs` and `term_emit.rs`, covered by tests on both
sides).

## Term words (tag in low 3 bits)

`REF=0` (heap idx, unbound = self-ref) · `ATOM=1` (id) · `INT=2`
(immediate i61; out-of-range literals are a compile error until boxed
i64 lands in M4) · `STR=3` (idx → `[functor:u32|arity:u32][args…]`) ·
`LST=4` (idx → `[head][tail]`) · `FLT=5` (idx → f64 bits, `to_bits`
equality).

## M3 additions (implemented)

| symbol | signature (IR) | purpose |
|---|---|---|
| `plg_rt_cp_top` | `i64 (ptr)` | choice-point height (cut barriers, commit slots) |
| `plg_rt_cut` | `void (ptr, i64)` | truncate cps to height (M4: stop at CATCH frames) |
| `plg_rt_deref` | `i64 (ptr, i64)` | first-arg indexing dispatch |
| `plg_rt_str_key` | `i64 (ptr, i64)` | STR → packed functor cell, else u64::MAX |
| `plg_rt_b_is` | `i32 (ptr, i64, i64)` | is/2 (v1 eval semantics, exact error strings) |
| `plg_rt_b_arith_cmp` | `i32 (ptr, i32, i64, i64)` | op 0..5 = `<` `>` `=<` `>=` `=:=` `=\=` |
| `plg_rt_b_neq` | `i32 (ptr, i64, i64)` | `\=` (trail-rewinding attempt) |
| `plg_rt_b_term_cmp` | `i32 (ptr, i32, i64, i64)` | op 0..5 = `==` `\==` `@<` `@>` `@=<` `@>=` |
| `plg_rt_b_compare` | `i32 (ptr, i64, i64, i64)` | compare/3 |

Op-code tables live in codegen `lower.rs` and runtime `control.rs` and
must stay in sync. Control constructs (`;`, `->`, `\+`, `once`) have NO
runtime symbols for compiled code — they compile to choice points and
commit slots (body.rs); the runtime's `control.rs` mirrors the same
shapes only for goals built at runtime (queries, M4 metacalls), walking
goal terms, never clauses.

## M4 additions (planned)

`plg_rt_call` registry metacall for `call/N` and `findall/3`; CATCH
choice-point frames for `catch/3`; remaining builtin families as
`plg_rt_b_*`. Errors become structured terms (today: preformatted
v1-compatible message strings in `Machine::error`).

## Environment

`PLG_MAX_STEPS` overrides the default 10,000 step ceiling (documented
extension over v1, which hardcoded it; the CLI contract is unchanged).
