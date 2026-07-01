# Architecture

patch-prolog compiles an ISO-subset Prolog program to a standalone
native binary. The compiled binary contains **no clause interpreter**:
predicates are native functions generated as LLVM IR; only primitive
services (heap, trail, unification, builtins, query parsing, output)
come from a runtime library statically linked into the binary.

## Pipeline

```
rules.pl ──parse──▶ AST ──analyze──▶ codegen ──▶ rules.ll (LLVM IR text)
                                                    │
        libplg_runtime.a (embedded in plgc) ──┐     │
                                              ▼     ▼
                          clang -O3 -g rules.ll -lplg_runtime -lm
                                              │
                                              ▼
                                   rules  (standalone binary)
```

- The compiler emits LLVM IR as **text** — no llvm-sys/inkwell binding,
  no LLVM version lock-in beyond "clang ≥ 15" (opaque pointers).
- `libplg_runtime.a` is built from `crates/runtime` and embedded into
  the `plgc` binary via `include_bytes!` (set up by
  `crates/compiler/build.rs`, which also enforces exact version match
  between compiler and runtime, and bakes in a content hash of the
  archive). At link time it is materialized at a content-addressed
  cache path (`$XDG_CACHE_HOME/plgc/runtime-<hash>/`, else
  `~/.cache/plgc/…`) that every run of the same build reuses; stale
  entries are age-swept. HOME-less environments fall back to a private
  per-link extraction that is removed after linking.
- `-Wl,--gc-sections` (Linux) / `-Wl,-dead_strip` (macOS) strips
  runtime code the program can't reach, keeping binaries small.
- Users of `plgc` need clang. Users of compiled binaries need nothing.

This is the architecture proven by patch-seq: a compiled binary contains
no clause interpreter. The rejected alternative — embedding a serialized
clause database inside a shipped interpreter — would put the whole
interpreter (and a Rust runtime) into every "compiled" program, and is why
this engine generates native code per predicate instead.

## Crates

| Crate | Artifact | Role |
|---|---|---|
| `plg-shared` | rlib | `AtomId` + well-known atoms, `Term`, `StringInterner`, `FirstArgKey`, operator table. Linked into BOTH compiler and runtime — zero dependencies, by rule. |
| `plg-frontend` | rlib | Tokenizer + operator-precedence parser + ISO error types (ported from v1). Compiler-side only. |
| `plg-runtime` | **staticlib** + rlib | The machine substrate compiled code calls into: heap/trail/choice points, generic unify, ~60 builtins, the minimal goal-only `--query` parser, JSON/text output, process entry. Ships inside every compiled binary. |
| `plg-compiler` | bin `plgc` + rlib | CLI, codegen (IR text emission), clang driver, runtime embedding. |
| `plg-lsp` | bin `plgl` | Language server (diagnostics, completion, hover, goto-definition). A frontend consumer — never links the runtime. |
| `plg-repl` | bin `plgr` | Interactive REPL that drives the compiler; never interprets. |

Dependency rule: nothing heavy (clap, serde, …) may enter `plg-runtime`
or `plg-shared`; every byte there lands in every user binary. (The
compiler-side crates — `plg-frontend`, `plg-compiler`, `plg-lsp`,
`plg-repl` — are dev tooling and carry no such constraint.) This is a
strong default, not an absolute: a dependency that demonstrably pays for
its bytes against the footprint gate, or is scoped to compiler-side crates,
can be considered.

## Execution model (summary)

The runtime substrate — cell heap, trail, choice points, tagged words,
generic unification, first-argument indexing, cut as choice-point-stack
truncation — is the **Edinburgh / Warren Abstract Machine (WAM) model**
[Warren, SRI Tech Note 309, 1983], the shared machine model behind
DEC-10/Quintus/SICStus/SWI/GNU Prolog. plgc inherits that machine model
and diverges from WAM only in the **codegen target**: where a classical
WAM system compiles Prolog to an instruction set (`get`/`put`/`unify`,
`call`/`proceed`) over a register file and a local-stack environment,
plgc compiles each predicate to one native LLVM function in
continuation-passing style. The data structures are WAM's; the
instruction set is replaced by direct native-code emission. See
[Compilation Model](compilation-model.md) for the why and the escape
hatches.

- Each predicate compiles to one LLVM function in continuation-passing
  style: it receives the Machine pointer, its arguments as tagged 64-bit
  words, and a success continuation. Solutions are delivered by
  `musttail`-calling the continuation; failure is a plain return.
- Alternatives (untried clauses, disjunction branches) live on a
  runtime-managed choice-point stack holding retry function pointers
  plus heap/trail marks. Backtracking = rewind marks + tail-call retry.
- Cut truncates the choice-point stack to the barrier recorded at
  predicate entry (stopping at catch frames).
- All transfers are tail calls and continuation frames are
  heap-allocated, so Prolog recursion depth never grows the C stack;
  determinate last-goal recursion is a true jump.
- Backtracking resets the heap top to the choice point's mark —
  memory reclamation without GC.

## Runtime `--query` support

The compiler bakes two global tables into every binary:

- the **atom table** (all atom names, in id order), and
- the **predicate registry** `{functor_id, arity, fn_ptr}`.

At startup the runtime rebuilds the name→id map from the atom table, so
a runtime-parsed query interns into the same id space (new atoms get
fresh ids that correctly unify with nothing in the program). The
registry maps a parsed goal to its compiled entry point — this is also
how `call/1` and `findall/3` re-enter compiled code. Predicates declared
`:- dynamic` with no clauses are registered to an always-fail stub
(silent-fail linter contract); unknown predicates raise
`existence_error(procedure, F/A)`.

## Wire contract (compiled binaries)

Preserved exactly from v1 so existing harnesses keep working:

- `--query "goal(X)"`, `--limit N`, `--format json|text`
- exit `0` no solutions · `1` solutions found · `2` query parse error ·
  `3` runtime error
- JSON: `{"solutions":[{"X": ...}], "count": N, "exhausted": bool}`

## Build system

`justfile` is the source of truth; CI (`.forgejo/workflows/ci-linux.yml`)
only calls `just ci`. Recipe ordering matters: `build` runs
`build-runtime` before `build-compiler` so the canonical
`target/release/libplg_runtime.a` is fresh when `build.rs` embeds it.
