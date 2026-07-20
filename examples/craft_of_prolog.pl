%
% Snippets from "The Craft of Prolog" — load into the repl with:
%   plgr
%   :load examples/craft_of_prolog.pl
% then query interactively, e.g.:
%   abs_diff(3, 10, D).              % D = 7
%   birthday(Who, date(dec, Day)).   % Who = noelen, Day = 25
%
% They compile standalone too:
%   plgc run examples/craft_of_prolog.pl --query "abs_diff(3, 10, D)" --format text
%
%
%abs diff
%
abs_diff(X, Y, Diff) :- compare(R, X, Y), abs_diff(R, X, Y, Diff).
abs_diff(<, X, Y, Diff) :- Diff is Y - X.
abs_diff(>, X, Y, Diff) :- Diff is X - Y.
abs_diff(=, _, _, 0).
%
%bday
%
birthday(byron, date(feb,4)).
birthday(noelen, date(dec,25)).
birthday(richard, date(oct,11)).
birthday(clare, date(sep,15)).
