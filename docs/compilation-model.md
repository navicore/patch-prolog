# Compilation Model

How Prolog's nondeterministic control becomes native code. This sits between
the [Architecture](ARCHITECTURE.md) overview and the [Runtime ABI](RUNTIME_ABI.md)
symbol-level contract: it explains the *shape* of the generated code — the
function signatures and exact symbols are in the Runtime ABI.

## The shape: CPS + a choice-point trampoline

Each predicate compiles to one LLVM function with a **uniform signature**,
`i32 (ptr %m, i64 %env)` (returns `0` = fail/exhausted, `1` = stop). Forward
execution is continuation-passing: a goal installs the "rest of the
conjunction" as a continuation and `musttail`s into the callee; a solution is
delivered by `musttail`ing that continuation.

Because every function shares the one signature, `musttail` is guaranteed
tail-call optimization under the ordinary C calling convention — so recursion
and backtracking never grow the C stack.

This is the simplest design that is *genuinely compiled*. The alternatives:

- **WAM-instruction emission** (the GNU Prolog / SICStus path — lowering
  Prolog to Warren's instruction set: `get`/`put`/`unify` head-unification
  triples over argument registers, `call`/`execute`/`proceed` control) —
  fastest and most faithful to the standard implementation strategy, but
  needs the full WAM register/environment design up front. A named future
  escape hatch, not the current target. (plgc keeps the WAM *machine model* —
  heap, trail, choice points, tagged words, indexing — and replaces only
  the *instruction set* with direct native CPS codegen; see
  [Runtime ABI](RUNTIME_ABI.md#lineage).)
- **A pure choice-point trampoline** (success also routed through the stack)
  — simple, but bounces once per goal, forfeiting the `musttail` win on
  determinate iteration.
- **Interpretation with embedded data** — rejected permanently; that was the
  original patch-prolog's mistake (a compiler in costume over an
  interpreter).

## Choice points and backtracking

Alternatives — untried clauses, disjunction branches — become entries on a
runtime **choice-point stack**, each snapshotting the trail, heap top, and
variable counter, plus a *retry function*. Failure is `ret i32 0`: it unwinds
the constant-depth tail-call chain to the **driver loop** (`solve.rs`), which
pops a choice point, rewinds the trail (undo bindings) and the heap top
(**backtrack-deallocation — memory reclaimed with no GC**), and `musttail`s
the retry. The driver is the trampoline; depth never grows the C stack.

Clause groups push lazily: a predicate's entry tries clause 1 and pushes one
choice point whose retry is clause 2, which pushes clause 3, and so on.
First-argument indexing compiles to a `switch` on the first argument's
tag/value (the WAM first-argument keys); a single keyed candidate
pushes **no** choice point.

## Cut

The barrier is the choice-point stack height recorded at predicate entry.
`!` compiles to a runtime call that truncates the stack back to that height,
**stopping at catch frames** (cut is opaque to `catch`). If-then-else and
`once/1` are local barriers around the condition; `\+ G` is `(G -> fail ;
true)`.

## catch / throw

There is no native stack unwinding to fight: continuations are heap frames
and transfers are tail calls, so `throw/1` is pure data. The runtime walks
the choice-point stack for the topmost catch frame whose catcher unifies with
the thrown ball, rewinds the trail and heap, and tail-calls the recovery
continuation. The step-limit ball is **uncatchable** — it skips every catch
frame and exits with code 3.

## What is compiled vs. called

**Compiled inline**, in the clause code itself:

- clause dispatch (the first-argument-indexing `switch`),
- conjunction sequencing, disjunction branching, and the if-then-else /
  `once` / `\+` commit machinery (choice points + commit-height slots),
- cut,
- atom and immediate-integer constants (their tagged words are baked into the
  IR).

**Runtime calls** — the pragmatic scope line; inlining any of these is a
named escape hatch if profiles ever demand it (symbols in the
[Runtime ABI](RUNTIME_ABI.md)):

- generic unification, including head unification — one shared
  bind/deref/trail implementation so compiled and runtime paths can't
  diverge,
- `is/2` and arithmetic comparison (the evaluator, with its exact error
  strings),
- the deterministic builtin library, plus `findall/3`, `catch/3`, `throw/1`,
  the metacall path, and `between/3`.

## Worked example

```prolog
ancestor(X, Z) :- parent(X, Z).
ancestor(X, Z) :- parent(X, Y), ancestor(Y, Z).
```

- `ancestor/2`'s entry pushes one choice point (retry = clause 2's body) and
  `musttail`s into `parent/2` with the caller's continuation — clause 1's
  body goal is in tail position.
- Clause 2's retry allocates a fresh `Y`, builds a continuation frame for
  `ancestor(Y, Z)`, and `musttail`s `parent(X, Y)` with it.
- That continuation `musttail`s `ancestor/2` again — the recursive call is a
  true jump, so a million-deep `ancestor` chain runs in constant C stack.
- When everything is exhausted the outermost call returns `0`; the driver
  reports the solution count and exit code.

## Guarantees and guards

| risk | guard |
|---|---|
| `musttail` emission bug → stack growth | codegen post-check (every `musttail` is followed by a bare `ret`) + a deep-recursion integration test |
| compiled vs. generic unify divergence | shared primitives + property tests |
| atom-id divergence compile ↔ runtime | a single emitted atom table + round-trip unit tests |
| runaway heap in determinate queries | the uncatchable step limit (copying GC is a future escape hatch) |
