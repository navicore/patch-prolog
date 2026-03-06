# ISO Prolog Compliance Decisions

Design decisions about ISO standard conformance, documented here so they aren't lost across feature work.

## Unification (ISO 8.3.2)

- `=/2` does **not** perform occurs check — `X = f(X)` succeeds, creating a circular term
- `apply()` uses cycle detection (visited-variable set) to safely resolve circular terms during solution extraction
- `unify_with_occurs_check/2` is not yet implemented; the `occurs_in` method is retained for future use

## Float Equality

- Unification uses `f64::to_bits()` for structural equality — NaN unifies with NaN
- This differs from arithmetic comparison where NaN comparisons return false

## Arithmetic

- `mod` uses ISO floored semantics: result has the sign of the divisor (via `rem_euclid`), not truncated remainder
- Integer overflow is a runtime error (not silent wraparound)
- Float NaN/Infinity after any arithmetic operation is a runtime error
- Division by zero (integer or float) is a runtime error

## Term Standard Order (ISO 8.4.2)

Variables < Numbers < Atoms < Compounds

- Float < Integer when arithmetically equal (e.g., `1.0 @< 1`)
- NaN sorts after all other floats (deterministic total order)

## Built-in Error Behavior

- `number_chars/2` and `number_codes/2` return syntax error for non-numeric strings (not silent failure)
- `number_chars/2` and `number_codes/2` reject NaN/Infinity parse results
- `=../2` with `T =.. [F]` where F is unbound returns instantiation error (ISO 8.5.3)
- `=../2` with empty list returns error (not failure)
- `functor/3` with negative arity returns error
- `atom_chars/2` is for atoms only — `number_chars/2` handles numbers

## What We Don't Implement

- `assert/1`, `retract/1` — knowledge base is immutable (compiled at build time)
- `call/N` for N > 1 — only `call/1` is supported
- `unify_with_occurs_check/2`
- `catch/3`, `throw/1` — no exception system
- Module system
- Definite clause grammars (DCG)
