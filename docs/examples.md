# Examples

The `examples/` directory has runnable programs. Two are walked through
here: a dependency graph that shows the language basics, and a schema linter
that shows the headline use case — compiling a rule set into a standalone
checker.

Build any example and query it:

```sh
plgc build examples/deps.pl -o deps
./deps --query "needs(app, X)" --format text
```

## `deps.pl` — facts, rules, and recursion

```prolog
depends_on(app, auth).
depends_on(app, ui).
depends_on(auth, crypto).
depends_on(ui, render).
depends_on(render, crypto).

% needs/2 — the transitive closure of depends_on.
needs(X, Y) :- depends_on(X, Y).
needs(X, Y) :- depends_on(X, Z), needs(Z, Y).

% two components that share a direct dependency.
shares_dep(A, B) :- depends_on(A, D), depends_on(B, D), A \= B.
```

A handful of `depends_on/2` **facts** describing a build graph, then **rules**
that derive new relations. `needs/2` is **recursive** — what a component
depends on, transitively (a direct dependency, or a dependency of one). And
`shares_dep/2` uses `\=/2` to exclude a component matching itself.

Querying derives answers from the rules — and **backtracking** yields every
solution:

```sh
./deps --query "depends_on(app, X)" --format text   # direct dependencies
# X = auth
# X = ui

./deps --query "needs(app, X)" --format text         # transitive
# X = auth
# X = ui
# X = crypto
# X = render
# X = crypto

./deps --query "shares_dep(auth, render)" --format text
# true.
```

`needs(app, X)` reaches `crypto` twice — once through `auth`, once through
`ui → render` — because there are two paths to it; backtracking finds both.
`shares_dep(auth, render)` holds because both depend on `crypto`.

Use `findall/3` to collect every solution into a list:

```sh
./deps --query "findall(D, needs(app, D), Ds)" --format text
# D = _0
# Ds = [auth, ui, crypto, render, crypto]
```

(`D` is the template — left unbound; `Ds` is the collected result.)

## `linting.pl` — compiling a checker into a binary

This is the use case patch-prolog is built for: write the rules, compile
them to a single native binary, and run it anywhere with no Prolog system
installed. The **policy** — allowed types, sensitive names, required fields,
the declared schema — is baked in; the **message to validate** arrives with
each query, as a list of `field(Name, Type)` terms. One binary gates any
message: exit code 1 rejects it, 0 passes it to the next step.

```prolog
allowed_type(string).  allowed_type(integer).
allowed_type(boolean). allowed_type(array).

sensitive(ssn).
sensitive(password).

required_field(id).  required_field(name).  required_field(email).

expected_type(id, integer).
expected_type(name, string).
expected_type(email, string).

violation(Fields, Field, sensitive_field) :-
    member(field(Field, _), Fields),
    sensitive(Field).

violation(Fields, Field, unknown_type) :-
    member(field(Field, Type), Fields),
    \+ allowed_type(Type).

violation(Fields, Field, wrong_type) :-
    member(field(Field, Type), Fields),
    expected_type(Field, Expected),
    Type \= Expected.

violation(Fields, Field, missing_required) :-
    required_field(Field),
    \+ member(field(Field, _), Fields).
```

```sh
plgc build examples/linting.pl -o linting
./linting --format text --query \
  "violation([field(id,string), field(ssn,string), field(age,century)], F, R)"
# F = ssn
# R = sensitive_field
# F = age
# R = unknown_type
# F = id
# R = wrong_type
# F = name
# R = missing_required
# F = email
# R = missing_required
```

The real power is the **exit code**: a CI step doesn't need to parse output
at all — a non-zero exit means violations were found.

```sh
./linting --query "violation([field(id,integer)], _, _)" >/dev/null
if [ $? -eq 1 ]; then
    echo "message rejected"; exit 1
fi
```

And for tooling that *does* want structured data, the `bson` format is ready
to parse (the engine speaks `text` and `bson`, no JSON — a host wanting JSON
derives it from bson):

```sh
./linting --format bson --query \
  "violation([field(id,integer), field(name,string), field(email,string), field(ssn,string)], F, R)"
# a bson document: {count:1, exhausted:true, solutions:[{F: BinData, R: BinData}]}
```

That `linting` binary is ~700K, depends only on system libc/libm, and needs
no Prolog runtime to run — hand it to anyone.

## Where to next

- [Language Guide](language-guide.md) — the concepts these programs use.
- [Compiler Usage](compiler-usage.md) — every `plgc` flag and the query
  wire-contract.
- [REPL Guide](repl-guide.md) — explore a program interactively before
  compiling it.
