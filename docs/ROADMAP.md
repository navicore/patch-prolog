# Roadmap

Goal: feature parity with patch-prolog's documented ISO subset, as a
true standalone compiler. Each milestone has a hard verification gate.

## M0 — Scaffold ✅ (2026-06-04)

Workspace, four crates, justfile, Forgejo CI, docs skeleton, examples
ported. The build.rs runtime-embed chain works end to end.

## M1 — Frontend port ✅ (2026-06-04)

Ported v1's tokenizer/parser/term/error into `plg-shared` +
`plg-frontend`, split into focused modules, with all 48 v1 frontend
unit tests (15 tokenizer + 27 parser + 6 error).
`plgc check examples/family.pl` parses and reports `file:line:col`.

## M2 — Minimal end-to-end compilation ✅ (2026-06-04)

Runtime: Machine, cell heap, trail, generic unify, choice points,
registry, minimal goal parser, v1 wire-format output, exit codes, solve
driver. Codegen: facts, rules with conjunctive bodies, multi-clause
predicates with choice points, recursion. `plgc build` + `plgc run`
(compile-temp-and-exec, never interprets).

Gate results:
- `./fam --query "parent(tom, X)"` etc. byte-identical to the v1
  interpreter (including solution order, limit/exhausted semantics,
  existence/step-limit error text, exit codes).
- 5,000-deep `ancestor` recursion (≈12.5M backtracking ops) under a
  256KB stack — the musttail CPS design holds even at `-O0`.
- Compiled binaries link only libc/libm/libgcc_s; no Rust anywhere.
- Footprint: hello-world 4.2M with DWARF (`-g`), **432K stripped**
  (under the patch-seq ~730K bar; M5 decides the `-g` default).
- Known M2 limits: builtins/control reserved (compile fine, raise a
  clear runtime error when reached — `reserved_builtin()` in
  codegen/clause.rs shrinks as M3/M4 land); integer literals limited to
  i61 immediates (boxing in M4); runtime query parser handles
  atoms/vars/ints/compounds/lists/conjunctions (operators arrive with
  their builtins).

## M3 — Control and arithmetic

Backtracking across clauses/disjunctions, if-then-else, cut barriers,
negation-as-failure, first-argument indexing as IR switch, ISO
arithmetic (`is/2`, comparisons; floored mod, checked overflow, NaN
rejection), step limit.

Gate: recursive predicates (ancestor, append) match the v1 oracle;
deep determinate recursion runs in constant C stack (tested).

## M4 — Builtin parity and errors

All remaining v1 builtin families: type checks, term construction
(`functor/3`, `arg/3`, `=..`, `copy_term/2`), atom/number conversions,
lists, sorting, `findall/3`, `between/3`, I/O (`write/1`, `nl/0`),
`catch/3`/`throw/1` with the full ISO error taxonomy, dynamic
silent-fail.

Gate: the entire ported v1 integration suite passes against compiled
binaries; full differential corpus matches; dead-code guard passes.
Afterwards the differential dependency on the old repo is retired.

## M5 — Polish

Binary-size tuning, shell completions, shebang script mode, docs and
footprint finalization.

Gate: full `just ci` including `check-binary-contents`.

## Future (explicitly out of scope for now)

- REPL and LSP (v1 had both; they return only after compiler parity)
- WAM-style instruction-level codegen (performance escape hatch)
- Copying GC for long-running determinate queries
- `assert/retract` beyond the silent-fail dynamic contract
- Cross-compilation (`--target`)
