# Testing

## Running Tests

```bash
cargo test --all          # Everything (unit + integration + doc-tests)
cargo test -p patch-prolog-core # Unit tests only (patch-prolog-core crate)
cargo test                # Integration tests only (root crate)
```

## Test Organization

### Unit Tests (134 tests)

Located in `#[cfg(test)]` modules within each source file in `crates/patch-prolog-core/src/`:

| File | Tests | Coverage |
|------|-------|----------|
| `term.rs` | 6 | StringInterner, functor_arity, first_arg_key, serialization |
| `tokenizer.rs` | 17 | Token types, operators, strings, edge cases |
| `parser.rs` | 16 | Terms, rules, queries, operators, lists, errors |
| `unify.rs` | 14 | Variable binding, compound unification, occurs check removal, cycle detection, float to_bits equality |
| `builtins.rs` | 65 | Arithmetic, type checking, comparisons, overflow, NaN, mod edge cases |
| `database.rs` | 3 | Serialization roundtrip, indexed lookup |
| `index.rs` | 6 | First-argument indexing |
| `solver.rs` | 7 | Step limit, solution iteration |

### Integration Tests (143 tests)

Located in `tests/integration.rs`. Full pipeline tests: parse source + query, solve, verify solutions.

#### Helper Functions

```rust
solve_all(source, query)         -> Vec<Vec<(name, value)>>  // All solutions
first_binding(source, query, var) -> Option<String>           // First solution, one var
solve_with_limit(source, query, limit) -> usize              // Count with step limit
solve_expect_error(source, query) -> String                   // Expect error message
```

#### Test Categories

- **Core predicates**: unification, arithmetic, comparison, cut, negation
- **Type checking**: var/1, nonvar/1, atom/1, number/1, integer/1, float/1, compound/1, is_list/1
- **Control flow**: if-then-else, disjunction, once/1, call/1
- **findall/3**: filters, empty results, cut inside findall, step limit truncation
- **List operations**: member, append, length, last, reverse, nth0, nth1
- **Term ordering**: compare/3, @</2 family, compound term ordering
- **Term introspection**: functor/3, arg/3, =../2
- **Enumeration**: between/3 (bound X fast path, unbound iteration, step limits)
- **Sorting**: msort/2, sort/2
- **String/number**: atom_length, atom_concat, atom_chars, number_chars, number_codes
- **Peano**: succ/2, plus/3
- **Safety**: depth limits, integer overflow, float NaN/Infinity, division by zero
- **Regression tests**: one or more per PR review round (rounds 9-15), covering specific bugs found and fixed

## Test Counts

- 134 unit tests in patch-prolog-core
- 143 integration tests in tests/integration.rs
- **277 total**

## Writing Tests

Integration tests should use the helper functions above. Prefer `first_binding` for simple value checks and `solve_all` when you need to verify solution count or multiple bindings. Use `solve_expect_error` when the query should produce a runtime error.

For findall tests, use `first_binding(source, query, "Xs")` — the template variable inside findall is unbound after findall restores state, so checking it would return `_0`.

Tests that need custom step limits should construct the solver manually with `.with_max_depth(N)`.
