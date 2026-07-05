%
%pull these into your repl with :edit
%
%
%abs diff
%
abs_diff(X, Y, Diff) :- compare(R, X, Y), abs_diff(R, X, Y, Diff).
abs_diff(<, X, Y, Diff) :- Diff is Y - X.
abs_diff(>, X, Y, Diff) :- Diff is X - Y.
abs_diff(=, _, _, 0).
