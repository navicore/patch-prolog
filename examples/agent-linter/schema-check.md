---
name: schema-check
description: Verify API schemas against a compiled Prolog policy before returning them. Use whenever drafting, editing, or reviewing schema or field definitions.
---

# Schema self-check contract

This project has a **compiled policy checker** at `./schema-lint` (a Prolog
rule set compiled to a standalone native binary — no runtime dependencies).
You MUST run it against any API schema before presenting it as final, and
re-run it after every fix, until it passes.

## The schema encoding

Render the schema as a Prolog list of `field(Name, Type)` terms, where
`Name` and `Type` are lowercase atoms (`[a-z][a-z0-9_]*`). Example — a user
record with an integer id, two strings, and one field of an invented type:

```
[field(id,integer), field(name,string), field(email,string), field(age,century)]
```

## The check

```sh
./schema-lint --query "violation([field(id,integer), field(ssn,string)], Field, Reason)"
```

Output and verdict (the exit code IS the verdict — always check it):

| Exit | Meaning | Your action |
|------|---------|-------------|
| `0`  | Clean — no violations (stdout shows `false.`) | Present the schema. |
| `1`  | Violations found, printed as `Field = <name>` / `Reason = <rule>` pairs | Fix EVERY reported field, re-run the check. |
| `2`  | The query itself was malformed | Fix your query encoding, re-run. |
| `3`  | Runtime error | Report; do not present the schema. |

Violation rules you may see: `sensitive_field` (the field must never appear
in a public schema — remove it), `unknown_type` (the type is outside the
allowed vocabulary — use only `string`, `integer`, `boolean`, `array`),
`wrong_type` (a known field carries the wrong type — correct it),
`missing_required` (a mandatory field is absent — add it).

## Rules

- NEVER skip the check, and never present a schema the checker rejected.
- NEVER invent types, and never edit the checker or the policy to make a
  failure go away — the policy is the authority; if it rejects the schema,
  the schema changes, not the policy.
- The checker's reasons are the explanation: quote them when you describe
  what you fixed.
