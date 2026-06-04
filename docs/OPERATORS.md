# Operators

`prlg` supports the standard **prefix** and **infix** operators of ISO Prolog.
**Postfix is not supported** — see [Postfix is unsupported, by design](#postfix-is-unsupported-by-design) below.

`op/3` (user-defined operators) is also not supported. The operator table below
is the complete fixed set the engine recognizes.

## Canonical form is always available

Every operator term is sugar for a compound. Anything you can write with an
operator, you can write in functor notation:

| Operator form | Canonical form |
|---|---|
| `2 + 3`       | `+(2, 3)` |
| `- a`         | `-(a)` |
| `\+ Goal`     | `\+(Goal)` |
| `H = [X\|T]`   | `=(H, '.'(X, T))` |
| `X is 6 /\ 3` | `is(X, /\(6, 3))` |

The arithmetic evaluator, unification, and clause resolution see the compound
form — operator syntax is purely a reader-level convenience. If you ever hit a
parse-precedence surprise, the canonical form will always parse unambiguously.

## Prefix operators

| Op   | Type | Prec | Meaning |
|------|------|------|---------|
| `-`  | fy   | 200  | Arithmetic negation. Folds for literal numbers: `-3` reads as the integer `-3`, not the compound `-(3)`. |
| `+`  | fy   | 200  | Identity / explicit positive sign. Folds for literal numbers: `+3` reads as `3`. For non-literals, builds `+(X)`. |
| `\`  | fy   | 200  | Bitwise complement (evaluable in `is/2`). No folding — `\3` reads as the compound `\(3)`. |
| `\+` | fy   | 900  | Negation as failure (control construct). |

`!` (cut) is a 0-arity atom, not a prefix operator — it does not take an argument.

The clause and directive markers `:-` and `?-` are recognized by the parser but
are not user-redefinable operators:

- `Head :- Body.` — clause separator (infix at the top level of a program).
- `:- Directive.` — directive prefix (e.g. `:- dynamic(foo/1).`).
- `?- Goal.` — query prefix (optional, accepted by the query reader).

## Infix operators

Listed from loosest binding to tightest. Associativity uses the standard
fixity codes (`xfx` non-associative, `xfy` right-associative, `yfx`
left-associative).

| Op   | Type | Prec | Meaning |
|------|------|------|---------|
| `;`  | xfy  | 1100 | Disjunction (also the if-then-else outer in `(C -> T ; E)`). |
| `->` | xfy  | 1050 | If-then. Only meaningful inside a parenthesized goal expression. |
| `,`  | xfy  | 1000 | Conjunction. |
| `=`  | xfx  | 700  | Unification. |
| `\=` | xfx  | 700  | Not-unifiable. |
| `==` | xfx  | 700  | Term identity (no unification). |
| `\==`| xfx  | 700  | Term non-identity. |
| `is` | xfx  | 700  | Arithmetic evaluation. |
| `<`  | xfx  | 700  | Arithmetic less-than. |
| `>`  | xfx  | 700  | Arithmetic greater-than. |
| `=<` | xfx  | 700  | Arithmetic less-or-equal. (Note: `=<`, not `<=`.) |
| `>=` | xfx  | 700  | Arithmetic greater-or-equal. |
| `=:=`| xfx  | 700  | Arithmetic equality. |
| `=\=`| xfx  | 700  | Arithmetic inequality. |
| `@<` | xfx  | 700  | Standard term ordering: less. |
| `@>` | xfx  | 700  | Standard term ordering: greater. |
| `@=<`| xfx  | 700  | Standard term ordering: less-or-equal. |
| `@>=`| xfx  | 700  | Standard term ordering: greater-or-equal. |
| `=..`| xfx  | 700  | Univ: term ↔ list decomposition. |
| `+`  | yfx  | 500  | Addition. |
| `-`  | yfx  | 500  | Subtraction. |
| `/\` | yfx  | 500  | Bitwise AND. |
| `\/` | yfx  | 500  | Bitwise OR. |
| `xor`| yfx  | 500  | Bitwise XOR. |
| `*`  | yfx  | 400  | Multiplication. |
| `/`  | yfx  | 400  | Float division (ISO 9.1.4 — always returns a float, even for integer operands). |
| `//` | yfx  | 400  | Integer division (truncates toward zero). |
| `mod`| yfx  | 400  | ISO modulo (sign follows divisor). |
| `rem`| yfx  | 400  | ISO remainder (sign follows dividend). |
| `div`| yfx  | 400  | Floor division (rounds toward −∞; differs from `//` when signs disagree). |
| `<<` | yfx  | 400  | Shift left (integer). |
| `>>` | yfx  | 400  | Shift right (integer). |
| `**` | xfx  | 200  | Float power (always returns a float). |
| `^`  | xfy  | 200  | Integer power for `Int ^ Int` with non-negative exponent; float otherwise. |
| `:`  | xfy  | 200  | Module qualifier (parser-only; no module system, but the term parses and round-trips). |

## Postfix is unsupported, by design

`prlg` recognizes no postfix operators and none are planned.

Three reasons:

1. **Postfix is pure syntactic sugar — zero expressive power.** A term written
   with a hypothetical postfix `yf` operator (`X yf`) is exactly the compound
   `yf(X)`. The form unification and resolution see is identical to what
   functor notation already gives you.
2. **Standard Prolog ships no built-in postfix operators.** ISO 13211-1
   defines the postfix *types* (`xf`, `yf`) so a program could declare one via
   `op/3`, but the standard operator table itself contains zero postfix
   entries.
3. **`op/3` is not supported.** Without `op/3` there is no path to a
   user-defined operator of any fixity, so the choice is moot anyway.

Teaching materials sometimes present postfix alongside prefix and infix for
symmetry. Treat that as a description of the language family; this engine is a
deliberate subset.

## Known quirks

- **Literal folding.** `-3` and `+3` read as the integer literals `-3` and `3`,
  not the compounds `-(3)` and `+(3)`. To force the compound form (e.g. when
  pattern-matching with `functor/3`), use a non-numeric operand: `- a`, `+ X`.
  `\3` is not folded — it parses as `\(3)`.
- **`!` is an atom, not an operator.** `! X` is a syntax error; if you want
  cut followed by `X`, write `!, X`.
- **`->` outside parens.** The if-then operator is only meaningful inside a
  parenthesized goal expression. `( Cond -> Then ; Else )` is the idiom; bare
  `Cond -> Then` at clause-body level may not parse the way you expect.
- **`/` is always float.** Per ISO 9.1.4, `10 / 3` evaluates to `3.3333...`,
  not `3`. Use `//` (truncating) or `div` (floor) for integer division.
- **`=<` vs `<=`.** The Prolog convention is `=<`. Writing `<=` is a parse
  error.

## See also

- [docs/ARCHITECTURE.md](ARCHITECTURE.md) — overall engine structure.
- ISO/IEC 13211-1:1995, §6.3.4 (operator notation) and §8.7 (arithmetic).
