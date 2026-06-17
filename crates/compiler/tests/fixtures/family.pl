% Classic family relationships example.
parent(tom, mary).
parent(tom, james).
parent(tom, ann).
parent(mary, bob).
parent(james, carol).

grandparent(X, Z) :- parent(X, Y), parent(Y, Z).

ancestor(X, Y) :- parent(X, Y).
ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).

sibling(X, Y) :- parent(Z, X), parent(Z, Y), X \= Y.
