# Design: WASM Tier-2 host glue (reactor bson → JSON)

Status: **decision recorded, not yet implemented.** The Tier-2 reactor
(`--target worker`) emits bson; this design wires the host glue to decode it to
JSON for HTTP clients. All conversion logic is host-side (JS); plgc adds only
the one data export the host can't do without. Re-enables the gated
`wasm-reactor-smoke` (the temporary deferral from the IO.md work).

## Intent

The worker is an HTTP endpoint (Cloudflare Workers / `workerd`). Its clients
want JSON over HTTP. The engine speaks text + bson (no JSON —
docs/design/IO.md), so the reactor emits bson; the host glue turns bson into
JSON. This puts the bson→JSON burden entirely on host code (the user's stated
principle), and the only plgc-side change is exposing the atom table the host
must have to render term values.

## Why the host must decode (not pass bson through)

bson term values are `BinData(0x00)` carrying a `TermBuf` whose atoms are
**ids**, not names. Only the engine has the atom table; an HTTP client can't
call into the wasm module to resolve them. So the decode has to happen host-side,
where the atom table is reachable via the new export. Serving raw bson to
clients would ship opaque atom ids.

## Constraints

- **No JSON in plgc.** The engine stays text + bson. JSON exists only in the
  host glue (`reactor.mjs`) and any CLI-side utility a user adds later.
- **The reactor must keep emitting bson**, not text — the worker needs the full
  envelope including captured `write/1` `output`, which the text encoder drops.
- **The atom-table export is data, not JSON.** plgc exposes the table; the host
  does all conversion logic. Minimal ABI surface.
- **The JSON shape is host scaffolding, not an engine contract.** `reactor.mjs`
  is overrideable per deployment; the default shape is a starting point, freely
  editable. No backward-compat obligation (zero users).
- **The cell ABI is a documented cross-language seam.** The JS host re-implements
  the `plg-shared::cell` tag/payload layout; `wasm-reactor-smoke` is the drift
  catcher (mirrors how `RUNTIME_DECLS`↔`abi.rs` already couple across the IR/Rust
  boundary).

## Approach

One new reactor export + a host-side decode pipeline.

The reactor already emits the bson envelope (`wire::PLG_ENC_BSON`). The host
calls `plg_rt_atom_table()` once at instantiation, caches the id→name array,
then per request: bson bytes → bson parse → walk each term's `TermBuf` via the
cell ABI resolving atom ids → render native JSON.

## Structure

### Module / file boundaries

- **`crates/compiler/src/codegen/program.rs`** — emit a tiny `plg_rt_atom_table`
  function for the `Worker` target (alongside `plg_init`), returning the packed
  `(@plg_atom_strs, atom_count)`. No IR change to the atom table itself.
- **`crates/compiler/src/link.rs`** — add `plg_rt_atom_table` to
  `REACTOR_EXPORTS` so wasm-ld roots it.
- **`crates/compiler/src/worker_glue.rs`** — rewrite `reactor.mjs`'s `runQuery`
  return path (the `FIXME (wasm track)` site): bson decode + TermBuf walk +
  atom-id lookup + JSON render. Add `initAtoms(ex)`, called once, caching the
  id→name array from the export.
- **`scripts/reactor-smoke.mjs`** — assert host-produced JSON against
  hand-written fixtures for known queries (no native differential: native no
  longer emits JSON).
- **`justfile`** — un-gate `wasm-reactor-smoke`; restore the recipe body and the
  1,000,000-deep V8 `return_call` check.

Not touched: the reactor's bson emission (`reactor.rs`), the cell ABI
(`plg-shared::cell`), the CLI path.

### Public interface (the one ABI addition)

```js
// new reactor export, codegen-emitted alongside plg_init:
// plg_rt_atom_table() -> u64   packed (atom_strs_array_ptr << 32) | atom_count
//   atom_strs_array_ptr → [count x ptr] of null-terminated UTF-8 atom name cstrings, id order
```
Added to `REACTOR_EXPORTS`. `initAtoms` calls it once, reads `count` cstring
pointers, decodes each to a JS string, caches `atoms[id] = name`. Everything
else in this design is host-internal.

### Data shapes

- **bson envelope** (reactor output, unchanged): `{count:int32, exhausted:bool,
  output?:string, solutions:[{<var>: BinData(0x00, <TermBuf bytes>), ...}]}`.
- **TermBuf BinData payload**: `[ver:u8=1][cell_count:u32 LE][root:u64 LE]
  [cells:u64 LE…]`.
- **cell ABI** (`plg-shared::cell`, mirrored in JS — tag = word & 7, payload =
  word >> 3): `REF=0` · `ATOM=1`(payload = atom id → `atoms[id]`) · `INT=2`
  (signed 61-bit immediate, arithmetic `>> 3`) · `STR=3`(payload = buf idx →
  `[functor(=id<<32 | arity)] [args…]`) · `LST=4`(`[head][tail]`) · `FLT=5`
  (f64 bits) · `BIG=6`(i64). Cycles (a re-encountered STR/LST buffer index) cut
  to `"_"`, matching how text rendering handles them.
- **default JSON shape** (native mapping, host scaffolding): atom → `"foo"`;
  integer/float → number; proper list → `[...]`; compound →
  `{"functor":"f","args":[...]}`; improper list → `{"head":[...],"tail":...}`;
  unbound var → `"_"`. Envelope: `{"count":N,"exhausted":B,"output":"...",
  "solutions":[{"X":<term>,...}]}`.

## Domain Events

- *Instantiate* — host calls `plg_init`, then `plg_rt_atom_table`, caches the
  atom name array. One-time per isolate.
- *Request* — host calls `runQuery`; reactor returns bson bytes; host parses the
  envelope, walks each term's TermBuf resolving atom ids, renders JSON, returns
  it as the HTTP body.
- *Error* — the reactor's bson error document (`{error:"..."}`) decodes to a JSON
  error object; the host maps it to the HTTP status it wants (scaffolding choice).

## Checkpoints

- `just wasm-reactor-smoke` (re-enabled) green: host-produced JSON for
  `needs(app, X)` etc. matches fixtures; 1,000,000-deep `call/1` returns in a V8
  isolate via `return_call`.
- A cyclic term (`X = f(X)`) served through the worker renders without looping
  (cycle cut to `"_"`) — the JS walker mirrors the TermBuf's structural cycle
  handling.
- `nm`/exports check on the reactor module: `plg_rt_atom_table` is exported.
- Core `just ci` unaffected (wasm workflow is separate).
