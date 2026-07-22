// Cold-chain release-decision service — the REST contract layer.
//
// Copy this over the generated edge/worker.js after `plgc build --target
// worker` (the scaffolding worker.js is a raw query passthrough; it is never
// regenerated, so edits/copies are safe from rebuilds). The wasm module name
// below matches the build output; the tutorial walks through every piece.

import { runQuery, assertExports } from "./reactor.mjs";
import reactorModule from "./coldchain.worker.wasm";

let cached;
function reactor() {
  if (!cached) {
    const instance = new WebAssembly.Instance(reactorModule, {});
    assertExports(instance.exports);
    instance.exports.plg_init();
    cached = instance.exports;
  }
  return cached;
}

// JSON body → Prolog goal. Validating atoms against Prolog's atom syntax
// doubles as the injection guard — nothing else reaches the goal string.
const ATOM = /^[a-z][a-z0-9_]*$/;
const bad = (msg) => Object.assign(new Error(msg), { status: 400 });

function atom(value, name) {
  if (typeof value !== "string" || !ATOM.test(value))
    throw bad(`${name} must be a lowercase atom like 'basil'`);
  return value;
}
function num(value, name) {
  if (typeof value !== "number" || !Number.isFinite(value))
    throw bad(`${name} must be a finite number`);
  return value;
}

function goalFromBody(body) {
  const commodity = atom(body.commodity, "commodity");
  const origin = atom(body.origin, "origin");
  const packaging = atom(body.packaging, "packaging");
  const tested = atom(body.tested, "tested"); // yes | no
  if (!Array.isArray(body.readings)) throw bad("readings must be an array");
  const readings = body.readings
    .map((r, i) => `r(${num(r.minute, `readings[${i}].minute`)}, ${num(r.temp, `readings[${i}].temp`)})`)
    .join(", ");
  return `release(${commodity}, ${origin}, ${packaging}, ${tested}, [${readings}], D, Rs)`;
}

// why(Sev, Detail) compounds render as {functor, args}; reshape for clients.
function renderReason(t) {
  if (t && t.functor === "why") {
    const [severity, detail] = t.args;
    return detail && detail.functor
      ? { severity, rule: detail.functor, args: detail.args }
      : { severity, rule: detail };
  }
  return t;
}

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const headers = { "content-type": "application/json" };

    // Ad-hoc queries (the generated behavior), handy while developing rules.
    if (request.method === "GET" && url.searchParams.get("query")) {
      return new Response(runQuery(reactor(), url.searchParams.get("query").trim()), { headers });
    }

    // The service contract: POST /v1/release with a JSON lot description.
    if (request.method === "POST" && url.pathname === "/v1/release") {
      let body;
      try {
        body = await request.json();
      } catch {
        return new Response('{"error":"invalid JSON body"}', { status: 400, headers });
      }
      let goal;
      try {
        goal = goalFromBody(body);
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), { status: e.status ?? 400, headers });
      }
      const envelope = JSON.parse(runQuery(reactor(), goal));
      if ("error" in envelope)
        return new Response(JSON.stringify(envelope), { status: 422, headers });
      const sol = envelope.solutions[0] ?? {};
      return new Response(
        JSON.stringify({ decision: sol.D ?? null, reasons: (sol.Rs ?? []).map(renderReason) }),
        { headers },
      );
    }

    return new Response('{"error":"not found"}', { status: 404, headers });
  },
};
