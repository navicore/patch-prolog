# Solver Design

The solver is the largest module (~2,750 lines) and the heart of the engine. This document records the key design decisions and why they were made.

## Three Solver Paths

The solver has three resolution functions instead of one. This was a deliberate trade-off:

| Function | Returns | Used by | Can error? |
|----------|---------|---------|------------|
| `solve()` | `SolveResult` | Main query loop | Yes |
| `try_solve_once()` | `bool` | `\+`, `once/1`, if-then conditions | No (uses flags) |
| `try_solve_collecting()` | `bool` | `findall/3` | No (uses flags) |

**Why not one function?** `solve()` manages the choice point stack and returns solutions to the caller incrementally. `try_solve_once` and `try_solve_collecting` need to run within a single `solve()` step — they can't yield back to the caller. They also need different semantics: `try_solve_once` stops at the first success; `try_solve_collecting` gathers all successes.

**The `try_exec_misc` limitation**: Both `try_solve_once` and `try_solve_collecting` delegate common built-ins to `try_exec_misc`, which returns `Option<bool>`. This means it cannot propagate errors — predicates that throw errors in `solve()` silently fail in these contexts. This is the most significant known architectural limitation. Fixing it requires either changing the return type (ripple through all callers) or adding an error flag similar to `steps_exceeded`.

## Communication Flags

Because `try_solve_once` returns `bool`, it communicates side-channel information through flags on the `Solver` struct:

- **`cut_in_try_solve`**: Set when `!` fires inside `try_solve_once` or `try_solve_collecting`. Clause iteration loops check this flag to stop trying alternatives. Saved/restored around each clause attempt.

- **`steps_exceeded`**: Set when the step limit fires inside `try_solve_once` or `try_solve_collecting`. Callers in `solve()` check this to distinguish "goal failed" from "goal timed out" — critical for NAF, where timeout must not be treated as success.

## Goal List

Goals use `VecDeque<Term>` for O(1) `push_front`/`pop_front`. Early versions used `Vec<Term>` with `remove(0)`, which was O(n) per goal step.

## Variable Counter

`var_counter` is initialized to `max(query_var_ids, goal_var_ids) + 1`, not a magic constant. Each clause rename allocates fresh variable IDs starting from the current counter. The counter is saved/restored on backtracking via `ChoicePoint`.

## Cut Semantics

Cut removes choice points up to **and including** the nearest `cut_barrier`. An earlier implementation stopped AT the barrier without removing it, which left the barrier's alternatives available on subsequent backtracking.

In `try_solve_once`/`try_solve_collecting`, cut can't manipulate the choice stack (it's the main solver's stack). Instead, the `cut_in_try_solve` flag is set, and clause iteration loops check it.

## Step Limit

The step counter is global — shared across `solve()`, `try_solve_once()`, and `try_solve_collecting()`. `findall/3` does not get an independent step budget. This prevents adversarial programs from using nested findalls to multiply their effective compute.

The default limit is 10,000 steps, configurable via `with_max_depth()`. "Steps" counts goal resolution attempts, not Rust stack frames — deep recursion inside `try_solve_collecting` can still overflow the Rust stack before the step limit fires.

## between/3

`between/3` has three separate implementations because each solver path handles enumeration differently:

- **`solve()`**: Pushes a choice point for `between(low+1, high, X)` — uses normal backtracking. Has an O(1) fast path when X is already bound.
- **`try_solve_once()`**: Iterative for-loop with per-iteration step counting. O(1) fast path for bound X.
- **`try_solve_collecting()`**: Same iterative approach as `try_solve_once`.

This duplication exists because `try_exec_misc` can't handle between's backtracking semantics.
