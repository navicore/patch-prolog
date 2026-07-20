%
% Cut (!) examples — load into the repl with:
%   plgr
%   :load examples/cut_examples.pl
% then query interactively, e.g.:
%   max(3, 7, M).          % M = 7
%   evens([1,2,3,4,5,6], E).  % E = [2, 4, 6]
%   drink(D).              % D = milk ; D = beer — the cut blocks gin
%
% They compile standalone too:
%   plgc run examples/cut_examples.pl --query "evens([1,2,3,4,5,6], E)" --format text
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
