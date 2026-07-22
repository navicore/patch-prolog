% Cold-chain + outbreak release decisions for fresh produce.
%
% The scenario: a multistate Cyclospora outbreak is active. Cyclospora
% contaminates produce AT THE SOURCE (irrigation water, field handling) —
% refrigeration does not mitigate it, so an implicated lot has no release
% path no matter how perfect its cold chain is. Temperature rules still
% apply to every lot for ordinary quality/safety reasons.
%
% The policy below is what a wall of if/then and switch statements usually
% becomes. Here each rule is a clause: adding an advisory or a packaging
% exception never touches the control flow, and `findall` enumerates EVERY
% reason a lot fails — the audit trail is the answer, not a log line.
%
% Try it (compiles to a temp binary and runs the query):
%   plgc run examples/coldchain/coldchain.pl --query \
%     "release(romaine, us_az, standard, no, [r(0,2.0), r(120,6.5), r(240,6.9), r(360,3.0)], D, Rs)"
%     D = quarantine
%     Rs = [why(quarantine, excursion_exceeded(240, 120))]
%
% Same lot, certified shipper packaging (extends the excursion budget):
%   plgc run examples/coldchain/coldchain.pl --query \
%     "release(romaine, us_az, certified_shipper, no, [r(0,2.0), r(120,6.5), r(240,6.9), r(360,3.0)], D, Rs)"
%     D = release
%
% An implicated lot is rejected even with lab tests and a perfect cold chain:
%   plgc run examples/coldchain/coldchain.pl --query \
%     "release(basil, mx_sonora, certified_shipper, yes, [r(0,3.0), r(180,3.2)], D, Rs)"
%     D = reject
%     Rs = [why(reject, implicated_source(basil, mx_sonora))]
%
% Run it backward over the compiled-in sample lots (the generative query):
%   plgc run examples/coldchain/coldchain.pl --query "release_sample(Lot, D, Rs)"
%
% NOTE: advisory data below is ILLUSTRATIVE, not a real FDA advisory.

% ── Outbreak advisory (compiled in; an evolving outbreak = edit + redeploy) ──

% Commodities named in the active Cyclospora advisory.
watchlist(basil).
watchlist(cilantro).
watchlist(raspberries).
watchlist(mesclun).

% Traceback-implicated (commodity, origin) pairs: no release path.
implicated(basil, mx_sonora).

% ── Commodity policy: temperature band + excursion budget ───────────────────

% class_band(Commodity, MinC, MaxC) — acceptable transport temperatures.
class_band(basil, 2, 7).
class_band(cilantro, 0, 4).
class_band(raspberries, 0, 2).
class_band(mesclun, 0, 4).
class_band(romaine, 0, 4).

% excursion_limit(Commodity, Minutes) — cumulative out-of-band time tolerated.
excursion_limit(basil, 60).
excursion_limit(cilantro, 60).
excursion_limit(raspberries, 120).
excursion_limit(mesclun, 60).
excursion_limit(romaine, 120).

% The packaging exception: a certified active-cooling shipper extends the
% excursion budget. In imperative code this is where the nested ifs begin.
packaging_bonus(certified_shipper, 120).

% limit_for/3 — effective excursion budget for a (commodity, packaging) pair.
limit_for(Commodity, Pkg, Limit) :-
    excursion_limit(Commodity, Base),
    ( packaging_bonus(Pkg, Bonus) -> Limit is Base + Bonus ; Limit = Base ).

% ── The rules: each clause is one way a lot earns a reason ──────────────────
% reason(Commodity, Origin, Pkg, Tested, Readings, why(Severity, Detail))
% Severities, worst first: reject > hold > quarantine.

% An implicated (commodity, origin) pair is rejected outright. Note what is
% NOT in this clause: temperature. No cold chain clears a source-pathogen lot.
reason(Commodity, Origin, _Pkg, _Tested, _Readings,
       why(reject, implicated_source(Commodity, Origin))) :-
    implicated(Commodity, Origin).

% A watchlist commodity from a NON-implicated origin may still carry the
% parasite: hold for lab testing unless the lot arrived with a certified
% negative result. `\+` expresses the exception directly.
reason(Commodity, Origin, _Pkg, Tested, _Readings,
       why(hold, untested_watchlist(Commodity, Origin))) :-
    watchlist(Commodity),
    \+ implicated(Commodity, Origin),
    Tested = no.

% Fail closed: an unrecognized commodity earns manual review, never a
% release. Without this clause an unknown commodity matches no rule and
% findall returns [] — a silent, automatic release.
reason(Commodity, _Origin, _Pkg, _Tested, _Readings,
       why(hold, unknown_commodity(Commodity))) :-
    \+ class_band(Commodity, _, _).

% Hard temperature breach: any single reading more than 5° outside the band
% is damage no cumulative budget excuses.
reason(Commodity, _Origin, _Pkg, _Tested, Readings,
       why(quarantine, temp_breach(Minute, Temp))) :-
    member(r(Minute, Temp), Readings),
    class_band(Commodity, Lo, Hi),
    ( Temp < Lo - 5 ; Temp > Hi + 5 ).

% Cumulative excursion: total out-of-band minutes exceeds the effective limit.
reason(Commodity, _Origin, Pkg, _Tested, Readings,
       why(quarantine, excursion_exceeded(Minutes, Limit))) :-
    oob_minutes(Commodity, Readings, 0, Minutes),
    limit_for(Commodity, Pkg, Limit),
    Minutes > Limit.

% oob_minutes/4 — sum, over readings r(Minute, Temp), of the interval since
% the previous reading for every reading outside the commodity's band.
% (Each reading is taken to represent the interval that ended at it.)
oob_minutes(_, [], _, 0).
oob_minutes(Commodity, [r(Minute, Temp)|Rs], Prev, Total) :-
    oob_minutes(Commodity, Rs, Minute, Rest),
    class_band(Commodity, Lo, Hi),
    ( (Temp < Lo ; Temp > Hi)
    -> Delta is Minute - Prev, Total is Rest + Delta
    ;  Total = Rest ).

% ── The decision: every reason, then the worst severity wins ────────────────

% release(Commodity, Origin, Pkg, Tested, Readings, Decision, Reasons)
%   Tested: yes | no   (certified negative Cyclospora result for the lot)
%   Readings: list of r(MinuteSinceLoading, TempC)
release(Commodity, Origin, Pkg, Tested, Readings, Decision, Reasons) :-
    findall(R, reason(Commodity, Origin, Pkg, Tested, Readings, R), Reasons),
    decide(Reasons, Decision).

decide(Reasons, Decision) :-
    ( member(why(reject, _), Reasons) -> Decision = reject
    ; member(why(hold, _), Reasons)   -> Decision = hold_for_testing
    ; Reasons \= []                   -> Decision = quarantine
    ;                                   Decision = release ).

% ── Sample lots, compiled in, for the generative query ──────────────────────

sample_shipment(lot_1, basil, mx_sonora, certified_shipper, yes,
    [r(0, 3.0), r(180, 3.2), r(360, 3.1)]).
sample_shipment(lot_2, romaine, us_az, standard, no,
    [r(0, 2.0), r(120, 6.5), r(240, 6.9), r(360, 3.0)]).
sample_shipment(lot_3, raspberries, mx_jalisco, certified_shipper, yes,
    [r(0, 1.0), r(200, 1.4), r(400, 1.6)]).
sample_shipment(lot_4, mesclun, us_ca, standard, no,
    [r(0, 1.5), r(180, 1.9), r(360, 2.0)]).

% release_sample/3 — ask the rule set which sample lots fail, and why.
release_sample(Lot, Decision, Reasons) :-
    sample_shipment(Lot, Commodity, Origin, Pkg, Tested, Readings),
    release(Commodity, Origin, Pkg, Tested, Readings, Decision, Reasons).
