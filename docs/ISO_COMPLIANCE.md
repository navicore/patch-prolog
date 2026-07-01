# Semantics & ISO Conformance

patch-prolog implements an ISO subset. This page is the precise statement
of how it behaves where the standard leaves room for choices, the few
deliberate divergences, and the safety guarantees layered on top. The
[Language Guide](language-guide.md) teaches these with examples; this is the
reference, with ISO section numbers.

## Unification (ISO 8.3.2)

- `=/2` does **not** perform the occurs check — `X = f(X)` succeeds,
  creating a cyclic term, which is the ISO default. Solution display
  handles cycles safely.
- `unify_with_occurs_check/2` is the safe variant: it fails on
  `X = f(X)`-style cycles instead of building them.

## Float equality

- Unification treats floats by bit pattern, so **`NaN` unifies with
  `NaN`**.
- This differs from arithmetic comparison, where any comparison involving
  `NaN` is false (`=:=`, `<`, etc.).

## Arithmetic

- `mod` uses ISO **floored** semantics — the result takes the sign of the
  divisor (`-7 mod 3` is `2`), not the truncated remainder. (`rem` takes
  the sign of the dividend.)
- **Integer overflow is a runtime error** (`evaluation_error(int_overflow)`),
  never a silent wraparound.
- A `NaN` or infinite float result from any operation is a runtime error.
- Division by zero (integer or float) is a runtime error.

## Standard order of terms (ISO 8.4.2)

    Variables < Numbers < Atoms < Compounds

- A float sorts just before an integer of equal value (`1.0 @< 1`).
- `NaN` sorts after all other floats (a deterministic total order).

## Built-in error behavior

- `number_chars/2` and `number_codes/2` raise a **syntax error** on a
  non-numeric string (not silent failure), and reject `NaN`/infinity.
- `T =.. [F]` with `F` unbound raises an instantiation error (ISO 8.5.3);
  an empty list is an error, not a failure.
- `functor/3` with a negative arity is an error.
- `atom_chars/2` is for atoms only; `number_chars/2` handles numbers.
- When `is/2` (or an arithmetic comparison) meets a non-evaluable functor,
  the `type_error(evaluable, Culprit)` culprit is a **predicate indicator**
  `Name/Arity` (ISO 8.6) — so a bare atom `foo` reports `foo/0`, exactly
  like the compound case `foo(1)` reports `foo/1`. (The original v1
  interpreter reported the bare atom for the arity-0 case.)

## Exceptions

- `catch/3` and `throw/1` (ISO 7.8.9, 7.8.10) use the ISO error taxonomy:
  `error(Formal, Context)`, where `Formal` is one of `instantiation_error`,
  `type_error/2`, `existence_error/2`, `domain_error/2`,
  `evaluation_error/1`, `permission_error/3`, `representation_error/1`,
  `resource_error/1`, or `syntax_error`.
- `throw/1` of an unbound variable raises `instantiation_error`.
- `catch/3` is opaque to cut: a `!` inside the goal cannot escape the catch
  frame.

## Cut (ISO 7.8.4)

`!` is **transparent through `,`, `;`, and `->`**: a cut inside a
disjunction branch cuts the whole clause, including the other branch and
the predicate's remaining clauses (the standard WAM/ISO cut-barrier rules,
implemented here as choice-point-stack truncation to the barrier recorded
at predicate entry). With `m(1). m(2). m(3).`:

```prolog
t(X) :- (m(X), X > 1, ! ; X = fallback).
```

`?- t(X).` yields exactly `X = 2`. Cut remains **opaque** inside `\+/1`,
`once/1`, the condition of `->`, and `catch/3`.

> If you used the original patch-prolog *interpreter*, note this is a change:
> it treated the cut as local to the disjunction branch (and also produced
> `X = fallback`). patch-prolog follows ISO here, matching SWI and GNU
> Prolog.

## Dynamic and undefined predicates (ISO 7.7.3)

- `:- dynamic(F/A).` declares a predicate whose clauses may be absent. An
  **undefined dynamic** predicate **fails silently**; an **undefined
  non-dynamic** predicate raises `existence_error(procedure, F/A)`.
- A direct call to a predicate defined nowhere is still a *well-formed
  program*: it compiles and raises that (catchable) `existence_error` if
  reached. Because patch-prolog is a whole-program compiler, such a call can
  never succeed, so `plgc build`/`run`/`check` **warn** about it and `plgl`
  shows a squiggle. `--deny-undefined` promotes the warning to a hard error
  — opt-in strictness above the ISO-compliant default, not a divergence from
  it. See [Compiler Usage](compiler-usage.md#undefined-predicates).

## Safety guarantees (beyond ISO)

- A runaway computation hits a **step limit** and stops with
  `resource_error(steps)`. This error is deliberately **uncatchable**, so a
  buggy clause cannot trap its own timeout. This is a safety guarantee, not
  an ISO requirement.

## Not implemented

These are deliberate omissions of the subset:

- `assert/1`, `retract/1` — the knowledge base is immutable (compiled at
  build time); `:- dynamic` only enables silent-fail, not runtime mutation.
- `op/3` — the operator table is fixed; see [Operators](OPERATORS.md).
- Module system.
- Definite clause grammars (DCG).
