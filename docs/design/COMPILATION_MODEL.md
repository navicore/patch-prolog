# Compilation Model (ADR)

**Status: accepted.**

How Prolog's nondeterministic control compiles to LLVM IR. The chosen
design is the simplest one that is *genuinely compiled* — native clause
dispatch, native sequencing, native tail recursion — while reusing v1's
exact runtime semantics as a library.

## Decision

**Continuation-passing style (CPS) + an explicit runtime-managed
choice-point stack + `musttail call tailcc` for every control transfer.**

Alternatives considered:

- **WAM-instruction emission** (GNU Prolog's path): fastest and most
  faithful, but requires the full WAM register/environment design up
  front. Named escape hatch for later optimization, not v1.
- **Pure choice-point trampoline** (success also routed through the
  stack): simple, but forfeits the musttail win on determinate
  iteration — a bounce per goal.
- **Interpretation with embedded data**: rejected permanently
  (LESSONS_FROM_V1.md).

## Predicate ABI

```llvm
; one function per predicate
define tailcc i1 @plg_pred__ancestor_2(ptr %M, i64 %A0, i64 %A1,
                                       ptr %K, ptr %Kenv)
```

- `%M` — the Machine: heap, trail, choice-point stack, var counter,
  step counter/limit, atom table. The single context pointer.
- `%A0..%An` — arguments as tagged words (see Term representation).
- `%K`/`%Kenv` — success continuation + its heap-allocated environment
  (the compiled "rest of the conjunction" and the caller's K).
- Deliver a solution: `musttail call tailcc i1 %K(ptr %M, ptr %Kenv)`.
  If K returns `1` ("stop"), propagate `1`. If `0` ("more"), try the
  next alternative.
- Exhausted: `ret i1 0`.

`tailcc` guarantees tail-call optimization between functions of
differing signatures (same return type) — that is the convention's
purpose. clang ≥ 15 is enforced at link time. The generated `main` uses
the C convention and enters the tailcc world through one regular call
(same boundary rule as patch-seq). **Fallback** if a target ever fights
musttail: uniform signature with WAM-style argument registers in the
Machine.

## Choice points and backtracking

A choice-point entry stores:

```
{ trail_mark, heap_top, var_top, cut_parent, retry_fn, retry_env, flags }
```

`retry_fn` is the compiled "try the next clause / branch" function;
`retry_env` captures the original arguments and continuation. On
backtrack the runtime pops the entry, rewinds trail (undo bindings),
resets heap top (**backtrack-deallocation — no GC**), restores the var
counter, and `musttail`s `retry_fn`. `flags` marks CATCH and disjunction
frames.

## Cut

Barrier = choice-point stack height at predicate entry (`B0`). `!`
compiles to a runtime call truncating the stack to `B0`, **stopping at
CATCH frames** (cut is opaque to catch — v1 solver rule). If-then-else
and `once/1` are local barriers around the condition; `\+ G` is
`(G -> fail ; true)`.

## catch/throw

No native unwinding exists to fight: continuations are heap frames and
transfers are tail calls, so `throw/1` is pure data — walk the
choice-point stack for the topmost CATCH frame whose catcher unifies
with the ball, truncate, rewind trail/heap, tail-call the recovery
continuation. The uncatchable flag (step limit) skips all catch frames
and exits with code 3, exactly as v1.

## Term representation

Tagged 64-bit words; 3 low tag bits:

| tag | meaning | payload |
|---|---|---|
| `REF` | variable cell | pointer to heap cell (unbound = self-ref) |
| `ATOM` | atom | `AtomId << 3` |
| `INT` | small integer | immediate i61; values outside i61 box to a heap i64 cell |
| `STR` | compound | pointer to `[functor:u32 \| arity:u32][arg0]...` |
| `LST` | cons | pointer to `[head][tail]` |
| `FLT` | float | pointer to boxed f64 (`to_bits` equality — NaN unifies with NaN) |

Arithmetic computes in checked i64 regardless of representation;
overflow raises `evaluation_error(int_overflow)` (ISO_COMPLIANCE.md).

## What is compiled vs called

Compiled inline in v1:
- clause dispatch: `switch` on `%A0` tag/value (first-argument indexing,
  same keys as v1 `first_arg_key()`); var first-arg ⇒ try all clauses
- head unification for shallow patterns (atom/int/var/one-level struct)
- tag-test builtins (`var/1`, `atom/1`, `integer/1`, …)
- arithmetic evaluation and comparison (checked ops + branch-to-error)
- cut, conjunction sequencing, disjunction branching

Runtime calls in v1 (inline later if profiles demand):
- `plg_rt_unify` — generic unification fallback for deep heads. Shares
  the same bind/deref/trail primitives as compiled head-unify so the
  two cannot diverge.
- `plg_rt_put_*` term construction helpers
- the builtin library (lists, atoms, `functor/3`, `=..`, `sort`,
  `findall`, I/O, …) — direct ports of v1 `builtins.rs`
- `plg_rt_call` — metacall dispatch via the registry

## Worked example

```prolog
ancestor(X, Z) :- parent(X, Z).
ancestor(X, Z) :- parent(X, Y), ancestor(Y, Z).
```

- `@plg_pred__ancestor_2` pushes one choice point (retry = clause 2's
  compiled body) and tail-calls `@plg_pred__parent_2` with the caller's
  own `%K` (clause 1's body goal is in tail position).
- Clause 2's retry function allocates fresh `Y`, builds a continuation
  frame for `ancestor(Y, Z)`, and tail-calls `parent(X, Y)` with it.
- The continuation tail-calls `@plg_pred__ancestor_2(Y, Z, K, Kenv)` —
  the recursive call is a true jump: a million-deep ancestor chain runs
  in constant C stack.
- When everything is exhausted the outermost call returns `0`; the
  runtime driver reports count/exit code.

## Risks and guards

| risk | guard |
|---|---|
| musttail emission bug → stack growth | codegen post-check: every `musttail` is followed by a bare `ret`; deep-recursion integration test |
| compiled vs generic unify divergence | shared primitives + differential tests vs the v1 interpreter oracle |
| atom-id divergence compile↔runtime | single emitted table; round-trip unit tests |
| runaway heap in determinate queries | uncatchable step limit (v1 semantics); copying GC is a future escape hatch |
