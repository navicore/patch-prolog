# Tutorial: give your agent a conscience — Prolog-checked output

Coding agents are fluent but unsound: ask one for an API schema and it will
confidently hand you a field named `ssn` typed as a `century`. This example
puts a **compiled Prolog policy** in your agent's tool reach, so the agent
*verifies its own work before returning it* — with a checker that cannot
hallucinate, because it's a rule set compiled to machine code.

The loop, in one picture:

```
you:  "draft a user-record schema"
agent: writes a draft
agent: runs ./schema-lint on it          ← the Prolog checker
agent: reads violations, fixes, re-runs  ← until exit 0
agent: presents a clean schema + the reasons it fixed
```

Everything deterministic is in this directory or one build command away; the
only probabilistic component is the agent itself — and you already have one.

## Components

| Component | Responsibility | Provided by |
|---|---|---|
| Your coding agent (pi, Claude Code, …) | drafts the schema, runs the checker, fixes violations | **you, already installed** |
| `examples/linting.pl` | the policy: allowed types, sensitive fields, required fields, expected types | this repo (the same program the [examples walkthrough](../../docs/examples.md) uses) |
| `schema-lint` binary | sound verification, exit-code verdict, no runtime deps | one `plgc build` command |
| `schema-check.md` | the contract that makes the agent *always* check | this directory |

No wasm toolchain, no Cloudflare account, no API keys beyond the agent you
already use. Native `plgc` is the only build prerequisite.

## Step 1 — build the checker

The policy is `examples/linting.pl`: violations are `sensitive_field`,
`unknown_type`, `wrong_type`, and `missing_required`, evaluated against a
schema handed in with the query. Compile it once:

```sh
plgc build examples/linting.pl -o schema-lint
```

Convince yourself it's an oracle before trusting the agent with it. A clean
schema exits 0 and prints `false.`; a dirty one exits 1 and prints every
violation:

```sh
./schema-lint --query "violation([field(id,integer), field(name,string), field(email,string)], F, R)"
# false.                                             (exit 0 — clean)

./schema-lint --query "violation([field(id,integer), field(ssn,string), field(age,century)], Field, Reason)"
# Field = ssn
# Reason = sensitive_field
# Field = age
# Reason = unknown_type
# Field = name
# Reason = missing_required
# Field = email
# Reason = missing_required                       (exit 1 — rejected)
```

Note what just happened: one binary, zero dependencies, and the **exit code
is the verdict** — `0` clean, `1` violations, `2` bad query, `3` runtime
error. That is exactly the shape an agent can act on without parsing prose.

## Step 2 — teach your agent the contract

`schema-check.md` (in this directory) is the rule that makes the agent
*always* run the check and never argue with the policy. Paste its contents
into whichever instruction channel your harness reads:

- **pi**: copy it to `.pi/skills/schema-check.md` in your project (it has
  skill frontmatter — pi discovers root `.md` files there), or reference it
  from your `AGENTS.md`.
- **Claude Code**: paste into `CLAUDE.md`.
- **Anything else**: paste into the system/developer instructions.

The contract in one sentence: *before presenting any schema, render it as a
Prolog list of `field(Name, Type)` terms, run `./schema-lint`, fix every
violation, re-run until exit 0 — and never edit the policy to make a failure
go away.*

## Step 3 — set the trap

Ask your agent for something with a planted flaw, in a project directory
where `schema-lint` and the instruction are in place:

> Draft a user-record schema for our public API: an integer id, name, email,
> and for account-recovery convenience include the user's ssn and password.
> Also add an age field of type century.

An unassisted agent will cheerfully return exactly that. With the contract in
place, the flow becomes (a **representative** transcript — your agent's prose
will differ; the tool calls are the part to watch):

```
agent: here's a first draft: [field(id,integer), field(name,string),
        field(email,string), field(ssn,string), field(password,string),
        field(age,century)]
agent: ./schema-lint --query "violation([field(id,integer), …], Field, Reason)"
       → exit 1: ssn/password sensitive_field, age unknown_type
agent: removing ssn and password (sensitive_field), retyping age as integer
agent: ./schema-lint --query "violation([field(id,integer), …], Field, Reason)"
       → exit 0
agent: final schema: id: integer, name: string, email: string, age: integer
       — dropped ssn/password (sensitive_field), corrected age to integer.
```

The agent caught its own mistake, cited the rule that caught it, and shipped
only what passed. You didn't review anything — the policy did.

## Step 4 — make the policy yours

The rules are data. Add your own to `examples/linting.pl` — a new sensitive
field, a new required field, a new expected type — rebuild, and every agent
in the project is bound by the new policy from the next check on:

```prolog
sensitive(api_key).
required_field(created_at).
expected_type(created_at, string).  % ISO-8601 timestamps
```

```sh
plgc build examples/linting.pl -o schema-lint
```

The recompile *is* the policy rollout. No agent-prompt edits, no re-training,
no hoping the model remembers.

## Why this works

- **Soundness where it matters.** The agent does the fuzzy work (naming,
  ergonomics, judgment); Prolog owns the rules. A violated invariant can't be
  talked into passing — `\+ allowed_type(Type)` doesn't care how politely
  it's asked.
- **Agent-legible interface.** Exit codes plus one-line reasons need no
  parsing sophistication. Any harness that can run a shell command can use it.
- **Zero-install deployment.** The checker is a ~1.2 MB static binary linking
  only libc/libm — drop it into any repo, container, or CI job.

## Where next

- The rule semantics (all four violation kinds, with a walkthrough):
  [docs/examples.md](../../docs/examples.md), the `linting.pl` section.
- The same "compiled policy as a service" pattern, deployed to the edge:
  [examples/coldchain/README.md](../coldchain/README.md).
- The natural sequel: this same loop where the *agent* is hosted — a sandbox
  with `schema-lint` in its filesystem, self-checking output before returning
  it. Watch this space.
