// Reactor smoke harness (Tier 2, docs/design/done/WASM_TIER2_PLAN.md). Instantiates
// a `--target worker` module under Node's V8 — the same engine Cloudflare
// Workers use — and round-trips ONE query through the buffer ABI.
//
// IMPORTANT: the marshalling itself is NOT duplicated here. This harness imports
// the *emitted* `reactor.mjs` sitting next to the module — the exact file
// `worker.js` deploys — so `just wasm-reactor-smoke` exercises the shipped code,
// not a parallel copy. The harness only does the host-specific parts Node and
// workerd don't share: read the bytes, instantiate, and print the result.
//
// Usage: node reactor-smoke.mjs <module.wasm> <query> [step_limit]
// Prints the raw JSON result to stdout; exits 2 if an export is missing.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [, , wasmPath, query, stepArg] = process.argv;
if (!wasmPath || query === undefined) {
  console.error("usage: node reactor-smoke.mjs <module.wasm> <query> [step_limit]");
  process.exit(64);
}

// The shipped ABI marshalling, imported from the emitted glue next to the wasm.
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

process.stdout.write(
  runQuery(instance.exports, query, { stepLimit: BigInt(stepArg ?? "0") }),
);
