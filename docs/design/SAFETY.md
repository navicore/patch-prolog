# Safety Design

patch-prolog is designed to run untrusted or semi-trusted Prolog rules against AI output. The engine must not panic, hang, or consume unbounded resources.

## Step Limit

- Default: 10,000 steps. Configurable via `Solver::with_max_depth()`.
- Enforced in all three solver paths (`solve`, `try_solve_once`, `try_solve_collecting`).
- `between/3` iteration increments the step counter per value (not just at entry).
- `findall/3` steps accumulate globally — no independent budget per findall.
- When the step limit fires inside `try_solve_once`, the `steps_exceeded` flag is set so callers don't misinterpret timeout as failure (critical for NAF correctness).

## Integer Overflow

All integer arithmetic uses Rust's `checked_*` methods:
- `checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_neg`, `checked_abs`
- Overflow returns `SolveResult::Error`, not silent wraparound
- `succ/2` and `plus/3` use `checked_add`/`checked_sub`
- `between/3` uses `checked_add` for the `low + 1` step
- `arith_mod` guards against `i64::MIN` divisor (where `.abs()` would overflow)

## Float Safety

- `check_float()` validates every arithmetic float result — NaN and Infinity are runtime errors
- `number_chars/2` and `number_codes/2` reject NaN/Infinity from `parse::<f64>()` results
- `format_float()` guards against NaN/Infinity before appending ".0" suffix
- Float unification uses `to_bits()` for structural equality (NaN == NaN)

## Circular Term Protection

- `=/2` does not perform occurs check (ISO compliance), so `X = f(X)` creates circular terms
- `apply()` (deep variable resolution) uses a visited-variable set to detect and break cycles
- `copy_term_impl` uses iterative list spine traversal to avoid stack overflow on long lists
- `is_proper_list` and `term_compare` list-vs-list use iterative loops (not recursion)

## What Is Not Guarded

- **Rust stack overflow in `try_solve_collecting`**: The function is stack-recursive. Deep goal chains inside `findall` can exhaust the Rust stack before the step limit fires. Converting to iterative is a future refactor.
- **`try_exec_misc` error propagation**: Built-in errors silently become failures in `once/1`, `\+`, and `findall/3` contexts. This is an architectural limitation of the `Option<bool>` return type.
