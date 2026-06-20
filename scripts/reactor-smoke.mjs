// Reactor smoke driver (Tier 2, docs/design/WASM_TIER2_PLAN.md). Instantiates a
// `--target worker` module under Node's V8 — the same engine Cloudflare Workers
// use — and round-trips ONE query through the linear-memory buffer ABI:
//
//   plg_init()                          build the Machine (once)
//   plg_rt_alloc(len) -> ptr            host writes the query bytes
//   plg_rt_run_query(ptr,len,lim,steps,depth) -> (len<<32)|ptr   JSON result
//   plg_rt_free(ptr,len)               host frees the query / result buffers
//
// It first asserts the four host exports exist: under `wasm-ld
// --allow-undefined` a missing or renamed export degrades to a silent wasm
// *import* rather than a link error, so this assertion is what catches a broken
// export at build time. This is a TEST driver, not the deployable worker glue
// (that is productized in Phase D); it stays minimal on purpose.
//
// Usage: node reactor-smoke.mjs <module.wasm> <query> [step_limit]
// Prints the raw JSON result to stdout; exits 2 if an export is missing.

import { readFileSync } from "node:fs";

const [, , wasmPath, query, stepArg] = process.argv;
if (!wasmPath || query === undefined) {
  console.error("usage: node reactor-smoke.mjs <module.wasm> <query> [step_limit]");
  process.exit(64);
}

const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const ex = instance.exports;

const REQUIRED = ["plg_init", "plg_rt_run_query", "plg_rt_alloc", "plg_rt_free", "memory"];
for (const name of REQUIRED) {
  if (!(name in ex)) {
    console.error(`MISSING EXPORT: ${name}`);
    process.exit(2);
  }
}

ex.plg_init();

const queryBytes = new TextEncoder().encode(query);
const qptr = ex.plg_rt_alloc(queryBytes.length);
new Uint8Array(ex.memory.buffer, qptr, queryBytes.length).set(queryBytes);

// limit = 0 (unbounded); step_limit = stepArg or 0 (module default);
// depth_limit = 0 (module default). step_limit is i64 -> BigInt.
const packed = ex.plg_rt_run_query(qptr, queryBytes.length, 0, BigInt(stepArg ?? "0"), 0);
ex.plg_rt_free(qptr, queryBytes.length);

// Packed (len << 32) | ptr — the return is i64, so a BigInt in JS.
const len = Number(packed >> 32n);
const ptr = Number(packed & 0xffffffffn);
const result = new TextDecoder().decode(new Uint8Array(ex.memory.buffer, ptr, len));
ex.plg_rt_free(ptr, len);

process.stdout.write(result);
