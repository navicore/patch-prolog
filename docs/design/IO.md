# Design: Plural wire encodings (text | bson)

Status: **partially implemented.** Output is done: the `Envelope`/`EncoderDesc`
seam, the json + bson encoders (TermBuf-in-BinData), and the capability table
(`:- io_format([...])` gating `--format`, with dead-stripping). Still pending:
bson **input** (`--input-format`, the one-field request document) and the CLI
rename (`json`→`text`, human form→`--pretty`). Addresses issue #38.

## Intent

Today every compiled binary speaks JSON on stdout, force-fed. JSON is a poor
wire format for a program that moves real volume, and the author has no agency
over it. This change separates the **envelope shape** (fixed: the engine's
contract — solutions, count, exhausted, output) from its **encoding**
(plural), and lets a program declare which encodings its binary advertises.
Two encodings cover the space: **text** (the current JSON bytes, human- and
machine-readable) and **bson** (binary, typed, length-framed, dense). The
author picks the set; the caller picks within it.

## Constraints

- **stdin/stdout only.** No new I/O channels. Portable Unix contract unchanged.
- **Envelope shape is fixed.** `count`, `exhausted`, `solutions[]`, optional
  `output` — the fields the engine emits. Only their *encoding* pluralises.
- **Exit codes unchanged.** `0` no solutions · `1` solutions · `2` parse/usage
  error · `3` runtime error.
- **Losslessness — asymmetric by encoding, stated honestly.** The two
  encodings are *not* information-equivalent on cyclic terms.
  **bson** is fully lossless, including cyclic terms (`X = f(X)`) and
  improper lists (`[a|b]`) — `copyterm::TermBuf` represents cycles
  structurally (memoized copy, no divergence). **text** is lossy on cyclic
  terms by design: `render.rs` cuts a re-encountered subterm to `"_N"`
  (v1's `apply()` behavior, preserved for byte-compat), so `X = f(X)` reads
  back as `f(_N)` and does not round-trip. Acyclic terms round-trip fully
  in both. This asymmetry is a deliberate point in bson's favor, *not* a
  defect to fix — making text lossless on cycles would break the v1
  byte-contract. The cyclic-term round-trip checkpoint is therefore scoped
  to **bson only**; text is exempt under the v1-byte-compat carve-out.
- **Footprint rule.** No serde/clap in `plg-runtime`; encoders are hand-rolled.
  Dead-stripping (`--gc-sections`/`-dead_strip`, already load-bearing) drops
  encoders the binary doesn't advertise.
- **Immutable-binary thesis untouched.** Input is a query against a fixed
  program, never runtime KB mutation.
- **Wire contract preserved for text.** The default `[text]` capability is
  byte-identical to today's JSON output; existing harnesses keep working.
  The default input path (argv `--query`) is also byte-identical and never
  reads stdin — the v1 no-stdin contract holds for the default invocation.
- **`plg-shared::cell` stays the single source of the word/cell ABI.**

## Approach

One envelope shape, two encodings, selected by a capability table the author
declares and the caller requests. Text streams (v1-preserved); bson captures
`write/1` into the envelope (binary formats physically cannot interleave with
raw text bytes — the encoding dictates the sink, no new flag needed). The
existing `Machine::OutputSink` (Stdout | Capture) is the lever.

Input is **one-field transport framing**, symmetric to output: a bson request
document is `{"query": "<our query syntax as a string>"}` (plus optional
`limit`/`format`). The engine parses the inner string exactly as today — no
bson-term loader, no parallel parser path. Justification: input isn't
user-authored structure in this architecture; the only entry point for user
logic is the goal term, and the engine owns constructing it. Wrapping a query
string in bson gives the caller a binary, typed, length-framed transport
without buying parser complexity on our side (the Event Hubs / Parquet
precedent — except here the string *is* the natural unit; there's no second
parse, so the cheat is honest).

**Input/output orthogonality.** Input and output encodings are fully
orthogonal — any `{in}×{out}` pairing the capability set permits is allowed.
The common case `--query "goal(X)" --format bson` (text-in / bson-out: a
human types a query, a machine consumes the dense result) is explicitly
supported. The capability table is **one set, gating both directions**: a
`[text]`-only binary accepts neither bson input nor bson output. Input
encoding defaults to text (argv `--query`); bson input is opt-in via a flag
**separate from output `--format`** — `--input-format text|bson` (default
`text`). `entry.rs` reads stdin **only** when `--input-format bson` is passed;
the default path never touches stdin, preserving the v1 no-stdin contract.
Keeping the two flags distinct avoids the overload trap of "absent `--query`
⇒ read bson from stdin," where the absence of one flag would imply the
presence of another.

Terms inside bson output use **TermBuf-in-BinData**: each solution's term
values are encoded as the existing `copyterm::TermBuf` cell bytes (M9's format,
single-sourced in `plg-shared::cell`) wrapped in a bson `BinData(0x00)` field.
This reuses the one ABI the project already refuses to duplicate, is maximally
dense, and is lossless by construction. The trade-off is opacity to generic
bson tooling — a caller speaking bson to a patch-prolog binary has opted into
this engine's world. Tagged-document encoding (self-describing bson per term)
is reserved as a trait-shaped fallback only if a real caller needs
generic-bson introspection.

## Structure

### Module / file boundaries

- **New `crates/runtime/src/wire.rs`** — the typed `Envelope`, `WireError`,
  the `Encoder` trait, `encoder_for`, and the `Json` (text) + `Bson` impls.
  Replaces the JSON-specific writers currently in `core.rs`. Zero new deps.
- **`crates/runtime/src/core.rs`** — shrinks to: parse, solve, build an
  `Envelope` borrowing `m.solutions`, hand to a chosen `Encoder`. The
  `write_solutions_json` / `write_error_json` functions move to `wire.rs` as
  the `Json` encoder impl. `QueryResult`, `run_query`, `exhausted` stay.
- **`crates/runtime/src/entry.rs`** — argv parser picks `Encoder` by
  `--format`, validates it against the baked capability table (else exit 2),
  drives the encoder against the chosen sink. Stops knowing about JSON.
- **`crates/runtime/src/reactor.rs`** — already capture-mode; swaps the old
  JSON writer for `encoder.write_envelope`. Minimal; validates the seam for a
  non-stdout transport.
- **`crates/runtime/src/machine.rs`** — `OutputSink` unchanged. No new channel.
- **Codegen** — new small `.rodata` capability table (the declared
  `io_format/1` set) alongside the atom table and registry (M9 pattern).
  Codegen emits calls only to encoders in the set; dead-stripping drops the
  rest. No IR changes beyond this table + the directive parse.

Not touched: solve driver, `RUNTIME_DECLS`, the C-ABI surface, exit codes.

### Public interfaces

```rust
// wire.rs — the fixed envelope shape (the engine contract, as a type)
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],   // existing type from render.rs
    pub program_output: Option<&'a str>,      // captured write/1 bytes; None when streamed
}
pub enum WireError {
    Parse(String),
    Runtime(String),
}

// the plural encoding
pub trait Encoder {
    fn write_envelope(&self, w: &mut dyn Write, e: &Envelope) -> io::Result<()>;
    fn write_error(&self, w: &mut dyn Write, e: &WireError) -> io::Result<()>;
    fn can_stream(&self) -> bool;   // text=true (v1 streaming), bson=false (must capture)
}
pub fn encoder_for(name: &str) -> Option<Box<dyn Encoder>>;  // "text" | "bson"

// transport-framed input (one-field): {"query": "...", "limit"?: N, "format"?: "..."}
pub struct Request { pub query: String, pub limit: Option<usize>, pub format: Option<String> }
// entry.rs: --format selects output Encoder; --input-format text|bson (default
// text) selects the input path. Input/output are orthogonal; the capability
// table gates both directions. stdin is read ONLY when --input-format bson.
```

### Data shapes

- **`Envelope`** — single source of the engine-output shape; every `Encoder`
  reads the same fields. `program_output` is `Some` only in capture mode
  (bson); `None` in stream mode (text), preserving the v1 byte contract.
- **Capability table** — codegen-baked `.rodata`: the set from
  `:- io_format([...])`, default `[text]`. **One set, gating both input and
  output.** `--format` or `--input-format` outside the set → exit 2.
- **`Machine::OutputSink`** — invariant unchanged: in `Capture`, `write/1`
  bytes survive heap rewind and are available post-solve. bson forces Capture;
  text uses Stdout (default) or Capture (if the author/caller opts in later).
- **Term encoding in bson** — `BinData(0x00)` carrying `copyterm::TermBuf`
  bytes; lossless via the single-sourced `plg-shared::cell` ABI. Scalar
  envelope fields map to native bson types (`count`→int32, `exhausted`→bool,
  `output`→string).

## Domain Events

- *Compile time* — author writes `:- io_format([text, bson]).`; codegen bakes
  the capability table and emits encoder calls only for the declared set.
  Encoders not in the set are dead-stripped from the final binary.
- *Request arrives* — caller passes `--format <cap>` (output, validated
  against the table) and `--input-format text|bson` (default `text`, also
  validated). text-in reads `--query` from argv; bson-in reads a one-field
  request document from stdin **only in that mode**. The two encodings are
  orthogonal: e.g. text-in / bson-out is allowed. Engine parses the inner
  query string; the wrapper is transport framing only.
- *Solution emitted* — text mode: `write/1` bytes stream to stdout, then the
  envelope serialised as JSON follows (v1 byte-identical). bson mode: `write/1`
  bytes are captured into the envelope's `output` field, and the whole envelope
  serialises as one bson document.
- *Error* — encoded per the chosen encoder (`{"error":"..."}` for text, a bson
  error document for bson); exit code (2/3) unchanged.

## Checkpoints

- A `[text]`-only binary (default) answers `--query` byte-identically to
  today's JSON path — existing harnesses and the differential corpus unchanged.
- A `[text, bson]` binary: `--format bson` emits a valid bson document;
  `bsondump` reads the scalar fields; decoding the `BinData` term fields via
  `copyterm::TermBuf` round-trips a cyclic term and an improper list losslessly.
- `--format bson` on a `[text]`-only binary exits 2 with a usage error.
- A binary declaring `[bson]` only: `ldd`/symbol check confirms no JSON encoder
  linked (dead-strip verified); footprint drops vs a `[text, bson]` binary.
- bson input: a one-field `{"query":"..."}` document on stdin produces the
  same solutions as the equivalent `--query` argv call (text-in / bson-out and
  bson-in / text-out cross-pairings both verified).
- Footprint gate (`tests/binary_size.rs`) and standalone-contract `ldd` check
  stay green for both text-only and bson-capable binaries.
