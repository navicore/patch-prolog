# bson-decode

A Go program that demonstrates patch-prolog's bson wire format with `--atoms`:
it fetches the atom map from a compiled binary, runs a query as bson, and
decodes the term values (TermBuf cells) using that map — producing readable
output equivalent to `--format text`, but arrived at entirely through the
binary wire path.

This is the reference consumer: **no JSON in the engine, all decode is
host-side.** The engine speaks text + bson; the host does the rest.

## Prerequisites

- A compiled patch-prolog binary (via `plgc build`).
- Go 1.23+.

## Usage

```
plgc build examples/deps.pl -o /tmp/deps
cd examples/bson-decode
go run . /tmp/deps 'shares_dep(render, auth, D)'
```

Output:
```
fetched 30 atoms
count=1 exhausted=true
D = crypto
```

## How it works

1. **Fetch the atom map** — runs `./prog --atoms --format bson` once, parses the
   `{count, atoms: [...]}` bson document, caches the id→name array.
2. **Run the query** — runs `./prog --query '...' --format bson`, parses the
   `{count, exhausted, solutions}` bson envelope.
3. **Decode term values** — for each solution binding, the value is a bson
   `BinData(0x00)` carrying a TermBuf (the cell-format term). The program walks
   the cells using the cell ABI (`plg-shared::cell`: tag in low 3 bits, payload
   = word >> 3) and resolves atom ids against the cached map, rendering the term
   as a readable string.

The cell walker mirrors the wasm host glue (`reactor.mjs`'s `renderWord`) and
the cell ABI documented in `docs/RUNTIME_ABI.md`.

## Why this matters

`bq` (a general bson→json tool) correctly extracts the bson structure but
shows term values as opaque `$binary` base64 — it doesn't know the TermBuf cell
format. This example shows the full decode: `--atoms` provides the atom names
(the hard part — they lived in the engine), and the cell walker (the mechanical
part) turns `BinData` + `atoms[id]` into readable values like `crypto`.

This proves bson is a fully-decoding peer to text: any term text can show, bson
+ `--atoms` + a cell walker can show too.
