# The clang Dependency (ADR)

**Status: accepted (status quo), 2026-06-04. Raised by the loglings
migration: is it acceptable for plgc to require user-installed clang?**

## Decision

Yes — plgc requires clang ≥ 15 on the machine that RUNS plgc, and this
is the normal position for a native-code compiler of this kind. The
dependency never extends to compiled binaries (they need only
libc/libm — enforced by `tests/binary_size.rs`).

## Why this is normal

Industry spectrum of self-containedness:

| Approach | Examples | System toolchain needed |
|---|---|---|
| Own everything | Go; Zig (bundles clang+lld) | none |
| Bundled codegen, system link driver | Rust, Swift | `cc` for linking |
| Emit IR/asm, system toolchain finishes | GNU Prolog (`gplc`→gcc), OCaml (`ocamlopt`→as+cc), GHC, patch-seq, **plgc** | compile + link |

The key reference points:

- **Rust itself needs a system C toolchain on Linux** — a fresh box
  fails with ``linker `cc` not found`` until build-essential is
  installed. Rust bundles LLVM for codegen but shells out to `cc` to
  drive linking (crt objects, libc, linker discovery). Users rarely
  notice only because dev machines already have one.
- **GNU Prolog**, the closest analog (Prolog → native), has invoked the
  system C compiler for 25 years.
- plgc is one notch stricter than Rust's "any cc": it needs **clang
  specifically**, because it emits LLVM IR text (gcc cannot consume
  `.ll`) and its constant-stack guarantee depends on `musttail`.

## The audience nuance (loglings)

The dependency falls on whoever runs `plgc` — for the linter use case
that's one CI machine; for loglings it's every learner. The mitigation
is documentation plus a good failure message (below): the install is
one `apt install clang` / `dnf install clang` /
`xcode-select --install`, the same class of hurdle Rust beginners hit
with build-essential.

## Rejected alternatives (revisit if learner friction proves real)

1. **Link LLVM into plgc** (llvm-sys/inkwell): removes
   clang-for-codegen but plgc grows to 100+ MB, builds pin to an LLVM
   release, and a system `cc` is STILL required to drive linking —
   Rust's exact position, at high cost for half the dependency.
2. **Emit C instead of IR** so any cc works: forfeits `musttail`
   except on clang/very-recent gcc — gives up the constant-stack
   design for portability we don't need.
3. **Bundle clang+lld wholesale** (the Zig route): correct and
   enormous; out of proportion for this project.

## Failure UX

`check_clang_version` runs before any link and fails with an
actionable, per-OS install hint (debian/fedora/macOS). plgc never
invokes cargo or rustc (LESSONS_FROM_V1.md rule 2); clang is the single
external tool, checked exactly once per process.

## Production threat model (the DMZ principle)

Raised 2026-06-04: hardened production machines should not carry
general-purpose compilers/toolchains ("don't leave a room full of guns
and ammo for an intruder"). The production use case is technical users
frequently recompiling as the facts of their world change.

**Chosen architecture: the immutable binary is the ONLY artifact in
production.** (User conviction, and a strong one: a single sealed
artifact means one hash to attest, no facts file to tamper with, no
connection string to leak; the facts' provenance IS the binary's
provenance; rollback = redeploy an old binary; "what did we believe at
time T" has a cryptographic answer.) Under this architecture the two
concerns compose cleanly:

- Compilation happens on a BUILD HOST (the armory — toolchains belong
  there); prod receives only sealed binaries. The clang dependency
  never reaches hardened machines at all.
- Fact churn becomes deploy cadence: facts change → rebuild → attest →
  ship. plgc builds in ~1s for normal programs, so minutes-scale churn
  is a boring pipeline.

Contingencies, in order:

1. **Fact-dense compile speed** (serves the conviction): large fact
   bases currently compile slowly (every ground fact is a clause
   function for clang). Future optimization: compile ground-fact
   predicates to static data tables in `.rodata` with a generic
   indexed lookup — same semantics, same single immutable binary,
   near-instant rebuilds at 100k+ facts. This is the aligned answer to
   high fact churn; tracked in ROADMAP "Future".
2. **The Zig route** (bundle the backend into plgc) only if compiles
   must happen ON hardened boxes (unusual under this architecture).
   Security note in its favor vs clang-on-PATH: an embedded backend is
   reachable only through the Prolog subset (no FFI, step-limited,
   stdout-only) — domain-specific, not general-purpose.
3. **Runtime fact loading** (`--facts`, ground terms into the dynamic
   registry) is REJECTED for this production shape: it reopens the
   mutable-file attack/audit surface the architecture exists to close.
   It remains on the table only for other consumers (e.g. a future
   loglings need), never as the prod default.
