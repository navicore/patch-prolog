% A build/package dependency graph.
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
