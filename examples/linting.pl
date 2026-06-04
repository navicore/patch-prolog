% Linting rules for checking AI-generated API schemas.

% Allowed field types
allowed_type(string).
allowed_type(integer).
allowed_type(boolean).
allowed_type(array).

% Required fields for a valid user object
required_field(user, id).
required_field(user, name).
required_field(user, email).

% Defined fields (these would come from the AI output, baked in at build time)
field(user, id, integer).
field(user, name, string).
field(user, email, string).
field(user, password, string).
field(user, ssn, string).

% Violations
violation(Field, sensitive_field) :-
    field(user, Field, _),
    sensitive(Field).

violation(Field, unknown_type) :-
    field(user, Field, Type),
    \+ allowed_type(Type).

% Sensitive field names that should not be exposed
sensitive(ssn).
sensitive(password).
