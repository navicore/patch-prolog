# Design: Exposing the language to Rust (experimental)

Status: **exploratory** — no commitment, no milestone. Critical analysis of
"use logic programming in a full-stack Rust app via macros / a DSL rather
than FFI."

## Intent

Make patch-prolog usable *from* Rust as a first-class part of an application,
not as an external `--query`/JSON subprocess. The hope: because the engine is
written in Rust, a macro or embedded-DSL surface could be tighter and more
ergonomic than FFI.

## The central confusion (must resolve before any work)

"Expose the language to Rust" means at least three different things. Only one
is both valuable *and* consistent with this architecture:

1. **Write Prolog clauses inside Rust source** (`prolog! { parent(tom,bob). }`).
   A proc-macro runs in `rustc`. It cannot emit our predicate native code —
   that is LLVM-IR-text → clang, a separate compiler. So the macro can only
   (a) re-implement a solver in Rust, or (b) shell out to `plgc` at build time.
   - (a) is the **embedded interpreter the architecture explicitly rejected**
     (ARCHITECTURE.md "rejected alternative"), *plus* it creates a **second
     Prolog semantics** — a shadow engine that will diverge from the
     LLVM-codegen one. This is strictly worse than the shadow *parser* the
     project deliberately refuses (M7/LESSONS_FROM_V1: "one vocabulary"). Near-
     disqualifying.
   - (b) is fine, but it is build-time codegen + FFI with a nicer face — see #3.

2. **A concatenative / combinator DSL for building goals in Rust.** Weakest
   option. Point-free postfix stacks fight Rust's type system, and a relational
   model maps poorly onto a value-stack. Adds a surface without removing the
   real seam. Recommend dropping it.

3. **Call compiled predicates in-process from Rust**, passing Rust data in and
   getting solutions back without the subprocess/JSON boundary. This is the
   genuinely valuable thing, and the rest of this doc is about it.

## The seam does not move

The boundary between Rust and compiled Prolog is the **C ABI at the linker**,
because the two halves are produced by two different compilers. Macros can
*decorate* that seam (generate `extern "C"` decls, marshalling glue, build
orchestration); they cannot *dissolve* it. "Macros vs FFI" is a false choice —
the honest design is **FFI with a macro-generated ergonomic face**. Anyone
selling "deep integration removes the FFI" is selling interpretation (#1a).

## The gating decision: query vs knowledge base

What does the full-stack app want to vary at runtime?

- **The query** (ground/var goals against a fixed program): *fully supported,
  cheap.* The runtime already parses a query into the program's atom id-space
  (new atoms get fresh ids that unify with nothing) and re-enters compiled code
  via the registry. Pass runtime data *as query-time term structure* (lists,
  args) and `member/2` etc. over it — works today.
- **The knowledge base** (add facts/relations from DB rows, request state, at
  runtime): **forbidden by design.** Facts are compile-time `.rodata` (M9);
  runtime `--facts` loading and `assert/retract` were *rejected* to keep the
  immutable-binary thesis (ROADMAP "Future"). If the app needs to derive over
  runtime-supplied *relations*, this engine fights it — and a different tool
  (an in-process Datalog/CHR library) may simply be the right answer. **Decide
  this first; it determines whether embedding is even the right project.**

## Approach (only if #3 + "vary the query" is the real need)

A thin, honest stack — each layer small and independently testable:

1. **`plgc --emit obj|staticlib`**: stop at a linkable artifact instead of a
   final executable (the `.ll` → clang path already exists; this is a new emit
   mode, not a new pipeline). The program's globals (atom table, registry,
   predicate fns) become linkable symbols.
2. **A callback run-query ABI** in the runtime: today `entry.rs::main` parses
   argv and prints JSON. Factor out a `plg_rt_run_query(m, goal, *solution_cb)`
   that invokes a callback per solution instead of writing stdout. `Machine`,
   `parse_query`, `solve`/`drive` are already the reusable core.
3. **A safe wrapper crate** (`patch-prolog-embed`?) owning the `Machine`
   lifecycle and turning the CPS/tagged-word ABI into a Rust iterator of
   solutions. This is where the unsafe is contained, once.
4. **`#[derive(ToTerm/FromTerm)]`**: typed marshalling between a Rust struct and
   a Prolog term shape. **This is the one place macros genuinely earn their
   keep** — and it is fully decoupled from the solver.
5. **A `build.rs` helper** that compiles a `.pl` (via #1) and links it — the
   plumbing that makes 1–4 usable, and the build-time validation point where a
   `prolog!`-style macro could surface parse errors at Rust-compile-time
   *without* implementing a solver.

Note: in-process `plg-compiler` linking is already a stated design target
(M6 note) — the embedding seam and that goal share plumbing.

## Constraints / what must not break

- **No second solver.** One Prolog semantics, ever. Kills #1a.
- **Runtime footprint rule** still binds anything linked into user programs
  (`plg-shared`/`plg-runtime`): no clap/serde creep.
- **Immutable-binary thesis** is not reopened: embedding exposes *querying a
  fixed program*, not runtime clause mutation.
- `plg-shared::cell` stays the single source of the word/cell ABI; any Rust-side
  marshalling builds on it, never a parallel copy.

## Domain events

- *Build time*: `.pl compiled → linkable object` (new); host link consumes it.
- *Runtime*: `query submitted → {solution emitted}* → exhausted|error`. The new
  event is **solution-emitted-as-callback**, replacing solution-written-to-stdout
  for the embedded path. Exit-code semantics become `Result`/iterator-end.

## Checkpoints

- A Rust integration test links a compiled `deps.pl` and iterates solutions of
  `needs(app, X)` in-process — byte-equivalent to the `--query` JSON path.
- `#[derive(FromTerm)]` round-trips a struct ↔ term independent of any solver.
- Footprint + `ldd` standalone-contract gates still pass for an embedding host.
- Decision recorded: "vary the query" (proceed) vs "vary the KB" (wrong tool —
  stop, or open a separate non-AOT track).

## Recommendation

Pursue #3 *only if* the real need is "vary the query against a fixed knowledge
base." Drop #1 (interpreter/shadow engine) and #2 (concatenative DSL). The
deliverable is an honest, thin FFI made ergonomic by exactly two macros
(marshalling derive + build-time validation) — not a macro that replaces the
compiler. If the app actually needs runtime-mutable relations, this is the
wrong engine and the right move is to say so.
