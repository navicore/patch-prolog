% Standard library predicates for patch-prolog.
% These are compiled into the binary at build time.

% member/2 - check membership in a list
member(X, [X|_]).
member(X, [_|T]) :- member(X, T).

% append/3 - concatenate two lists
append([], L, L).
append([H|T], L, [H|R]) :- append(T, L, R).

% length/2 - length of a list
length([], 0).
length([_|T], N) :- length(T, N1), N is N1 + 1.

% last/2 - last element of a list
last([X], X).
last([_|T], X) :- last(T, X).

% reverse/2 - reverse a list
reverse(List, Reversed) :- reverse_acc(List, [], Reversed).
reverse_acc([], Acc, Acc).
reverse_acc([H|T], Acc, Reversed) :- reverse_acc(T, [H|Acc], Reversed).

% nth0/3 - zero-indexed element access
nth0(0, [H|_], H).
nth0(N, [_|T], X) :- N > 0, N1 is N - 1, nth0(N1, T, X).

% nth1/3 - one-indexed element access
nth1(1, [H|_], H).
nth1(N, [_|T], X) :- N > 1, N1 is N - 1, nth1(N1, T, X).
