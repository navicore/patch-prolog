# Tutorial: an edge release-decision service

Build and deploy a real REST service on Cloudflare's edge: a **cold-chain
release-decision API** for fresh produce, written in Prolog, compiled to a
reactor wasm module, and served from a V8 isolate. Every command and every
output below is real — the example program ships as
[`examples/coldchain.pl`](examples.md) and each step was run against it.

**The scenario.** A multistate *Cyclospora* outbreak is active and your
distribution center needs a decision for every incoming lot: `release`,
`hold_for_testing`, `quarantine`, or `reject` — plus **the reasons**, because
a safety decision you can't explain is worthless. Cyclospora makes the policy
interesting: it's a parasite that contaminates produce *at the source*, so
refrigeration doesn't mitigate it — an implicated lot has no release path no
matter how perfect its cold chain is. Temperature rules still apply to every
lot for ordinary quality reasons.

This is exactly the kind of logic that rots into a wall of if/then and
switch statements: tiered severities, exceptions on exceptions, and "list
*everything* that's wrong." In Prolog each rule is one clause, exceptions are
guards on the clause, and `findall` makes the audit trail the answer itself.

> The advisory data in the example is **illustrative**, not a real FDA
> advisory.

## Prerequisites

A wasm-capable `plgc` (both wasm targets are needed — the `wasm` feature
embeds both runtime archives), plus `workerd` for the local loop:

```sh
rustup target add wasm32-unknown-unknown wasm32-wasip1
rustup component add llvm-tools-preview      # llc + wasm-ld
just install-wasm                            # plgc with wasm support
npm i -g workerd                             # local dev loop
```

You only need Cloudflare tooling (`npx wrangler login`) at the deploy step.

## Step 1 — the policy, as facts

Create `coldchain.pl` (or follow along in `examples/coldchain.pl`). First the
outbreak advisory and the per-commodity transport policy:

```prolog
% Commodities named in the active Cyclospora advisory.
watchlist(basil).
watchlist(cilantro).
watchlist(raspberries).
watchlist(mesclun).

% Traceback-implicated (commodity, origin) pairs: no release path.
implicated(basil, mx_sonora).

% class_band(Commodity, MinC, MaxC) — acceptable transport temperatures.
class_band(basil, 2, 7).
class_band(romaine, 0, 4).
% …one clause per commodity…

% excursion_limit(Commodity, Minutes) — cumulative out-of-band time tolerated.
excursion_limit(basil, 60).
excursion_limit(romaine, 120).

% The packaging exception: a certified shipper extends the budget.
packaging_bonus(certified_shipper, 120).
```

Nothing here is control flow — the advisory *is* data. When the traceback
implicates a new origin tomorrow, that's one more clause, not a code change.

## Step 2 — rules that earn reasons

The heart of the service: `reason/6` relates a lot to every reason it fails,
tagged with a severity (`reject` > `hold` > `quarantine`):

```prolog
% An implicated (commodity, origin) pair is rejected outright. Note what is
% NOT in this clause: temperature. No cold chain clears a source-pathogen lot.
reason(Commodity, Origin, _Pkg, _Tested, _Readings,
       why(reject, implicated_source(Commodity, Origin))) :-
    implicated(Commodity, Origin).

% A watchlist commodity from a NON-implicated origin may still carry the
% parasite: hold for lab testing unless it arrived with a certified negative.
reason(Commodity, Origin, _Pkg, Tested, _Readings,
       why(hold, untested_watchlist(Commodity, Origin))) :-
    watchlist(Commodity),
    \+ implicated(Commodity, Origin),
    Tested = no.

% Fail closed: an unrecognized commodity earns manual review, never a
% release. Without this clause it would match no rule and auto-release.
reason(Commodity, _Origin, _Pkg, _Tested, _Readings,
       why(hold, unknown_commodity(Commodity))) :-
    \+ class_band(Commodity, _, _).
```

Read the second clause as the requirement reads — "watchlist, *unless*
implicated (that's `reject`'s job), *unless* tested." The `\+` guards are the
exceptions, stated where they belong instead of nested inside an if-ladder.

## Step 3 — cumulative temperature excursions

Sensor readings arrive as a list of `r(MinuteSinceLoading, TempC)` terms.
Two rules: a single reading more than 5° outside the band is a hard breach,
and total out-of-band time beyond the (packaging-adjusted) budget is an
excursion:

```prolog
reason(Commodity, _Origin, _Pkg, _Tested, Readings,
       why(quarantine, temp_breach(Minute, Temp))) :-
    member(r(Minute, Temp), Readings),
    class_band(Commodity, Lo, Hi),
    ( Temp < Lo - 5 ; Temp > Hi + 5 ).

reason(Commodity, _Origin, Pkg, _Tested, Readings,
       why(quarantine, excursion_exceeded(Minutes, Limit))) :-
    oob_minutes(Commodity, Readings, 0, Minutes),
    limit_for(Commodity, Pkg, Limit),
    Minutes > Limit.

% Sum the intervals ending at each out-of-band reading.
oob_minutes(_, [], _, 0).
oob_minutes(Commodity, [r(Minute, Temp)|Rs], Prev, Total) :-
    oob_minutes(Commodity, Rs, Minute, Rest),
    class_band(Commodity, Lo, Hi),
    ( (Temp < Lo ; Temp > Hi)
    -> Delta is Minute - Prev, Total is Rest + Delta
    ;  Total = Rest ).
```

`member/2` in the first rule is nondeterministic — it produces one reason
*per breaching reading*, no loop required. `limit_for/3` applies the
certified-shipper bonus with a `->` guard.

## Step 4 — the decision, tested natively

`findall` gathers **every** reason; the worst severity wins:

```prolog
release(Commodity, Origin, Pkg, Tested, Readings, Decision, Reasons) :-
    findall(R, reason(Commodity, Origin, Pkg, Tested, Readings, R), Reasons),
    decide(Reasons, Decision).

decide(Reasons, Decision) :-
    ( member(why(reject, _), Reasons) -> Decision = reject
    ; member(why(hold, _), Reasons)   -> Decision = hold_for_testing
    ; Reasons \= []                   -> Decision = quarantine
    ;                                   Decision = release ).
```

Develop with the native loop — answers are byte-identical to what the edge
module will return, so all rule work happens here:

```sh
plgc run examples/coldchain.pl --query \
  "release(romaine, us_az, standard, no, [r(0,2.0), r(120,6.5), r(240,6.9), r(360,3.0)], D, Rs)"
# D = quarantine
# Rs = [why(quarantine, excursion_exceeded(240, 120))]

# The same lot in a certified shipper — the exception flips the decision:
plgc run examples/coldchain.pl --query \
  "release(romaine, us_az, certified_shipper, no, [r(0,2.0), r(120,6.5), r(240,6.9), r(360,3.0)], D, Rs)"
# D = release
# Rs = []

# An implicated lot: lab-tested, perfect cold chain — rejected anyway.
plgc run examples/coldchain.pl --query \
  "release(basil, mx_sonora, certified_shipper, yes, [r(0,3.0), r(180,3.2)], D, Rs)"
# D = reject
# Rs = [why(reject, implicated_source(basil, mx_sonora))]
```

**The party trick** — run it backward. With a few sample lots compiled in,
backtracking enumerates *which lots fail and why* across the whole rule set:

```sh
plgc run examples/coldchain.pl --query "release_sample(Lot, D, Rs)"
# D = reject
# Lot = lot_1
# Rs = [why(reject, implicated_source(basil, mx_sonora))]
# D = quarantine
# Lot = lot_2
# Rs = [why(quarantine, excursion_exceeded(240, 120))]
# D = release
# Lot = lot_3
# Rs = []
# D = hold_for_testing
# Lot = lot_4
# Rs = [why(hold, untested_watchlist(mesclun, us_ca))]
```

No imperative version of this policy gives you that query for free.

## Step 5 — compile to a worker module

```sh
mkdir -p edge
plgc build --target worker examples/coldchain.pl -o edge/coldchain.worker.wasm
# note: wrote reactor.mjs, worker.js, wrangler.toml, config.capnp next to …
```

You get the reactor module (**≈1.8 MB** — well inside the Workers budget)
plus four scaffolding files, written only if absent: `reactor.mjs` (the
buffer-ABI marshalling), `worker.js` (the fetch handler — you'll edit it in
Step 7), `wrangler.toml`, and `config.capnp`. Rebuilds refresh the `.wasm`
and never clobber your glue edits.

## Step 6 — serve it locally

```sh
cd edge && workerd serve config.capnp
```

The generated handler takes the goal from `?query=` or a POST body:

```sh
curl -s --get --data-urlencode \
  'query=release(romaine, us_az, standard, no, [r(0,2.0), r(120,6.5), r(240,6.9), r(360,3.0)], D, Rs)' \
  http://localhost:8080/
```

```json
{"count":1,"exhausted":true,"output":"","solutions":[{"D":"quarantine","Rs":[{"functor":"why","args":["quarantine",{"functor":"excursion_exceeded","args":[240,120]}]}]}]}
```

Same engine, same answer as the native run — terms render host-side as
atoms→strings, numbers→numbers, lists→arrays, compounds→`{functor, args}`.
Errors keep their native text:

```sh
curl -s --get --data-urlencode 'query=release(romaine,' http://localhost:8080/
# {"error":"Parse error: unexpected end of query"}
```

## Step 7 — make it a REST service

The generated handler is a raw query passthrough. A real service owns its
contract: JSON in, decision out, Prolog inside. Replace the scaffolding
`worker.js` (it is never regenerated — editing is the intended use):

```js
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
```

The concurrency contract is preserved without thought: `runQuery` is fully
synchronous, and the only `await` (reading the body) happens before it.

Restart `workerd` and exercise the contract:

```sh
curl -s -X POST http://localhost:8080/v1/release \
  -H 'content-type: application/json' \
  --data '{"commodity":"romaine","origin":"us_az","packaging":"standard","tested":"no",
           "readings":[{"minute":0,"temp":2.0},{"minute":120,"temp":6.5},
                       {"minute":240,"temp":6.9},{"minute":360,"temp":3.0}]}'
```

```json
{"decision":"quarantine","reasons":[{"severity":"quarantine","rule":"excursion_exceeded","args":[240,120]}]}
```

Same lot, `"packaging":"certified_shipper"` → `{"decision":"release","reasons":[]}`.
An implicated lot:

```json
{"decision":"reject","reasons":[{"severity":"reject","rule":"implicated_source","args":["basil","mx_sonora"]}]}
```

And the edges behave like an API's should: an unknown commodity fails closed
(`{"decision":"hold_for_testing","reasons":[{"severity":"hold","rule":"unknown_commodity","args":["kale"]}]}`),
a malformed body gets `400 {"error":"invalid JSON body"}`, a bad field gets
`400 {"error":"commodity must be a lowercase atom like 'basil'"}`, and an
unknown path gets 404.

## Step 8 — deploy, then watch the outbreak evolve

`wrangler.toml` was emitted with the module; deploy from the `edge/`
directory:

```sh
npx wrangler login     # once
npx wrangler deploy
curl -s -X POST https://coldchain.<your-subdomain>.workers.dev/v1/release \
  -H 'content-type: application/json' --data '{"commodity":"mesclun", … }'
```

The isolate cold-starts once (`plg_init` builds the machine), then every
request is a warm in-memory call — no process fork, no per-request parse.

Now the payoff. The traceback implicates raspberries from Jalisco. The update
is one clause:

```prolog
implicated(raspberries, mx_jalisco).
```

Rebuild and redeploy:

```sh
plgc build --target worker examples/coldchain.pl -o edge/coldchain.worker.wasm
npx wrangler deploy
```

`lot_3` — which released yesterday — now returns
`{"decision":"reject","reasons":[{"severity":"reject","rule":"implicated_source","args":["raspberries","mx_jalisco"]}]}`.
The policy change touched no control flow, because there isn't any: the
advisory is data, and the deploy *is* the policy update.

## Tuning and limits

- **`runQuery` options** mirror the native knobs:
  `runQuery(reactor(), goal, { stepLimit: 100_000_000n })` raises the step
  ceiling (i64, hence the BigInt); `limit` bounds solutions, `depthLimit`
  bounds metacall depth. The defaults are ample for this rule set.
- **One in-flight query per isolate** — the handler above satisfies this by
  never yielding around `runQuery`. Keep it that way when you extend it.
- **Footprint**: large fact tables inflate the module's `.rodata`. A real
  advisory with thousands of implicated (lot, origin) pairs still fits
  comfortably; a national lot-level database belongs behind the API, passed
  in per-request, not compiled in.

## Where next

- Add severities (e.g. `recall` for lots already shipped) — one clause each.
- Version the advisory (`advisory(2025_07, …)`) and return the version in the
  response for audit trails.
- Pair with the [WASM Worker reference](wasm-worker.md) for the buffer ABI if
  you outgrow `reactor.mjs`, or drop to [Tier 1](wasm-target.md) if the data
  outgrows isolates.
