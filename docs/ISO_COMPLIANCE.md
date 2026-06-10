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

## Cut (divergence from patch-prolog v1)

- `!` is **transparent in `;`, `->` and `,`** per ISO 7.8.4: a cut
  inside a disjunction branch cuts the whole clause, including the
  disjunction's other branch and the predicate's remaining clauses.
  `t(X) :- (m(X), X > 1, ! ; X = fallback).` with `m(1). m(2). m(3).`
  yields exactly `X = 2`.
- **v1 diverged here** (undocumented): it treated the cut as local to
  the disjunction branch and also produced `X = fallback`. patch-prolog2
  deliberately follows ISO (and SWI/GNU Prolog) instead of reproducing
  the v1 behavior; the differential test corpus whitelists this case.
- Cut remains **opaque** inside `\+/1`, `once/1`, the condition of
  `->` (local commit), and `catch/3` per ISO.

## Dynamic Predicates

- `:- dynamic(F/A).` directive declares a predicate as having clauses populated at runtime/build time externally. An undefined dynamic predicate fails silently; an undefined non-dynamic predicate raises `existence_error(procedure, F/A)` per ISO 7.7.3.
- This preserves the linter contract ("missing data = compliant") for predicates the user explicitly declares dynamic.

## Undefined-predicate lint (ISO-preserving, opt-in strictness)

- A direct body goal that calls a predicate defined nowhere (no clauses, no `:- dynamic`, not a builtin/stdlib) compiles and raises a **catchable** `existence_error` *when reached* — required so `catch(foo(X), error(existence_error(procedure,_),_), R)` works and so `unknown=error` stays a runtime condition (ISO 7.7.3, 7.8.9). The program is well-formed: the call may be caught or never reached. `compile`-and-`run` of such a program is verified by `undefined_in_rule_body_raises_when_reached`.
- Because patch-prolog is a whole-program compiler with no `assert`/`retract`, such a *direct* call can never succeed, so `plgc check`/`build`/`run` emit a **warning** (and `plgl` shows a warning squiggle) — the editor/CLI lint mature Prolog systems also provide (cf. SWI `list_undefined`). This does not change what compiles or how it runs.
- `--deny-undefined` promotes the warning to a hard error (no binary, non-zero exit) for callers that prioritize correctness over leniency. This is opt-in strictness layered *above* the ISO-compliant default, not a divergence from it. Runtime-built goals (`call/N`, variable goals) are never flagged — only statically-resolvable direct calls.

## What We Don't Implement

- `assert/1`, `retract/1` — knowledge base is immutable (compiled at build time); `:- dynamic` only enables silent-fail, not runtime mutation
- `op/3` — operator table is fixed; see [OPERATORS.md](OPERATORS.md)
- Module system
- Definite clause grammars (DCG)
