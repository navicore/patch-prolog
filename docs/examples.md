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
installed. Here the "data" (a schema's fields) is baked in at build time and
the rules flag violations.

```prolog
field(user, id, integer).
field(user, name, string).
field(user, email, string).
field(user, password, string).
field(user, ssn, string).

allowed_type(string).  allowed_type(integer).
allowed_type(boolean). allowed_type(array).

sensitive(ssn).
sensitive(password).

violation(Field, sensitive_field) :-
    field(user, Field, _),
    sensitive(Field).

violation(Field, unknown_type) :-
    field(user, Field, Type),
    \+ allowed_type(Type).
```

```sh
plgc build examples/linting.pl -o linting
./linting --query "violation(Field, Reason)" --format text
# Field = password
# Reason = sensitive_field
# Field = ssn
# Reason = sensitive_field
```

The real power is the **exit code**: a CI step doesn't need to parse output
at all — a non-zero exit means violations were found.

```sh
if ./linting --query "violation(_, _)" >/dev/null; then
    echo "schema violations found"; exit 1
fi
```

And for tooling that *does* want structured data, the `bson` format is ready
to parse (the engine speaks `text` and `bson`, no JSON — a host wanting JSON
derives it from bson):

```sh
./linting --query "violation(F, R)" --format bson
# a bson document: {count:2, exhausted:true, solutions:[{F: "password", ...}, ...]}
```

That `linting` binary is ~700K, depends only on system libc/libm, and needs
no Prolog runtime to run — hand it to anyone.

## Where to next

- [Language Guide](language-guide.md) — the concepts these programs use.
- [Compiler Usage](compiler-usage.md) — every `plgc` flag and the query
  wire-contract.
- [REPL Guide](repl-guide.md) — explore a program interactively before
  compiling it.
