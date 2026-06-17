# Language Guide

patch-prolog implements an **ISO subset of Prolog**. A program is a set of
*facts* and *rules*; you pose *queries* and the engine searches for proofs,
binding variables along the way. This guide teaches the language as this
engine implements it — including the places where it makes a deliberate,
documented choice. For the complete operator table see the **Grammar &
Operators** reference; for every builtin and stdlib predicate, the **Builtin
& Stdlib Reference**.

One thing to keep in mind throughout: this is a *compiler*. Your whole
program is known at build time, so there is no `assert`/`retract` — the
knowledge base is immutable. You change a program by editing its source and
recompiling (the REPL's `:edit` automates that loop).

## Terms

Everything is a **term**. There are only a few kinds:

| Kind | Examples |
|---|---|
| **Atom** | `tom`, `[]`, `'a quoted atom'`, `+` |
| **Number** | `42`, `-7`, `3.14` |
| **Variable** | `X`, `Parent`, `_`, `_Rest` |
| **Compound** | `parent(tom, bob)`, `point(1, 2)`, `-(3)` |

A **variable** starts with an uppercase letter or `_`; everything else
starting lowercase (or quoted) is an atom. This is the single most common
beginner trip-up: `Parent` is a variable, `parent` is a predicate name.

A **list** is sugar for nested compounds: `[a, b, c]` is `'.'(a, '.'(b,
'.'(c, [])))`, and `[]` is the empty-list atom. The `[H|T]` notation splits
a list into its head and tail.

Operators are sugar too: `2 + 3` is the compound `+(2, 3)`, `H = [X|T]` is
`=(H, '.'(X, T))`. Unification and resolution always see the compound form.

## Facts, rules, and clauses

A **fact** asserts something true:

```prolog
parent(tom, bob).
parent(bob, ann).
```

A **rule** says a head is true *if* its body is:

```prolog
ancestor(X, Y) :- parent(X, Y).
ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).
```

Read `:-` as "if" and `,` as "and." A predicate is identified by name and
arity (`ancestor/2`), and its clauses are tried top to bottom. Multiple
clauses are *alternatives*.

A **directive** runs at load/build time:

```prolog
:- dynamic(extra/1).
```

## Queries and the search

A query asks the engine to prove a goal:

```sh
./family --query "ancestor(tom, X)"
```

The engine searches for variable bindings that make the goal true, yielding
solutions one at a time. With `parent/2` and `ancestor/2` above,
`ancestor(tom, X)` yields `X = bob` then `X = ann`. If a goal can be proved
more than one way, **backtracking** explores each alternative; if it can't
be proved at all, the query fails (no solutions).

## Unification

`=/2` makes two terms equal by binding variables:

```prolog
?- point(X, 2) = point(1, Y).
% X = 1, Y = 2
```

> **No occurs check (by design).** `X = f(X)` *succeeds*, producing a
> cyclic term — matching ISO's default. Use `unify_with_occurs_check/2`
> when you need the safe version that fails instead.

Related: `\=/2` (not unifiable), and `==/2` / `\==/2`, which test
*structural identity without binding* — `X == Y` is false for two distinct
unbound variables, where `X = Y` would succeed.

## Backtracking

When a goal has several solutions, the engine produces them lazily:

```prolog
likes(sam, pizza).
likes(sam, sushi).
likes(ann, pizza).
```

```
?- likes(sam, Food).
Food = pizza ;
Food = sushi .
```

Composed goals backtrack jointly — `likes(P, pizza), likes(P, sushi)`
searches for a `P` satisfying both.

## Control constructs

| Form | Meaning |
|---|---|
| `A, B` | Conjunction — prove `A`, then `B`. |
| `(A ; B)` | Disjunction — `A`, or `B` on backtracking. |
| `(Cond -> Then ; Else)` | If-then-else — if `Cond` succeeds, commit and prove `Then`, else `Else`. |
| `\+ Goal` | Negation as failure — succeeds iff `Goal` cannot be proved. |
| `once(Goal)` | The first solution of `Goal` only. |
| `call(Goal)` / `call(G, Extra...)` | Call a goal built at runtime. |

The **cut** `!` commits to the choices made so far in the current clause,
discarding remaining alternatives. patch-prolog follows ISO: the cut is
**transparent** through `,`, `;`, and `->`. So with `m(1). m(2). m(3).`:

```prolog
cut_demo(X) :- (m(X), X > 1, ! ; X = fallback).
```

`?- cut_demo(X).` yields exactly `X = 2` — the cut prunes both the rest of
`m/1` *and* the `; X = fallback` branch. (The cut is *opaque* inside
`\+`, `once`, the condition of `->`, and `catch/3`.)

## Arithmetic

Arithmetic is explicit: `is/2` evaluates its right side, and the comparison
operators evaluate both sides.

```prolog
?- X is 2 + 3 * 4.        % X = 14
?- 6 =:= 2 * 3.           % true   (=:= compares values; = would unify terms)
```

Division has several forms, and the defaults follow ISO — worth memorizing:

| Op | Result | Example |
|---|---|---|
| `/` | **always a float** (ISO 9.1.4) | `10 / 3` → `3.333…` |
| `//` | integer, truncates toward zero | `-7 // 2` → `-3` |
| `div` | integer, floors toward −∞ | `-7 div 2` → `-4` |
| `mod` | remainder, sign of **divisor** | `-7 mod 3` → `2` |
| `rem` | remainder, sign of **dividend** | `-7 rem 3` → `-1` |

Other evaluable functions include `-` `abs/1` `min/2` `max/2` `sign/1`, the
bitwise operators (`/\ \/ xor << >> \`), and powers (`**` float, `^`
integer-or-float). Use `=:= =\= < > =< >=` to compare.

> **Arithmetic is safe, not silent.** Integer overflow raises
> `evaluation_error(int_overflow)` — it never wraps around. A `NaN`/infinite
> float result, or division by zero, is likewise a runtime error.

## Lists

Lists use `[H|T]` to split head from tail, and the stdlib (compiled into
every binary) provides the staples:

```prolog
?- append([1,2], [3,4], L).      % L = [1,2,3,4]
?- member(X, [a,b,c]).           % X = a ; X = b ; X = c
?- length([a,b,c], N).           % N = 3
```

Also available: `reverse/2`, `last/2`, `nth0/3`, `nth1/3`. To **collect**
every solution of a goal into a list, use `findall/3`; to **enumerate** a
range of integers, `between/3`:

```prolog
?- findall(C, ancestor(tom, C), Kids).   % Kids = [bob, ann]
?- between(1, 3, X).                      % X = 1 ; X = 2 ; X = 3
```

## Inspecting and building terms

| Predicate | Purpose |
|---|---|
| `functor(T, Name, Arity)` | Term's functor and arity, or build one. |
| `arg(N, T, A)` | The `N`th argument of a compound. |
| `T =.. List` | "Univ" — decompose/build a term as `[Functor|Args]`. |
| `copy_term(T, Copy)` | A fresh copy with renamed variables. |

Type tests classify a term: `var/1`, `nonvar/1`, `atom/1`, `number/1`,
`integer/1`, `float/1`, `compound/1`, `is_list/1`.

## Standard order of terms

Terms have a total order — **Variables < Numbers < Atoms < Compounds** (and
a float sorts just before an equal-valued integer). Compare with `@<`,
`@>`, `@=<`, `@>=` or `compare/3`, and sort with `sort/2` (dedupes) or
`msort/2` (keeps duplicates).

## Exceptions

`throw/1` raises a term; `catch(Goal, Catcher, Recovery)` runs `Recovery` if
`Goal` throws something that unifies with `Catcher`. Errors use the ISO
taxonomy — `error(Formal, Context)` where `Formal` is `type_error/2`,
`existence_error/2`, `evaluation_error/1`, `instantiation_error`, and so on.

Calling a predicate that is **defined nowhere** raises
`existence_error(procedure, Name/Arity)` — catchable, per ISO. Because the
compiler sees the whole program, it also *warns* about such calls at build
time (see Compiler Usage and `--deny-undefined`).

## What this subset leaves out

These are deliberate omissions, not gaps to be filled:

- **No `assert`/`retract`.** The knowledge base is immutable — compiled at
  build time. `:- dynamic(F/A)` only makes an *undefined* predicate
  fail silently instead of raising `existence_error`; it does **not** enable
  runtime clause mutation.
- **No `op/3`.** The operator table is fixed (see Grammar & Operators).
- **No postfix operators**, **no module system**, **no DCG**, **no
  `bagof`/`setof`**.

## Safety guarantees

Every compiled program is bounded: a runaway computation hits a **step
limit** and stops with `resource_error(steps)` — deliberately *uncatchable*,
so a buggy clause can't trap its own timeout. Combined with checked
arithmetic and no runtime file I/O, a compiled binary is safe to run on
untrusted input.
