## Spans (frontend → LSP → runtime errors)

**Status: Layers 1–2 implemented; Layer 3 (runtime provenance) pending.**
Replaces the buffer-scan / string-trailer hacks the diagnostics path
leaned on. Layer 1 (frontend `Span`/`Spanned`/`SourceMap`, structured
`ParseError`, tokenizer byte offsets) and Layer 2 (LSP consumes spans
directly — both `parse_at_line_col` and `call_site_ranges` deleted;
`plgc`'s `format_parse_error` resolves position from the span) are done,
single-buffer. The remaining work is Layer 3 below. The lint call-site
squiggle is realized via parser-recorded atom-functor occurrences
(`CallSite`) rather than a fully spanned AST — same user-visible result
(squiggles on real calls, never on comment text), no codegen ripple.

## Intent

Make source positions a first-class property of parsed AST so that
both **compile-time diagnostics** (LSP, `plgc check`) and
**runtime error messages** can name the exact file:line:col where a
goal lives. We are willing to pay a small frontend cost and a small
fixed footprint cost in the compiled binary (a static side-table) to
get a correct foundation. We are NOT willing to depend on DWARF — `-O3`
release builds can lose source-level debugger stepping; what must
survive is the textual error message printed by the runtime.

## Constraints

- `plg-shared` and `plg-runtime` zero-heavy-deps rule still holds; a
  `Span` value type is fine but no new crates.
- Wire contract is preserved: exit codes 0/1/2/3 and the JSON solution
  shape don't change. Runtime error *text* gains an optional `at
  file:line:col` suffix; harnesses currently substring-match on the
  ISO formal term and the `Context` atom — both stay byte-identical.
- ISO error term shape (`error(Formal, Context)`) is unchanged. Source
  location goes into the rendered top-level message and an optional
  third element of the Context (atom) or as a sibling line, NOT into
  the Formal — `catch/3` semantics must not change.
- `Term`/`Clause` live in `plg-shared` and are used by the parser, the
  lint, codegen, and tests. Adding a span must not break the existing
  `PartialEq`-on-`Term` uses (most are tests; the few semantic uses
  compare functor/args, not bodies — verify before the API breaks).
- Out of scope: DWARF `DILocation` emission; debugger source stepping
  at `-O3`; spans inside the runtime's own machine `Term`/`Cell`
  values (the heap term reps stay 8-byte tagged words).

## Approach

Three additive layers.

**1. Frontend carries spans.** Introduce `plg_shared::Span { file: FileId,
lo: u32, hi: u32 }` (byte offsets into the source text; line/col is
resolved from a `SourceMap` at format time). Parser produces
`Spanned<Term>` for body goals and clause heads and a `ClauseSpan` for
each `Clause`. `ParseError` becomes a struct `{ message: String, span:
Span }` — the `... at line N col M` trailer is dropped. `Parser`
returns a `SourceMap` alongside the clauses.

**2. LSP consumes spans directly.** `diagnostics.rs` deletes
`parse_at_line_col` and `call_site_ranges`. Parse errors map
`ParseError.span` to `Range` through the SourceMap. Undefined-call
warnings carry the call site's `Span` from the lint (the lint walks
spanned terms now), so squiggles land on the actual AST node — not on
every text match of the callee name.

**3. Runtime error provenance via a side-table.** Codegen emits a
single read-only global per binary:

```llvm
@plg_srcmap = [N x { i32 file_id, i32 line, i32 col }]   ; site id → location
@plg_files  = [F x ptr]                                   ; file_id → filename literal
```

At each call site that can raise (compiled predicate-call sites,
arithmetic, type-checking builtins, `throw/1`), codegen passes a
**site id** (u32, the index into `@plg_srcmap`) as an extra argument
or via a per-frame slot. The runtime error constructors
(`existence_procedure`, `type_error`, `evaluation`, …) gain a
`site_id` parameter and append `" at <file>:<line>:<col>"` to the
rendered message. The ISO term shape is untouched.

The site-id slot is one i32 per active frame — negligible vs the cell
heap, and `-Wl,--gc-sections` strips the tables for binaries that
never raise the corresponding error class.

The 8-byte `Span` lives only in compiler-side `Term` — the runtime's
machine cells are unchanged.

## Domain Events

- **Produced**: `ParseError { span }` (structured), `LintFinding { span }`
  (call-site precise), runtime error text with `at file:line:col`
  suffix.
- **Consumed**: LSP `diagnostics::compute` consumes spans directly
  (no regex). `plgc check` prints `file:line:col: <msg>` from the
  Span+SourceMap, not the embedded trailer. Compiled binary's stderr
  on `existence_error` / `type_error` / `evaluation_error` /
  `throw/1` gains the location suffix.
- **Side-table emission**: codegen writes `@plg_srcmap` + `@plg_files`
  for the linker; `--gc-sections` removes unused entries.

## Checkpoints

1. **Frontend** ✅ — parser tests assert on the structured
   `ParseError`/`.span` (no "at line N col M" string left); span byte
   ranges are pinned for the unexpected-token and EOF cases
   (`parse_error_span_*` in `parser/query/tests.rs`).
2. **LSP** ✅ — `diagnostics::compute` no longer calls
   `parse_at_line_col` or `call_site_ranges` (both deleted). The
   comment-vs-call test (`comment_mention_does_not_squiggle_only_the_real_call`)
   confirms exactly one squiggle, on the call.
3. **Runtime**: `existence_error` on `family.pl:12` (an undefined call
   in a clause body, deliberately introduced) prints
   `... Undefined procedure: foo/1 at examples/family.pl:12:7` on
   stderr. Existing oracle-bytes tests still pass — the ISO `error/2`
   term rendering is unchanged; only the post-term context line
   gains the suffix.
4. **Footprint**: hello-world default-build (no source loc errors
   reachable) stays under the 1.3M ceiling enforced by
   `tests/binary_size.rs` — `--gc-sections` proves the side-table is
   pay-for-what-you-use.
5. **Differential**: `just diff-test` continues to match the v1 oracle
   modulo the new `at file:line:col` suffix (the oracle never emitted
   it). Diff helper learns to strip the suffix when comparing.
