# Unsafe-Rust PoC Candidates (learning study)

**Status: exploratory note (2026-06-14).** Educational, not a committed
work item. Frames where `unsafe` *could* earn its keep in this engine,
which candidate best mirrors the `iddqd` pattern, and the guardrails any
PoC must carry to stay true to the project's correctness ethos.

## Intent

`iddqd` is interesting for one specific reason: it encapsulates `unsafe`
(raw pointers into a shared backing store, hash tables of indices) behind
a **100%-safe, Miri-checked API** to express things safe Rust can't say
cheaply — chiefly *a map whose key is borrowed from its value* and
*one item indexed by several keys at once*. The question: does anything
in patch-prolog match that shape well enough to be worth a learning PoC?

Answer: **yes, one clean fit (the interner), one real-perf-but-high-cost
fit (the runtime heap), and one that maps to a *future* feature, not
today's code (multi-index → fact-table compilation).** Ranked below.

**Guiding bar — learning is the motive, production-viability is the
filter.** The point of a PoC here is to learn, but we only spend effort
on candidates that have a *real chance of graduating to production* if
they prove out. "Graduating" means: same public API (no caller churn),
no weakening of the system's trust (Miri-clean, differential-proven
equal to the safe path, every existing test still passing on its own
terms), and a measured win that justifies the bytes/risk. A candidate
that can't clear that bar even in principle is interesting reading, not
work we schedule. If a PoC proves out it ships; if it doesn't, it is
**reverted/shelved, not parked half-in** — we don't accumulate
feature-gated curiosities that erode confidence in the codebase. The
learning is banked either way; the tree stays clean.

## The candidates (ranked for a *learning* PoC)

1. **String interner — the direct `iddqd` analogue. RECOMMENDED.**
   `crates/shared/src/interner.rs` is today `to_id: HashMap<String,
   AtomId>` + `to_str: Vec<String>` — every atom name is stored **twice**
   and the key is cloned on every `intern()`. The textbook unsafe fix
   (cf. the `string-interner` crate's bucket backend) is an arena of
   `String` chunks plus `HashMap<&'static str, AtomId>` where the `&str`
   is lifetime-laundered from the arena: store the bytes once, key by a
   borrow into the value. This *is* iddqd's borrow-key-from-value, in
   miniature. Verdict: best learning target — small, self-contained,
   swappable, the invariant ("arena chunks never move/free while a key
   points in") is exactly what Miri checks, and we can keep the safe
   impl and differential-test the two for equality.

2. **Runtime cell heap / unify walk — real perf, high verification cost.**
   `crates/runtime/src/machine.rs` (`heap: Vec<Word>`, `trail`) and
   `unify.rs` walk tagged 64-bit cells by index, with a bounds check on
   every `deref`/`heap[i]` in the innermost backtracking loop (the M2
   gate ran ~12.5M such ops). `unsafe { get_unchecked }` and raw cell
   pointers are the classic WAM speed-up and *would* bite at scale.
   But: this is the one place where being wrong is silent memory
   corruption in **every shipped user binary**, the current safe version
   already clears every perf gate, and you **cannot Miri a clang-linked
   native binary** — so the verification story that makes #1 safe to
   learn from is absent here. Treat as "study WAM-level unsafe," never as
   default-on, and only behind a Criterion bench that must show a win.

3. **Multi-index map — maps to the *future* fact-table feature, not now.**
   iddqd's headline product (one item, many keys) is what you'd want for
   "clauses by `(functor,arity)` AND by first-arg." But this is an AOT
   compiler: there is **no runtime clause DB** — indexing compiles to an
   LLVM `switch`, and the compiler's clause store is a cold
   `BTreeMap<(AtomId,u32), Vec<Clause>>`. The genuine fit is the ROADMAP
   "fact-table compilation" item (100k+ ground facts → `.rodata` tables);
   if/when that lands, *that* is where a multi-index structure — possibly
   the `iddqd` crate itself, compiler-side — slots in. Noted, not now.

Folded in: there is no separate union-find to optimize (#2 covers the
binding walk), and "atom name `.to_string()` on every dispatch"
(solve.rs) is a safe `&str`-return fix, not an unsafe one.

## Constraints

- **Zero-dep rule is the default for `plg-shared`/`plg-runtime`, but
  negotiable case-by-case** (see ARCHITECTURE.md). `unsafe` is not a
  dependency, so a *hand-rolled* unsafe interner is allowed in shared
  outright. The `iddqd` crate is a heavier ask: default-deny in
  shared/runtime, **but explicitly on the table for the fact-table
  feature (#3)** if it clears the footprint/`ldd` gate or is scoped
  compiler-side. Not assumed out — measured.
- **Correctness is the stated ultimate purpose.** A PoC that can't be
  *proven* equivalent to the safe path doesn't belong near the runtime.
  Hence the bias to #1, where Miri + a differential test give that proof.
- **Footprint + standalone contract unchanged.** The 1.3M ceiling and
  libc/libm-only `ldd` check (`check-binary-contents`) must stay green.
- **Out of scope:** any default-on unsafe in shipped binaries; touching
  `=/2` occurs-check or ISO semantics; the fact-table feature itself.

## Approach (if #1 is pursued as a study)

Keep `StringInterner` as the safe baseline. Add `UnsafeInterner` behind a
`cfg(feature = "unsafe-interner")` (default off): bump-arena of boxed
`str` chunks + `HashMap<&'static str, AtomId>`. Same public API
(`intern`/`resolve`). Prove it out, don't ship it on:
- **Miri** in CI over the interner's unit tests (the whole point).
- **Differential test**: feed both interners the example corpus' atom
  stream; assert identical `AtomId` assignment and `resolve` output —
  the same equivalence-pinning discipline the project already uses for
  the v1 oracle corpus.
- **Criterion bench**: intern N distinct atoms + resolve; record the
  delta. A clear win + Miri-clean + differential-equal → graduate it
  (replace the safe impl behind the unchanged API). Anything short of
  that → bank the learning, **revert the branch**; do not leave a
  feature-gated second interner lying around.

## Domain Events

- **PoC interner added** → CI gains a Miri job + an interner differential
  test + a bench; default build unaffected (feature off).
- **`intern(name)` (unsafe path)** → arena append (only on miss) →
  `&'static str` key insert → `AtomId`. Invariant that must hold: a key
  outlives no arena chunk it borrows (chunks are append-only, freed only
  with the interner).
- **Equivalence test runs** → consumes both interners → fails the build
  on any id/resolve divergence (drift is a test failure, not latent UB).

## Checkpoints

1. `cargo +nightly miri test -p plg-shared --features unsafe-interner` —
   clean (no UB reported).
2. Differential test: safe vs unsafe interner agree on ids + `resolve`
   across the example corpus.
3. Criterion: intern/resolve delta recorded here. Win + checks 1–2 green
   → graduate (swap behind the unchanged API). Wash or loss → bank the
   learning and revert; nothing feature-gated lingers.
4. Default `just ci` untouched and green; footprint + `ldd` contract
   unchanged.
5. Negative check: deliberately shrink an arena chunk's lifetime → Miri
   goes red (proves the harness actually catches the unsoundness).
