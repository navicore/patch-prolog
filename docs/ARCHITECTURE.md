# Architecture

patch-prolog is a Rust-based Prolog engine for linting generative AI output. It compiles `.pl` rules at build time into the binary, then runs queries at runtime against that embedded knowledge base.

## Workspace Layout

```
patch-prolog/              # Binary crate (CLI)
  src/main.rs              # CLI entry point, JSON/text output
  build.rs                 # Build-time Prolog compilation
  knowledge/               # .pl files compiled into binary
    stdlib.pl              # member/2, append/3, length/2, etc.
  tests/
    integration.rs         # End-to-end integration tests
crates/
  patch-prolog-core/             # Library crate (engine)
    src/
      term.rs              # Term, Clause, StringInterner, AtomId, VarId
      tokenizer.rs         # Lexer: Prolog source -> tokens
      parser.rs            # Parser: tokens -> Term/Clause AST
      unify.rs             # Substitution, unification, apply (deep walk)
      builtins.rs          # Built-in predicate dispatch -> BuiltinResult
      solver.rs            # Query resolution engine (core logic)
      database.rs          # CompiledDatabase: interner + clauses + index
      index.rs             # First-argument indexing for clause lookup
      lib.rs               # Public API re-exports
```

## Build-Time Compilation Pipeline

```
knowledge/*.pl  -->  build.rs  -->  CompiledDatabase  -->  bincode  -->  compiled_db.bin
                     (parse)        (interner+clauses     (serialize)    (embedded via
                                    +predicate_index)                     include_bytes!)
```

1. `build.rs` reads all `.pl` files from `knowledge/`, sorted alphabetically
2. Parses them into `Vec<Clause>` using a shared `StringInterner`
3. Builds a `CompiledDatabase` (interns `[]` and `!`, builds predicate index)
4. Serializes via bincode to `$OUT_DIR/compiled_db.bin`
5. Binary includes it with `include_bytes!` — zero runtime file I/O

## Term Representation

```rust
enum Term {
    Atom(AtomId),                              // Interned string ID
    Var(VarId),                                // Variable ID (u32)
    Integer(i64),
    Float(f64),
    Compound { functor: AtomId, args: Vec<Term> },
    List { head: Box<Term>, tail: Box<Term> }, // Cons cell
}
```

Atoms are interned — unification compares `u32` IDs, not strings. The interner is shared between the database and solver; the solver clones it for runtime atom creation (e.g., `atom_concat/3`).

## Solver Architecture

The solver has **three resolution paths**, each serving a different context:

### 1. `solve()` — Main Loop

The primary query resolution engine. Iterates a goal list (`VecDeque<Term>`), dispatches built-ins via `exec_builtin`, and manages backtracking through a choice point stack.

- Returns `SolveResult` (Success/Failure/Error)
- Supports full backtracking via `ChoicePoint` stack
- Handles cut via `cut_barrier` flag on choice points
- Step counter returns `SolveResult::Error` on limit

### 2. `try_solve_once()` — Single-Solution Context

Used by `\+` (NAF), `once/1`, if-then-else conditions. Returns `bool` — finds at most one solution.

- Cannot return errors directly (returns `false` on step limit)
- Sets `steps_exceeded` flag so callers can distinguish timeout from failure
- Sets `cut_in_try_solve` flag so clause iteration respects cut
- Used for contexts that need "does this succeed?" without full backtracking

### 3. `try_solve_collecting()` — Collection Context

Used by `findall/3`. Collects all solutions by iterating all clause alternatives.

- Returns `bool` (whether any solution was found)
- Pushes template instances into a results `Vec<Term>`
- Respects `cut_in_try_solve` in clause iteration (cut stops collection)
- Steps accumulate globally (no independent budget)

### Shared Helper: `try_exec_misc()`

Handles built-in predicates that are common across `try_solve_once` and `try_solve_collecting`. Returns `Option<bool>` — this means it **cannot propagate errors**. This is a known architectural limitation: predicates that error in the main loop (e.g., `succ(-1, _)`) silently fail in `try_exec_misc` contexts.

## Backtracking

Backtracking uses a **choice point stack**:

```rust
struct ChoicePoint {
    goals: VecDeque<Term>,   // Goal list snapshot
    untried: Vec<usize>,     // Remaining clause indices
    trail_mark: usize,       // Substitution undo point
    var_counter: VarId,      // Variable counter snapshot
    cut_barrier: bool,       // Marks clause-group boundary
    disjunction: bool,       // Disjunction alternative
}
```

- **Trail-based substitution**: bindings are recorded on a trail; `undo_to(mark)` unbinds everything since the mark
- **Cut**: removes choice points up to and including the nearest `cut_barrier`
- **Disjunction**: `;/2` pushes a choice point for the right branch

## Unification

- ISO-compliant: `=/2` does **not** perform occurs check (per ISO 8.3.2)
- `X = f(X)` succeeds, creating a circular term
- `apply()` has cycle detection via a visited-variable set to safely resolve circular terms
- Float equality uses `to_bits()` for structural comparison (NaN equals NaN)

## Safety Guarantees

- **Step limit** (default 10,000): enforced in all three solver paths. Configurable via `with_max_depth()`.
- **Integer overflow**: all arithmetic uses `checked_add/sub/mul/div/neg/abs`
- **Float NaN/Infinity**: `check_float()` validates every arithmetic float result; `parse::<f64>()` results are also checked
- **Mod semantics**: ISO floored semantics (`rem_euclid`), guards against `i64::MIN` divisor

## CLI Interface

```
patch-prolog --query "violation(X)" [--limit N] [--format json|text]
```

Exit codes:
- `0` = no solutions (compliant)
- `1` = solutions found (violations)
- `2` = parse error
- `3` = runtime error

## Known Limitations

- `try_exec_misc` cannot propagate errors — predicates that error in the main loop silently fail inside `once/1`, `\+`, and `findall/3`
- `try_solve_collecting` is stack-recursive — deep goal chains inside `findall` can overflow the Rust stack before the step limit fires
- `atom_concat/3` is not reversible (requires both input arguments bound)
- No `assert/1` or `retract/1` (knowledge base is immutable after build)
- No `call/N` for N > 1
