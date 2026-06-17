# Builtin Vocabulary Table (LSP port, delta-1)

**Status: landed (2026-06-09). First step of `docs/design/done/LSP_PORT.md`.**

## Intent

The set of builtin/control names the language knows is currently
hand-maintained in **two** places that must agree:
`DET_BUILTINS` in `crates/compiler/src/codegen/lower.rs` (`(name, arity,
C symbol)`) and the query-side dispatch in
`crates/runtime/src/control.rs` (`(name, arity) => Rust fn`). The new
`plg-lsp` crate's completion and hover would be a **third** copy (v1 kept
its own `BUILTIN_DOCS` table in `hover.rs` plus `builtin_atom_names()`/
`builtin_functor_names()` from core). Promote the *vocabulary itself* —
`(name, arity, one-line doc)` for every builtin, control construct,
inline op, and reserved atom — to a single zero-dep table in
`plg-shared`, so completion and hover read it directly and codegen and
runtime are checked against it.

## Constraints

- **`plg-shared` stays zero-dependency** (it is linked into every
  compiled program via `plg-runtime`). The table is plain `const` data
  and helper `fn`s — no serde, no macros pulling deps.
- **Doc strings must never reach a compiled program binary.** A
  referenced `&[BuiltinSpec]` array emits every `doc` pointer it holds.
  Therefore `plg-runtime` must **not** reference the table outside
  `#[cfg(test)]`; it keeps its hand-written dispatch `match` (it has to —
  it calls the actual Rust fns). The 432K-stripped hello-world gate
  (ROADMAP M2) is the regression guard.
- **The C-symbol mapping stays compiler-side.** `=..`→`univ` etc. is not
  mechanical; `DET_BUILTINS` keeps its symbol column. The shared table
  carries no symbols.
- Out of scope: `STDLIB_PL` relocation (delta-2), diagnostics positions
  (delta-3), the LSP port proper, structured error spans, any
  behavior/codegen change. Vocabulary membership only — no new builtins.

## Approach

New module `plg-shared/src/builtins.rs`:

```rust
pub enum BuiltinKind { Control, Inline, Det, Atom }   // partitions the validation
pub struct BuiltinSpec { pub name: &'static str, pub arity: u32,
                         pub kind: BuiltinKind, pub doc: &'static str }
pub const BUILTINS: &[BuiltinSpec] = &[ /* the full union, docs from v1 */ ];

impl BuiltinSpec {
    /// Completion-eligible iff the name is a typeable identifier
    /// (first char alphabetic/`_`). Offers `findall`/`once`/`is`/`nl`,
    /// suppresses operators and `!`/`\+`. NOT derived from `kind` —
    /// `is` (Inline) completes, `\+` (Control) does not. If a real
    /// counter-example appears, promote this to an explicit
    /// `complete: bool` column and overrule per-row.
    pub fn completable(&self) -> bool;
}

pub fn lookup(name: &str, arity: u32) -> Option<&'static BuiltinSpec>;
pub fn doc(name: &str) -> Option<&'static str>;           // hover: ALL rows, arity-insensitive
pub fn atom_names()    -> impl Iterator<Item = &'static str>;        // completion: completable() arity-0
pub fn functor_names() -> impl Iterator<Item = (&'static str, u32)>; // completion: completable() arity>0
```

Membership = the union already split across the new repo today:
`DET_BUILTINS` (Det) ∪ control (`,` `;` `->` `\+` `once` `catch` `throw`
`findall` `call` `between` — Control) ∪ inline (`=` `\=` `==` `\==` `is`
`compare`, `ARITH_OPS`, `ORDER_OPS` — Inline) ∪ atoms (`true` `fail`
`false` `!` `nl` — Atom). Doc text ported verbatim from v1's
`BUILTIN_DOCS`.

Consumers:
- **LSP completion / hover** — read `BUILTINS` directly (the only runtime
  reader of `doc`). Replaces v1's two core fns + local `BUILTIN_DOCS`.
  **Hover offers every row; completion offers only `completable()` rows**
  — bad completions (`,`, `\+`, `=..`) are worse than none; hover on
  them is valuable. The split lives in the accessors, so the LSP needs no
  filter of its own.
- **codegen `lower.rs`** — keeps `DET_BUILTINS` (with symbols). A
  `const`-assert (const-fn over both arrays) proves every `DET_BUILTINS`
  entry is a `kind == Det` row in `BUILTINS` with matching arity; a test
  proves the control/inline/atom names `lower_goal` recognizes are all
  present. `plgc` may carry the docs — it is a dev tool, not a compiled
  program.
- **runtime `control.rs`** — keeps its `match`; a `#[cfg(test)]` table
  asserts its recognized `(name, arity)` det set equals the `Det` subset
  of `BUILTINS`. No non-test reference → zero binary bytes.

Invariant chain: LSP ⟶reads⟶ shared ⟵const-assert⟵ codegen ⟵diff
corpus⟵ runtime, and shared ⟵cfg(test)⟵ runtime. The ADR's
"compile-time-shared" is literal for codegen (const-assert) and
test-time for runtime (match arms aren't const data); both run in
`just ci`.

## Domain Events

- **Builtin added / arity changed** → it must land in `BUILTINS` or CI
  fails. What must follow: add the `BuiltinSpec` row; add codegen
  symbol+lowering; add runtime dispatch arm; add a diff-corpus case.
- **Completion requested** (LSP) → consumes `atom_names`/`functor_names`
  (+ stdlib + buffer predicates) → emits items.
- **Hover requested** (LSP) → consumes `doc(name)` → markup, else falls
  through to the user-clause-head path.
- **`just ci`** → const-assert + the two subset tests fire; drift is a
  build/test failure, not a silent latent bug.

## Checkpoints

1. `BUILTINS` row count == v1 `BUILTIN_DOCS` entries reconciled with the
   current `DET_BUILTINS`/control/op sets (no name in one source missing
   from the table).
2. `cargo test -p plg-compiler` — const-assert (DET ⊆ BUILTINS) and the
   recognized-names test pass.
3. `cargo test -p plg-runtime` — det-dispatch-set == `Det` subset passes.
4. **Stripped hello-world binary still 432K** (and `strings` shows no
   doc text) — proves docs did not leak into the runtime/program.
5. Deliberately delete one `DET_BUILTINS` row → `plg-compiler` test goes
   red (negative check the invariant actually bites).

## Appendix: reconciled roster (55 rows)

Reconciled 2026-06-09 against new-engine sources (`DET_BUILTINS`,
`ARITH_OPS`, `ORDER_OPS`, `lower.rs`/`clause.rs`/`control.rs` arms) and
v1 `hover.rs::BUILTIN_DOCS` (52 entries). The only names with no v1 doc
are `,` `;` `->` (3 new one-liners below); the other 52 port verbatim.
`C` = `completable()`.

**Det (25)** — all `C` (alphabetic), docs from v1:
`var/1 nonvar/1 atom/1 number/1 integer/1 float/1 compound/1 is_list/1
functor/3 arg/3 =../2 copy_term/2 atom_length/2 atom_concat/3
atom_chars/2 number_chars/2 number_codes/2 msort/2 sort/2 succ/2 plus/3
unify_with_occurs_check/2 write/1 writeln/1 nl/0`

**Inline (16)** — docs from v1. `compare/3` and `is/2` are alphabetic →
`C`; the other 14 are operators → ¬`C`:
`=/2 \=/2 is/2 compare/3 ==/2 \==/2 @</2 @>/2 @=</2 @>=/2 </2 >/2 =</2
>=/2 =:=/2 =\=/2`

**Control (10):** `\+/1`(¬C) `once/1`(C) `catch/3`(C) `throw/1`(C)
`findall/3`(C) `call/1`(C, variadic) `between/3`(C) — docs from v1; and
`,/2` `;/2` `->/2` (¬C) — **new docs**:
- `,/2` → "`(A, B)` — conjunction: prove A, then B."
- `;/2` → "`(A ; B)` — disjunction: prove A, or B on backtracking. `(C -> T ; E)` reads as if-then-else."
- `->/2` → "`(C -> T)` — if-then: if C succeeds (committing to its first solution), prove T; otherwise fail."

**Atom (4):** `true/0`(C) `fail/0`(C) `false/0`(C) `!/0`(¬C) — docs from v1.

Deltas from v1's *completion* surface (intentional): `catch`/`throw`/
`unify_with_occurs_check` were doc'd but not offered by v1 — now offered
(`C`). v1's `call/1..8` rows collapse to one `call/1` (variadic noted in
doc). Net completion count is unchanged-or-better; hover gains `,` `;`
`->`.
