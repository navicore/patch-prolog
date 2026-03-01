# patch-prolog Completeness & Usability Plan

## Context

patch-prolog is a Rust-based Prolog engine for linting generative AI output. It compiles `.pl` rules at build time into a binary, then runs queries at runtime. The core engine works — unification, backtracking, cut, negation-as-failure, arithmetic — and Phase 1+2 improvements are complete.

---

## Phase 1: Critical Built-ins — COMPLETE

- [x] **1a. Type-checking predicates** in `builtins.rs`
  - [x] `var/1`, `nonvar/1`, `atom/1`, `number/1`, `integer/1`, `float/1`, `compound/1`, `is_list/1`

- [x] **1b. List predicates** as Prolog rules in `knowledge/stdlib.pl`
  - [x] `member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`

- [x] **1c. Solution collection: `findall/3`**
  - [x] Invokes solver internally, collects all solutions, builds result list
  - [x] Handles conjunction, disjunction, negation within findall goals

- [x] **1d. If-then-else and disjunction**
  - [x] Tokenizer: `->` (Arrow) and `;` (Semicolon) tokens
  - [x] Parser: `parse_paren_body()` handles `(Cond -> Then ; Else)` and `(A ; B)`
  - [x] Solver: `BuiltinResult` variants — Disjunction, IfThenElse, IfThen, Conjunction
  - [x] Disjunction choice points on backtrack stack

---

## Phase 2: Robustness & Safety — COMPLETE

- [x] **2a. Recursion depth limit**
  - [x] `max_depth` field on Solver (default 10,000)
  - [x] `with_max_depth()` builder method
  - [x] Returns `SolveResult::Error` instead of stack overflow

- [x] **2b. Integer overflow protection**
  - [x] `checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_neg`
  - [x] Returns arithmetic error on overflow

- [x] **2c. Float NaN/Infinity detection**
  - [x] `check_float()` validates every float operation result
  - [x] Returns error for NaN or Infinity

---

## Phase 3: Usability

### 3a. `once/1` and `call/1`
- [ ] `once/1` — solve goal, take first solution, cut
- [ ] `call/1` — execute a term as a goal (basic meta-call)
- Files: `builtins.rs`, `solver.rs`

### 3b. Atom/string predicates
- [ ] `atom_length/2`
- [ ] `atom_concat/3`
- [ ] `atom_chars/2`
- File: `builtins.rs`

### 3c. Additional arithmetic functions
Add to `is/2` evaluation:
- [ ] `abs/1`
- [ ] `max/2`
- [ ] `min/2`
- [ ] `sign/1`
- File: `builtins.rs`

---

## Phase 4: Testing — COMPLETE

- [x] **4a. Integration tests** (`tests/integration.rs`)
  - [x] Full pipeline tests (family relationships, factorial, list ops)
  - [x] Type-checking, if-then-else, disjunction, findall tests
  - [x] Error case tests (depth limit, overflow, div-by-zero, unbound vars)
  - [x] Edge case tests (empty KB, no matching predicate, parse errors, ground queries)

- [x] **4b. Edge case tests**
  - [x] Deeply recursive rules (verify depth limit works)
  - [x] Integer overflow in arithmetic
  - [x] Empty knowledge base with queries
  - [x] Division by zero
  - [x] Unbound variable in arithmetic

- [x] **4c. Stdlib tests**
  - [x] List predicates tested end-to-end in integration tests

### Test counts
- 111 unit tests in prolog-core
- 23 integration tests in tests/integration.rs
- **134 total**

---

## Phase 5: Nice-to-have (lower priority)

- [ ] `write/1`, `writeln/1`, `nl/0` — for debugging rules
- [ ] `compare/3`, `@</2` family — term ordering
- [ ] `functor/3`, `arg/3`, `=../2` — term introspection
- [ ] `assert/1`, `retract/1` — dynamic predicates
- [ ] `between/3` — integer enumeration
- [ ] `copy_term/2` — term copying with fresh variables
- [ ] `succ/2`, `plus/3` — Peano arithmetic
- [ ] `msort/2`, `sort/2` — list sorting
- [ ] `number_chars/2`, `number_codes/2` — number/string conversion
- [ ] REPL mode

---

## Architecture Notes

- **Workspace**: root `patch-prolog` binary + `crates/prolog-core` library
- **Build-time compilation**: `build.rs` compiles `knowledge/*.pl` into binary via bincode
- **Core modules**: term, tokenizer, parser, unify, builtins, solver, database, index
- **`[]` always interned**: `CompiledDatabase::new()` ensures this (required for findall, list ops)
- **BuiltinResult enum**: handles control flow — solver processes Disjunction, IfThenElse, Conjunction, FindAll variants
- **Disjunction choice points**: use `disjunction: bool` flag on ChoicePoint to distinguish from clause alternatives
- **Parenthesized expressions**: `parse_paren_body()` handles `;`, `->`, and `,` as control flow operators

## Current Built-in Predicates (~30 total)

| Category | Predicates |
|----------|-----------|
| Core | `true`, `fail`, `false`, `!`, `=`, `\=`, `is`, `<`, `>`, `=<`, `>=`, `=:=`, `=\=`, `\+` |
| Type checking | `var/1`, `nonvar/1`, `atom/1`, `number/1`, `integer/1`, `float/1`, `compound/1`, `is_list/1` |
| Control flow | `;/2` (disjunction), `->/2` (if-then), `,/2` (conjunction) |
| Collection | `findall/3` |
| Arithmetic ops | `+`, `-`, `*`, `/`, `mod`, unary `-` |

## Stdlib (knowledge/stdlib.pl)

`member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`
