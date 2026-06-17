# Lessons from patch-prolog (v1)

**Status: accepted — this document is the "never again" list.**

patch-prolog set out to be a Prolog *compiler*: "compile rules at build
time into a self-contained native binary." What shipped was a very good
Prolog *interpreter* wearing a compiler costume. This document records
exactly how that happened so reviews of this repo can check against it.

## What v1 actually did

`prlg compile` (v1 `crates/cli/src/compiler.rs`):

1. Parsed the `.pl` files (good — parse errors at compile time).
2. Serialized the clause database to `compiled_db.bin` with bincode.
3. **Scaffolded a temporary Cargo project** whose generated `main.rs`
   did `include_bytes!("compiled_db.bin")` and ran the full `Solver`.
4. **Shelled out to `cargo build --release`.**

Consequences:

- Users needed a **full Rust toolchain** to "compile" their rules —
  the headline requirement (standalone compiler) was silently lost.
- The output binary contained the entire tree-walking interpreter
  (`solver.rs`, 3,147 lines) plus serialized data. Nothing about the
  user's program was compiled; execution was interpretation, always.
- Every rule change re-built the interpreter from source via cargo —
  the exact pain (slow recompiles) the project existed to remove.

## Why it drifted

- The interpreter was built first (reasonable for validating semantics),
  but "compile" was then defined as *embedding data into the
  interpreter* because that was the path of least resistance.
- No early end-to-end check of the actual requirement: "run the output
  binary on a machine with no Rust installed."
- No architectural doc pinned what "compile" must mean; each increment
  was locally sensible.

## The rules this repo enforces

1. **A compiled binary contains no clause interpreter.** Clause control
   flow (selection, unification sequencing, backtracking joins) is LLVM
   IR generated per predicate. The runtime staticlib provides primitives
   (heap, trail, unify, builtins) — it has no `solve()` loop over a
   clause database. If a change adds "walk the clauses at runtime," it
   is v1 again; reject it.
2. **`plgc` never invokes cargo or rustc.** The only external tool a
   user needs is clang (link step). The runtime is embedded in `plgc`
   as a prebuilt `.a` (patch-seq pattern).
3. **`plgc run` compiles.** Fast iteration is a temp-binary compile +
   exec, so dev mode and production mode share one execution path. v1's
   `run` was a second, in-process interpreter path — the two paths could
   (and did conceptually) diverge.
4. **The requirement is tested in CI**: integration tests execute
   compiled binaries; the footprint check and dead-code check keep the
   runtime honest.

## What v1 got right (and we keep)

- The frontend: tokenizer, operator-precedence parser, ISO error
  taxonomy — ported nearly verbatim (M1).
- Documented semantics: `docs/ISO_COMPLIANCE.md` (unification without
  occurs check, floored mod, checked overflow, NaN rejection, term
  ordering, uncatchable step limit) — ported as the conformance spec.
- The wire contract: exit codes 0/1/2/3 and the JSON solutions format.
- Safety posture: step limits, overflow detection, no runtime file I/O.
- The test corpus: hundreds of (program, goal, expected) cases — ported
  to run against compiled binaries, and used differentially with the v1
  interpreter as a semantics oracle during the port.
