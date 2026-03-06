# Roadmap

## Completed

### Phase 1: Critical Built-ins
Type checking (`var/1`, `nonvar/1`, `atom/1`, `number/1`, `integer/1`, `float/1`, `compound/1`, `is_list/1`), list predicates in stdlib (`member/2`, `append/3`, `length/2`, `last/2`, `reverse/2`, `nth0/3`, `nth1/3`), `findall/3`, if-then-else, disjunction.

### Phase 2: Robustness & Safety
Step limit (default 10,000), integer overflow protection (`checked_*`), float NaN/Infinity detection.

### Phase 3: Usability
`once/1`, `call/1`, atom predicates (`atom_length/2`, `atom_concat/3`, `atom_chars/2`), arithmetic functions (`abs/1`, `max/2`, `min/2`, `sign/1`).

### Phase 4: Testing
277 tests (134 unit + 143 integration). Full pipeline tests, error cases, edge cases, stdlib coverage.

### Phase 5: Nice-to-have Built-ins
I/O (`write/1`, `writeln/1`, `nl/0`), term ordering (`compare/3`, `@</2` family), term introspection (`functor/3`, `arg/3`, `=../2`), `between/3`, `copy_term/2`, Peano arithmetic (`succ/2`, `plus/3`), sorting (`msort/2`, `sort/2`), number conversion (`number_chars/2`, `number_codes/2`).

---

## Future

- [ ] `assert/1`, `retract/1` — dynamic predicates (requires mutable knowledge base)
- [ ] REPL mode — interactive query shell
- [ ] `call/N` for N > 1 — higher-order meta-call
- [ ] `unify_with_occurs_check/2` — ISO standard predicate (infrastructure exists)
- [ ] `catch/3`, `throw/1` — exception handling system
- [ ] Refactor `try_exec_misc` to propagate errors — resolves the silent-failure inconsistency between main solver and `once/1`/`findall/3` contexts
- [ ] Convert `try_solve_collecting` from stack-recursive to iterative — prevents Rust stack overflow on deep `findall` goals
