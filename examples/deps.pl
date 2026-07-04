% A build/package dependency graph.
%:- io_format([text, bson]).

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

% shares_dep/3 — like shares_dep/2 but also reports the shared dependency D.
% One solution per shared dep, so `shares_dep(auth, B, D)` enumerates every
% (other-component, common-dependency) pair.
shares_dep(A, B, D) :- depends_on(A, D), depends_on(B, D), A \= B.

% shared_deps/3 — for each other component B, the LIST of deps A shares with it.
% (findall gathers the common deps for the pair; with no setof/2 in this engine,
% a pair sharing N deps is reported N times here — fine for this graph, where
% each sharing pair has exactly one dep in common.)
shared_deps(A, B, Ds) :- shares_dep(A, B), findall(D, shares_dep(A, B, D), Ds).
