# Design: Plural wire encodings (text | bson)

Status: **implemented as `text` + `bson`, no JSON** (issue #38). The engine
speaks two wire encodings — `text` (readable, default) and `bson` (binary,
TermBuf-in-BinData) — gated by a codegen-baked capability table
(`:- io_format([...])`, default `[text, bson]` — both core formats available
out of the box; the directive is opt-out, to restrict) with dead-stripping. JSON is **not**
an engine format; a host wanting it derives it from bson at the host boundary
(the WASM reactor emits bson; its host glue decodes bson→JSON). The body below
is the original design record — it predates the JSON-removal decision and still
references json; read it as history. The authoritative current contract is in
ARCHITECTURE.md and RUNTIME_ABI.md.

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
- **Losslessness is non-negotiable.** Any encoding round-trips the full term
  space, including cyclic terms (`X = f(X)`) and improper lists (`[a|b]`).
- **Footprint rule.** No serde/clap in `plg-runtime`; encoders are hand-rolled.
  Dead-stripping (`--gc-sections`/`-dead_strip`, already load-bearing) drops
  encoders the binary doesn't advertise.
- **Immutable-binary thesis untouched.** Input is a query against a fixed
  program, never runtime KB mutation.
- **Wire contract preserved for text.** The default `[text]` capability is
  byte-identical to today's JSON output; existing harnesses keep working.
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
```

### Data shapes

- **`Envelope`** — single source of the engine-output shape; every `Encoder`
  reads the same fields. `program_output` is `Some` only in capture mode
  (bson); `None` in stream mode (text), preserving the v1 byte contract.
- **Capability table** — codegen-baked `.rodata`: the set from
  `:- io_format([...])`, default `[text]`. `--format` outside the set → exit 2.
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
- *Request arrives* — caller passes `--format <cap>` (validated against the
  table) and, for bson input, a one-field request document on stdin. Engine
  parses the inner query string; the wrapper is transport framing only.
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
  same solutions as the equivalent `--query` argv call.
- Footprint gate (`tests/binary_size.rs`) and standalone-contract `ldd` check
  stay green for both text-only and bson-capable binaries.
