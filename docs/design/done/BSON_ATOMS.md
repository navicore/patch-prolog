# Design: bson self-describing mode (`--atoms`)

Status: **decision recorded, not yet implemented.** A bson result optionally
carries the atom map inline so a host can fully decode term values (which are
atom-id-keyed TermBuf) from the single bson document — no second call, no file,
no external atom source. Keeps JSON (and any host format) out of the engine:
the host does the bson→X conversion using the inline map.

## Intent

A bson consumer that needs the actual term *values* (not just `count`/
`exhausted`) must resolve atom ids to names, and the atom names live in the
engine. The wasm path solves this with `plg_rt_atom_name` (the live host calls
in). The native bson path has no live call, so the map must travel **with the
result**. `--atoms` makes the bson envelope self-describing: it adds an `atoms`
array the consumer looks up term atom-ids against, in the same stdout document
as the solutions.

Two forms: **with a query** (`--query ... --atoms`) embeds the post-query map
(covering query-introduced atoms) inside the result envelope; **standalone**
(`--atoms` with no `--query`) emits just the program atom map (no query runs, so
no query-introduced atoms — the boundary for the one-shot-fetch use case).

## Constraints

- **stdin/stdout model.** One stream per invocation; `--format` controls the
  wire format. The atom map rides **inside** the bson result — no sidecar file,
  no second call.
- **Opt-in.** Default bson stays dense (opaque atom ids). `--atoms` is the
  self-describing mode a decoder opts into; consumers reading only
  `count`/`exhausted` pay nothing.
- **No JSON in the engine.** The map is data (atom names); the host does all
  conversion (cell walk + render to JSON/YAML/whatever).
- **Post-query coverage.** The map is emitted as part of the result, so it
  includes atoms the query introduced (ids ≥ the program table) — no stale-map
  or query-atom gap.
- **bson-only.** `--atoms` with `--format text` is a silent no-op (text already
  renders names; `--atoms` would be noise).

## Approach

`--atoms` is a boolean modifier on the bson output. When set, the bson envelope
gains an `atoms` field: the post-query interner (`Machine.atoms`, ids
`0..len`) serialized as a bson array of name strings (id = array index). The
consumer reads the array once and resolves term atom-ids by index.

## Structure

### Module / file boundaries

- **`crates/runtime/src/wire.rs`** — `Envelope` gains `pub atoms:
  Option<&[String]>`; `PLG_ENC_BSON`'s writer emits the `atoms` bson array
  (strings, id order) when `Some`. Reuses the existing bson emitter helpers.
- **`crates/runtime/src/entry.rs`** — parse `--atoms`; when set and the output
  is bson, build the `Vec<String>` from `Machine.atoms` (post-query) and set it
  on the envelope. With `--format text`, `--atoms` is a silent no-op.
- **`crates/runtime/src/reactor.rs`** — unchanged (`atoms: None`). The wasm
  path keeps `plg_rt_atom_name`; convergence is a possible later step.

### Public interface

```rust
// wire.rs — the envelope gains an optional inline atom map
pub struct Envelope<'a> {
    pub count: usize,
    pub exhausted: bool,
    pub solutions: &'a [RenderedSolution],
    pub program_output: Option<&'a str>,
    pub atoms: Option<&'a [String]>,   // NEW: id → name, id = index
}
```
CLI: `--atoms` (boolean; bson-output modifier; no-op on text).

### Data shape

- **bson envelope with `--atoms`:** `{count: int32, exhausted: bool, output?:
  string, atoms: [<name0>, <name1>, …], solutions: [ {<var>: BinData, …}, … ]}`
  (field order: count, exhausted, output, atoms, solutions — `atoms` sorts
  between `output` and `solutions`, mirroring where it would sit alphabetically).
- The `atoms` array is the full post-query interner; element `i` is the name of
  atom id `i`. The consumer ignores entries it doesn't reference.

## Domain Events

- *Query with `--atoms`* — solve, then materialize `Machine.atoms` (ids
  `0..len`, post-query — includes any atoms the query introduced) into the
  `Vec<String>`, set `Envelope.atoms`, serialize bson with the `atoms` field.
- *Decode* — the host parses the bson, reads `atoms[id]` to resolve each term's
  atom ids, walks the TermBuf cells, renders to its target format.

## Checkpoints

- `./prog --query 'parent(tom,X)' --format bson --atoms` → bson with an `atoms`
  array; `bq` (or a host) decodes it and resolves `X`'s atom id to `"bob"`
  using the inline array — no external atom source.
- A query that introduces an atom (`X = f(X)` under `--atoms`) → the `atoms`
  array includes `f` (post-query), so the term decodes fully (cycle still cuts
  to `"_"` per the TermBuf contract).
- Default bson (no `--atoms`) → unchanged, no `atoms` field, dense.
- `--atoms --format text` → identical to `--format text` (no-op).
