% Sample linting rules for demonstration.
% These rules check for violations in AI-generated output.

% Example: flag fields that are not in the allowed set
allowed_field(name).
allowed_field(age).
allowed_field(email).

violation(Field, not_permitted) :-
    field(Field),
    \+ allowed_field(Field).
