# Builtin & Stdlib Reference

Every predicate the engine provides. The **builtins** are compiled into the
language itself; the **standard library** is a small set of list predicates
compiled into every binary. The descriptions here mirror the engine's
builtin table (`plg-shared::BUILTINS`) verbatim — a test fails the build if
this page and the table disagree.

Evaluable arithmetic *functions* (`+`, `*`, `mod`, `abs`, …) are not
predicates; see the [Language Guide](language-guide.md) and the
[Operators](OPERATORS.md) reference for those and for operator precedence.

## Type checks

| Predicate | Description |
|---|---|
| `var/1` | Type check: succeeds if argument is an unbound variable. |
| `nonvar/1` | Type check: succeeds if argument is bound. |
| `atom/1` | Type check: succeeds if argument is an atom. |
| `number/1` | Type check: succeeds if argument is an integer or float. |
| `integer/1` | Type check: succeeds if argument is an integer. |
| `float/1` | Type check: succeeds if argument is a float. |
| `compound/1` | Type check: succeeds if argument is a compound term. |
| `is_list/1` | Type check: succeeds if argument is a proper list. |

## Unification & term comparison

| Predicate | Description |
|---|---|
| `=/2` | Unification: `X = Y` succeeds if X and Y can be made identical. |
| `\=/2` | Not-unifiable: succeeds when `=` would fail. |
| `==/2` | Term identity: structural equality without unification. |
| `\==/2` | Term non-identity. |
| `unify_with_occurs_check/2` | Unification with occurs check: rejects `X = f(X)`-style cycles. |
| `compare/3` | `compare(Order, T1, T2)` — bind Order to <, =, or > per standard term ordering. |
| `@</2` | Standard term ordering: less. |
| `@>/2` | Standard term ordering: greater. |
| `@=</2` | Standard term ordering: less-or-equal. |
| `@>=/2` | Standard term ordering: greater-or-equal. |

## Arithmetic

| Predicate | Description |
|---|---|
| `is/2` | Arithmetic evaluation: `X is Expr` binds X to the value of Expr. |
| `=:=/2` | Arithmetic equality. |
| `=\=/2` | Arithmetic inequality. |
| `</2` | Arithmetic less-than. |
| `>/2` | Arithmetic greater-than. |
| `=</2` | Arithmetic less-or-equal (note: `=<`, not `<=`). |
| `>=/2` | Arithmetic greater-or-equal. |

## Control

| Predicate | Description |
|---|---|
| `,/2` | `(A, B)` — conjunction: prove A, then B. |
| `;/2` | `(A ; B)` — disjunction: prove A, or B on backtracking. `(C -> T ; E)` reads as if-then-else. |
| `->/2` | `(C -> T)` — if-then: if C succeeds (committing to its first solution), prove T; otherwise fail. |
| `\+/1` | Negation as failure: succeeds when its argument fails. |
| `once/1` | `once(Goal)` — succeed at most once for Goal. |
| `call/1` | Meta-call: execute its argument as a goal. Variadic — extra args are appended. |
| `true/0` | Always succeeds. |
| `fail/0` | Always fails. |
| `false/0` | Always fails (alias for `fail`). |
| `!/0` | Cut: commit to current choices; remove choice points back to the parent clause. |

## Finding solutions & enumeration

| Predicate | Description |
|---|---|
| `findall/3` | `findall(Template, Goal, List)` — collect all solutions of Goal. |
| `between/3` | `between(Low, High, X)` — enumerate or test integers in [Low, High]. |

## Exceptions

| Predicate | Description |
|---|---|
| `catch/3` | `catch(Goal, Catcher, Recovery)` — run Goal; on thrown error matching Catcher, run Recovery. |
| `throw/1` | Raise an error term that propagates to the nearest matching `catch/3`. |

## Term construction & inspection

| Predicate | Description |
|---|---|
| `functor/3` | `functor(Term, Name, Arity)` — inspect or construct a term's functor. |
| `arg/3` | `arg(N, Term, Arg)` — extract the N-th argument of Term. |
| `=../2` | Univ: `T =.. L` decomposes T into a list of its functor and args. |
| `copy_term/2` | `copy_term(T, C)` — bind C to a copy of T with fresh variables. |

## Atoms & numbers

| Predicate | Description |
|---|---|
| `atom_length/2` | `atom_length(A, L)` — bind L to the length of atom A. |
| `atom_concat/3` | `atom_concat(A, B, C)` — concatenate A and B into C, or, with C bound and A/B unbound, nondeterministically split C into every prefix/suffix pair. |
| `atom_chars/2` | `atom_chars(A, Chars)` — convert between an atom and a list of single-char atoms. |
| `number_chars/2` | `number_chars(N, Chars)` — convert between a number and a list of single-char atoms. |
| `number_codes/2` | `number_codes(N, Codes)` — convert between a number and a list of character codes. |

## Arithmetic relations

| Predicate | Description |
|---|---|
| `succ/2` | `succ(X, S)` — Peano successor relation; S = X + 1, both non-negative. |
| `plus/3` | `plus(X, Y, Z)` — addition relation; any one argument may be unbound. |

## Sorting

| Predicate | Description |
|---|---|
| `msort/2` | `msort(L, Sorted)` — sort without removing duplicates. |
| `sort/2` | `sort(L, Sorted)` — sort and remove duplicates. |

## I/O

| Predicate | Description |
|---|---|
| `write/1` | Write a term to stdout (no newline). |
| `writeq/1` | Write a term to stdout, quoting atoms so it reads back (no newline). |
| `writeln/1` | Write a term to stdout followed by a newline. |
| `nl/0` | Write a newline to stdout. |

## Standard library (lists)

Defined in Prolog (`stdlib.pl`) and compiled into every binary.

| Predicate | Description |
|---|---|
| `member/2` | `member(X, List)` — succeeds once for each element of List. |
| `append/3` | `append(A, B, C)` — C is A concatenated with B (relational; works in reverse). |
| `length/2` | `length(List, N)` — N is the number of elements in List. |
| `last/2` | `last(List, X)` — X is the last element of List. |
| `reverse/2` | `reverse(List, Rev)` — Rev is List in reverse order. |
| `nth0/3` | `nth0(N, List, X)` — X is the element at zero-based index N. |
| `nth1/3` | `nth1(N, List, X)` — X is the element at one-based index N. |
