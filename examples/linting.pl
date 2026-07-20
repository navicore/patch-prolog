% Linting rules for validating messages with typed fields — e.g. an
% AI-generated API schema — before they pass to the next pipeline step.
%
% The message under check is handed in with the query, as a list of
% field(Name, Type) terms — one binary validates any message:
%
%   plgc run examples/linting.pl --format text --query \
%     "violation([field(id,string), field(ssn,string), field(age,century)], F, R)"
%     F = ssn
%     R = sensitive_field
%     F = age
%     R = unknown_type
%     F = id
%     R = wrong_type
%     F = name
%     R = missing_required
%     F = email
%     R = missing_required
%
% Or build a standalone gate and drive it from the shell:
%   plgc build examples/linting.pl -o linting
%   ./linting --query "violation([field(id,integer)], F, R)" --format text
% The exit code is the verdict: 1 = message rejected, 0 = safe to pass on —
% so a CI step doesn't need to parse output at all.
%
% See docs/examples.md for a full walkthrough.

% ---- policy: the reusable asset, compiled in once -----------------------

% Allowed field types
allowed_type(string).
allowed_type(integer).
allowed_type(boolean).
allowed_type(array).

% Field names that must never appear in a public schema
sensitive(ssn).
sensitive(password).

% Fields a valid user object must have
required_field(id).
required_field(name).
required_field(email).

% The declared schema: what type each known field must have
expected_type(id, integer).
expected_type(name, string).
expected_type(email, string).

% ---- violations found in a schema (a list of field(Name, Type) terms) ---

% A sensitive field is present.
violation(Fields, Field, sensitive_field) :-
    member(field(Field, _), Fields),
    sensitive(Field).

% A field has a type outside the allowed vocabulary.
violation(Fields, Field, unknown_type) :-
    member(field(Field, Type), Fields),
    \+ allowed_type(Type).

% A known field carries the wrong type.
violation(Fields, Field, wrong_type) :-
    member(field(Field, Type), Fields),
    expected_type(Field, Expected),
    Type \= Expected.

% A required field is absent.
violation(Fields, Field, missing_required) :-
    required_field(Field),
    \+ member(field(Field, _), Fields).
