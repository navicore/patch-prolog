# ISO Prolog Compliance Decisions

Design decisions about ISO standard conformance, documented here so they aren't lost across feature work.

## Unification (ISO 8.3.2)

- `=/2` does **not** perform occurs check — `X = f(X)` succeeds, creating a circular term
- `apply()` uses cycle detection (visited-variable set) to safely resolve circular terms during solution extraction
- `unify_with_occurs_check/2` IS now implemented (built-in dispatches to `subst.unify_with_occurs_check`); the `occurs_in` method underlies it.

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

## Exception Handling

- `catch/3` and `throw/1` are implemented (ISO 7.8.9, 7.8.10) with the ISO error-term taxonomy: `error(Formal, Context)` where `Formal` is one of `instantiation_error`, `type_error/2`, `existence_error/2`, `domain_error/2`, `evaluation_error/1`, `permission_error/3`, `representation_error/1`, `resource_error/1`, `syntax_error`.
- `throw/1` of an unbound variable raises `instantiation_error` per ISO.
- Step limit raises `resource_error(steps)` — intentionally **uncatchable** so a rule can't trap its own timeout. This is a safety guarantee, not an ISO requirement.
- `catch/3` is opaque to cut: `!` inside catch's goal cannot escape past the catch frame.

## Dynamic Predicates

- `:- dynamic(F/A).` directive declares a predicate as having clauses populated at runtime/build time externally. An undefined dynamic predicate fails silently; an undefined non-dynamic predicate raises `existence_error(procedure, F/A)` per ISO 7.7.3.
- This preserves the linter contract ("missing data = compliant") for predicates the user explicitly declares dynamic.

## What We Don't Implement

- `assert/1`, `retract/1` — knowledge base is immutable (compiled at build time); `:- dynamic` only enables silent-fail, not runtime mutation
- `op/3` — operator table is fixed; see [OPERATORS.md](OPERATORS.md)
- Module system
- Definite clause grammars (DCG)
