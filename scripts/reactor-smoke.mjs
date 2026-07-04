// Reactor smoke harness (Tier 2, docs/design/WASM_HOST_GLUE.md). Instantiates a
// `--target worker` module under Node's V8 — the same engine Cloudflare Workers
// use — and exercises the bson→JSON host glue.
//
// IMPORTANT: the marshalling/decode is NOT duplicated here. This harness imports
// the *emitted* `reactor.mjs` sitting next to the module — the exact file
// `worker.js` deploys — so `just wasm-reactor-smoke` exercises the shipped code,
// not a parallel copy. The harness only does the host-specific parts Node and
// workerd don't share: read the bytes, instantiate, and assert/print.
//
// Two modes:
//   node reactor-smoke.mjs <module.wasm>
//     Fixture mode: run a battery of known queries against the module and assert
//     the host-produced JSON. The fixtures are pinned to examples/deps.pl (the
//     program the smoke recipe always builds); the recipe is the only caller.
//     Exits non-zero on any mismatch.
//   node reactor-smoke.mjs <module.wasm> <query> [step_limit]
//     Single-query mode: run one query, print the JSON. Used for the
//     constant-stack V8 check, where the recipe asserts the output itself.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [, , wasmPath, query, stepArg] = process.argv;
if (!wasmPath) {
  console.error("usage: node reactor-smoke.mjs <module.wasm> [query] [step_limit]");
  process.exit(64);
}

// The shipped ABI marshalling + bson decode, imported from the emitted glue next
// to the wasm. This is the whole point of the harness: it tests the deployed
// reactor.mjs, not a copy.
const abiUrl = pathToFileURL(join(dirname(wasmPath), "reactor.mjs"));
const { runQuery, assertExports } = await import(abiUrl);

const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
try {
  assertExports(instance.exports);
} catch (e) {
  console.error(e.message);
  process.exit(2);
}
instance.exports.plg_init();
const ex = instance.exports;

// Single-query mode: print the JSON (the recipe asserts).
if (query !== undefined) {
  process.stdout.write(runQuery(ex, query, { stepLimit: BigInt(stepArg ?? "0") }));
  process.exit(0);
}

// Fixture mode: known queries → expected host-produced JSON. No native
// differential (native no longer emits JSON); these pin the bson→JSON decode.
const fixtures = [
  [
    "needs(app, X)",
    `{"count":5,"exhausted":true,"output":"","solutions":[{"X":"auth"},{"X":"ui"},{"X":"crypto"},{"X":"render"},{"X":"crypto"}]}`,
  ],
  [
    "depends_on(app, D)",
    `{"count":2,"exhausted":true,"output":"","solutions":[{"D":"auth"},{"D":"ui"}]}`,
  ],
  [
    "shared_deps(auth, render, Ds)",
    `{"count":1,"exhausted":true,"output":"","solutions":[{"Ds":["crypto"]}]}`,
  ],
];

let fail = 0;
for (const [q, expected] of fixtures) {
  const got = runQuery(ex, q);
  if (got === expected) {
    console.log(`✅ ${q}`);
  } else {
    console.log(`❌ ${q}\n   expected: ${expected}\n   got:      ${got}`);
    fail = 1;
  }
}
process.exit(fail);
