# Examples

The `examples/` directory has runnable programs. Two are walked through
here: a classic genealogy database that shows the language basics, and a
schema linter that shows the headline use case — compiling a rule set into
a standalone checker.

Build any example and query it:

```sh
plgc build examples/family.pl -o family
./family --query "grandparent(tom, X)" --format text
```

## `family.pl` — facts, rules, and recursion

```prolog
parent(tom, mary).
parent(tom, james).
parent(tom, ann).
parent(mary, bob).
parent(james, carol).

grandparent(X, Z) :- parent(X, Y), parent(Y, Z).

ancestor(X, Y) :- parent(X, Y).
ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).

sibling(X, Y) :- parent(Z, X), parent(Z, Y), X \= Y.
```

A handful of `parent/2` **facts**, then **rules** that derive new relations.
`grandparent/2` is one rule (a parent of a parent); `ancestor/2` is
**recursive** (a parent, or a parent of an ancestor); `sibling/2` uses
`\=/2` to exclude a person being their own sibling.

Querying derives answers from the rules — and **backtracking** yields every
solution:

```sh
./family --query "grandparent(tom, X)" --format text
# X = bob
# X = carol

./family --query "ancestor(tom, X)" --format text
# X = mary
# X = james
# X = ann
# X = bob
# X = carol

./family --query "sibling(mary, X)" --format text
# X = james
# X = ann
```

Use `findall/3` to collect every solution into a list:

```sh
./family --query "findall(G, grandparent(tom, G), Gs)" --format text
# G = _0
# Gs = [bob, carol]
```

(`G` is the template — left unbound; `Gs` is the collected result.)

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

And for tooling that *does* want the data, the default JSON format is ready
to parse:

```sh
./linting --query "violation(F, R)" --format json
# {"count":2,"exhausted":true,"solutions":[{"F":"password","R":"sensitive_field"},
#                                          {"F":"ssn","R":"sensitive_field"}]}
```

That `linting` binary is ~700K, depends only on system libc/libm, and needs
no Prolog runtime to run — hand it to anyone.

## Where to next

- [Language Guide](language-guide.md) — the concepts these programs use.
- [Compiler Usage](compiler-usage.md) — every `plgc` flag and the query
  wire-contract.
- [REPL Guide](repl-guide.md) — explore a program interactively before
  compiling it.
