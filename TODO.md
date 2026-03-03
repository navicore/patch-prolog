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

## Phase 3: Usability — COMPLETE

### 3a. `once/1` and `call/1`
- [x] `once/1` — solve goal, take first solution, cut (uses `try_solve_once` + choice stack truncation)
- [x] `call/1` — execute a term as a goal (basic meta-call)
- Files: `builtins.rs`, `solver.rs`

### 3b. Atom/string predicates
- [x] `atom_length/2`
- [x] `atom_concat/3`
- [x] `atom_chars/2`
- Files: `builtins.rs` (BuiltinResult variants), `solver.rs` (execution with mutable interner)

### 3c. Additional arithmetic functions
Added to `is/2` evaluation:
- [x] `abs/1`
- [x] `max/2`
- [x] `min/2`
- [x] `sign/1`
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
- 132 unit tests in prolog-core
- 101 integration tests in tests/integration.rs
- **233 total**

---

## Phase 5: Nice-to-have — COMPLETE

- [x] `write/1`, `writeln/1`, `nl/0` — I/O for debugging rules
- [x] `compare/3`, `@</2`, `@>/2`, `@=</2`, `@>=/2` — term ordering (ISO standard order)
- [x] `functor/3`, `arg/3`, `=../2` — term introspection/decomposition
- [x] `between/3` — integer enumeration (with backtracking, works inside findall)
- [x] `copy_term/2` — term copying with fresh variables
- [x] `succ/2`, `plus/3` — Peano arithmetic (bidirectional)
- [x] `msort/2`, `sort/2` — list sorting (sort removes duplicates)
- [x] `number_chars/2`, `number_codes/2` — number/string conversion (bidirectional)
- [ ] `assert/1`, `retract/1` — dynamic predicates (future)
- [ ] REPL mode (future)

---

## Architecture Notes

- **Workspace**: root `patch-prolog` binary + `crates/prolog-core` library
- **Build-time compilation**: `build.rs` compiles `knowledge/*.pl` into binary via bincode
- **Core modules**: term, tokenizer, parser, unify, builtins, solver, database, index
- **`[]` and `!` always interned**: `CompiledDatabase::new()` ensures this (required for findall, list ops, once/1)
- **Solver runtime interner**: Solver clones db.interner for atom predicates that create new atoms at runtime (atom_concat, atom_chars)
- **BuiltinResult enum**: handles control flow — solver processes Disjunction, IfThenElse, Conjunction, FindAll variants
- **Disjunction choice points**: use `disjunction: bool` flag on ChoicePoint to distinguish from clause alternatives
- **Parenthesized expressions**: `parse_paren_body()` handles `;`, `->`, and `,` as control flow operators

## Current Built-in Predicates (~55 total)

| Category | Predicates |
|----------|-----------|
| Core | `true`, `fail`, `false`, `!`, `=`, `\=`, `is`, `<`, `>`, `=<`, `>=`, `=:=`, `=\=`, `\+` |
| Type checking | `var/1`, `nonvar/1`, `atom/1`, `number/1`, `integer/1`, `float/1`, `compound/1`, `is_list/1` |
| Control flow | `;/2` (disjunction), `->/2` (if-then), `,/2` (conjunction) |
| Meta-call | `once/1`, `call/1` |
| Collection | `findall/3` |
| Atom/string | `atom_length/2`, `atom_concat/3`, `atom_chars/2` |
| Arithmetic ops | `+`, `-`, `*`, `/`, `mod`, unary `-`, `abs/1`, `max/2`, `min/2`, `sign/1` |
| I/O | `write/1`, `writeln/1`, `nl/0` |
| Term ordering | `compare/3`, `@</2`, `@>/2`, `@=</2`, `@>=/2` |
| Term introspection | `functor/3`, `arg/3`, `=../2` |
| Enumeration | `between/3` |
| Copying | `copy_term/2` |
| Peano arithmetic | `succ/2`, `plus/3` |
| Sorting | `msort/2`, `sort/2` |
| Number conversion | `number_chars/2`, `number_codes/2` |

## Stdlib (knowledge/stdlib.pl)

`member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`
