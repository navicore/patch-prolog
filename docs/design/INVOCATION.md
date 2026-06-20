# Design: Invoking compiled binaries (deployment contexts)

Status: **exploratory decision record** — no milestone yet; to be scoped later
against examples / POCs. Captures *how a host should invoke a prebuilt
patch-prolog binary* across likely deployment contexts, and why FFI is
deliberately off the table for now. (WASM is a separate axis — its own doc.)

## Intent

Decide the most efficient *robust* way to run a compiled program in the
contexts we actually expect — an HTTP API endpoint, a k8s cron/batch job, a
Cloudflare service — without adding complexity ahead of a measured need.

## Frame: three independent axes

"Invoke efficiently" decomposes into three choices that are easy to conflate:

- **Process model** — one-shot exec / resident process / in-process (FFI)
- **Transport** — argv+stdout / stdin-or-socket line protocol / C ABI
- **Marshalling** — JSON text (current) / binary / native tagged words

The most expensive thing imaginable — running clang per request — is *already*
gone, because the binary is prebuilt. We are optimizing the cheap end, so the
bar for adding coupling is high.

## Key fact: process-per-request is strong here, not a compromise

A prebuilt native binary spawned per request is **not** in the class of a
Python/Node interpreter cold-start. CGI was slow because each request forked an
*interpreter* and reparsed the script — exactly the cost AOT removes. Per-request
cost here is `fork/exec` of a static binary (~1ms) plus rebuilding the atom
name→id map (`entry.rs`), which is over *distinct atom names*, not facts —
modest even at 100k facts. It scales better than it looks:

- **Memory under concurrency**: N concurrent requests are N processes of the
  *same* binary, so `.rodata` fact tables are COW/page-cache shared. Concurrency
  costs heap+stack per process, not a fresh knowledge base each time.
- **Blast radius**: the step limit (default 10k) + immutable program mean a
  runaway query kills one process, nothing else. Crash isolation is free.

This is also the model that matches the immutable-binary thesis: the running
process holds no mutable state, same as the artifact.

## Context → natural model

- **k8s cron / batch — one-shot CLI. Already perfect.** The binary *is* the job;
  one exec, startup amortized over the run. FFI would be pure downside.
- **HTTP API endpoint — the only context where invocation cost matters**, and the
  default is still **spawn-per-request**: stateless, crash-isolated, horizontally
  scalable, footprint-safe. Add a **resident query mode** (binary stays up,
  reads newline-delimited queries on stdin or a unix socket, program + atom map
  resident) *only if measurement* shows fork/exec or atom-map rebuild dominating
  at the target QPS. Resident mode keeps the process boundary and is a tiny line
  protocol — **not** an embedded HTTP server. HTTP/TLS stays in the host, any
  language; putting a web server in the runtime would blow the footprint rule.
- **Cloudflare — splits in two:**
  - *Containers* → runs the native ELF → identical to the k8s story.
  - *Workers* → V8 isolates, JS/WASM only; a native binary cannot exec there at
    all. Not an FFI question — a **WASM target** question. Deferred to the WASM
    design doc.

## Decision: FFI is out (for now)

FFI is the highest-coupling, lowest-latency option and the wrong fit for exactly
the contexts that already "just work" (cron, serverless) — those want a
*process*, not a linked library. It earns its complexity only for a
latency-critical, co-located **Rust** host that can't tolerate even a unix-socket
round-trip (the `RUST_EMBEDDING.md` stack). Keep it gated behind a measured need;
the cheaper resident-mode lever comes first.

## Priority ladder

1. **One-shot CLI** — covers cron, batch, Cloudflare Containers today. (Have it.)
2. **Benchmark spawn-per-request startup** — cold-start ms vs program size. This
   single number decides whether HTTP needs anything more.
3. **Resident line-protocol query mode** — only if (2) says startup hurts. Still
   a process; language-agnostic; footprint-safe.
4. **FFI / C ABI** — last, scoped to the same-process Rust-host case.
5. **WASM target** — its own track (edge / scale-to-zero), orthogonal to 1–4.

## Constraints / what must not break

- **Footprint rule**: nothing heavy enters `plg-runtime` (no HTTP/TLS/serde). A
  resident mode is a minimal line protocol, hand-rolled, or it doesn't ship.
- **Immutable-binary thesis**: invocation never reopens runtime clause mutation;
  it exposes querying a fixed program only.
- **Wire contract preserved**: `--query`/`--limit`/`--format`, exit codes, JSON
  shape stay byte-compatible (existing harnesses).

## Domain events

- *Request arrives* → host either `exec`s the binary (one-shot) or writes a query
  line to a resident process → `{solution emitted}* → exhausted | error` →
  host maps to HTTP response. The resident path replaces process-exit/exit-code
  with end-of-stream framing on the same JSON shape.

## Checkpoints

- A spawn-per-request benchmark reports cold-start ms across small / 10k / 100k-
  fact programs — the number that gates step 3.
- An HTTP POC (any host language) answers requests via one-shot exec, byte-
  matching the `--query` JSON path.
- If resident mode is built: a line-protocol POC answers N queries in one process
  with zero per-query startup, output byte-equivalent to one-shot.

## Recommendation

For three of four contexts the binary is already the right unit — add no seam.
The only context where cost matters is the HTTP endpoint, and even there
spawn-per-request is the correct default until a measured number forces the
resident-mode step. Do meaningful work on process-per-request now; revisit when
a scaling signal — not a guess — appears.
