%
%pull these into your repl with :edit
%
%
%cut example
%
membercheck(X, [X|_]) :- !.
membercheck(X, [_|L]) :- membercheck(X, L).
%
%cut example
%
max(X, Y, X) :- X >= Y, !.
max(X, Y, Y).
%
%cut example
%
drink(milk).
drink(beer) :- !.
drink(gin).
%
%cut example
%
evens([], []).
evens([X|T], [X|L]) :- 0 is X mod 2, !, evens(T, L).
evens([X|T], L) :- evens(T, L).
