//! ISO Prolog error terms.
//!
//! Built-ins and the solver construct `PrologError` values when something goes
//! wrong. Each variant maps 1:1 to an ISO 13211-1 §7.12 formal error term;
//! `to_term` renders the value as `error(Formal, Context)` so user-level
//! `catch/3` recovery clauses can pattern-match against the structured term.
//!
//! Construction is cheap — no allocation on the success path. The optional
//! `context` string lives in the second argument of `error/2` and is the
//! human-readable message format-style; existing tests grep for substrings
//! in this field, so it's where helpful detail goes.
//!
//! Ported from patch-prolog's `error.rs`; the only change is sourcing
//! `Term` / `StringInterner` from `plg_shared` instead of the old in-crate
//! `term` module.

use plg_shared::{StringInterner, Term};

/// A thrown error as it flows through the solver. The term is what `catch/3`
/// pattern-matches against; the `uncatchable` flag is set for safety-ceiling
/// errors (step limit) so user `catch/3` clauses cannot trap them.
#[derive(Debug, Clone)]
pub struct ThrownError {
    pub term: Term,
    pub uncatchable: bool,
}

impl ThrownError {
    /// Build from a structured `PrologError` — `uncatchable` is derived from
    /// the variant.
    pub fn from_prolog(err: PrologError, interner: &mut StringInterner) -> Self {
        let uncatchable = err.is_uncatchable();
        let term = err.to_term(interner);
        ThrownError { term, uncatchable }
    }

    /// Build from a raw term (used by `throw/1`). Always catchable.
    pub fn from_term(term: Term) -> Self {
        ThrownError {
            term,
            uncatchable: false,
        }
    }
}

/// ISO formal-error vocabulary.
#[derive(Debug, Clone)]
pub enum PrologError {
    /// An argument that must be bound was unbound.
    Instantiation { context: String },
    /// An argument has the wrong type. `expected_type` is an atom name
    /// (e.g. "integer", "atom", "callable"); `culprit` is the offending term.
    Type {
        expected_type: &'static str,
        culprit: Term,
        context: String,
    },
    /// An object referred to by the goal does not exist.
    /// `object_type` is e.g. "procedure"; `culprit` is the indicator.
    Existence {
        object_type: &'static str,
        culprit: Term,
        context: String,
    },
    /// An argument is outside the valid domain.
    Domain {
        expected_domain: &'static str,
        culprit: Term,
        context: String,
    },
    /// Arithmetic evaluation failed (zero divisor, overflow, etc.).
    Evaluation {
        kind: &'static str, // "zero_divisor", "int_overflow", "float_overflow", ...
        context: String,
    },
    /// Operation not permitted.
    Permission {
        operation: &'static str,
        permission_type: &'static str,
        culprit: Term,
        context: String,
    },
    /// Implementation-defined representation limit exceeded.
    Representation {
        flag: &'static str, // "max_arity", "character_code", ...
        context: String,
    },
    /// Resource limit exhausted. Note: `Resource { kind: "steps", ... }` is
    /// the step-limit error and is intentionally uncatchable — see
    /// `is_uncatchable`.
    Resource {
        kind: &'static str, // "steps", "memory", ...
        context: String,
    },
    /// Syntax error during runtime term construction (e.g. `number_chars/2`).
    Syntax { context: String },
}

impl PrologError {
    /// Step-limit and other safety-ceiling errors must not be catchable by
    /// `catch/3` — otherwise a malicious rule could loop indefinitely by
    /// trapping its own timeout. The solver checks this before consulting
    /// the catch stack.
    pub fn is_uncatchable(&self) -> bool {
        matches!(self, PrologError::Resource { kind: "steps", .. })
    }

    /// Render as the ISO term `error(Formal, Context)`.
    /// `Formal` matches the variant (e.g. `type_error(integer, foo)`);
    /// `Context` is the human-readable atom (the message string interned
    /// as an atom). Test helpers grep this string for substrings.
    pub fn to_term(&self, interner: &mut StringInterner) -> Term {
        let formal = self.formal_term(interner);
        let context_atom = interner.intern(self.context());
        let error_functor = interner.intern("error");
        Term::Compound {
            functor: error_functor,
            args: vec![formal, Term::Atom(context_atom)],
        }
    }

    /// The human-readable context message. This is what test helpers
    /// substring-match against and what the CLI shows.
    pub fn context(&self) -> &str {
        match self {
            PrologError::Instantiation { context }
            | PrologError::Type { context, .. }
            | PrologError::Existence { context, .. }
            | PrologError::Domain { context, .. }
            | PrologError::Evaluation { context, .. }
            | PrologError::Permission { context, .. }
            | PrologError::Representation { context, .. }
            | PrologError::Resource { context, .. }
            | PrologError::Syntax { context } => context,
        }
    }

    fn formal_term(&self, interner: &mut StringInterner) -> Term {
        match self {
            PrologError::Instantiation { .. } => Term::Atom(interner.intern("instantiation_error")),
            PrologError::Type {
                expected_type,
                culprit,
                ..
            } => Term::Compound {
                functor: interner.intern("type_error"),
                args: vec![Term::Atom(interner.intern(expected_type)), culprit.clone()],
            },
            PrologError::Existence {
                object_type,
                culprit,
                ..
            } => Term::Compound {
                functor: interner.intern("existence_error"),
                args: vec![Term::Atom(interner.intern(object_type)), culprit.clone()],
            },
            PrologError::Domain {
                expected_domain,
                culprit,
                ..
            } => Term::Compound {
                functor: interner.intern("domain_error"),
                args: vec![
                    Term::Atom(interner.intern(expected_domain)),
                    culprit.clone(),
                ],
            },
            PrologError::Evaluation { kind, .. } => Term::Compound {
                functor: interner.intern("evaluation_error"),
                args: vec![Term::Atom(interner.intern(kind))],
            },
            PrologError::Permission {
                operation,
                permission_type,
                culprit,
                ..
            } => Term::Compound {
                functor: interner.intern("permission_error"),
                args: vec![
                    Term::Atom(interner.intern(operation)),
                    Term::Atom(interner.intern(permission_type)),
                    culprit.clone(),
                ],
            },
            PrologError::Representation { flag, .. } => Term::Compound {
                functor: interner.intern("representation_error"),
                args: vec![Term::Atom(interner.intern(flag))],
            },
            PrologError::Resource { kind, .. } => Term::Compound {
                functor: interner.intern("resource_error"),
                args: vec![Term::Atom(interner.intern(kind))],
            },
            PrologError::Syntax { .. } => Term::Atom(interner.intern("syntax_error")),
        }
    }

    /// Render the error term as a human-readable string.
    /// Used by the CLI and the test helper for substring assertions.
    pub fn to_display(&self, interner: &mut StringInterner) -> String {
        let term = self.to_term(interner);
        let mut out = String::new();
        format_term(&term, interner, &mut out);
        out
    }
}

/// Format an arbitrary term as a Prolog-syntax string. Used by the CLI for
/// rendering uncaught error terms. Lives here (not in a general
/// pretty-printer module) because the only consumer is the error path.
pub fn format_term(term: &Term, interner: &StringInterner, out: &mut String) {
    match term {
        Term::Atom(id) => out.push_str(interner.resolve(*id)),
        Term::Var(id) => {
            out.push('_');
            out.push_str(&id.to_string());
        }
        Term::Integer(n) => out.push_str(&n.to_string()),
        Term::Float(f) => out.push_str(&f.to_string()),
        Term::Compound { functor, args } => {
            out.push_str(interner.resolve(*functor));
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_term(a, interner, out);
            }
            out.push(')');
        }
        Term::List { head, tail } => {
            out.push('[');
            format_term(head, interner, out);
            let mut cur = tail.as_ref();
            loop {
                match cur {
                    Term::List { head, tail } => {
                        out.push_str(", ");
                        format_term(head, interner, out);
                        cur = tail;
                    }
                    Term::Atom(id) if interner.resolve(*id) == "[]" => break,
                    other => {
                        out.push('|');
                        format_term(other, interner, out);
                        break;
                    }
                }
            }
            out.push(']');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiation_error_term_shape() {
        let mut interner = StringInterner::new();
        let err = PrologError::Instantiation {
            context: "X must be bound".into(),
        };
        let term = err.to_term(&mut interner);
        match term {
            Term::Compound { functor, args } => {
                assert_eq!(interner.resolve(functor), "error");
                assert_eq!(args.len(), 2);
                match &args[0] {
                    Term::Atom(id) => assert_eq!(interner.resolve(*id), "instantiation_error"),
                    _ => panic!("expected atom formal term"),
                }
            }
            _ => panic!("expected compound error/2"),
        }
    }

    #[test]
    fn type_error_term_shape() {
        let mut interner = StringInterner::new();
        let foo = interner.intern("foo");
        let err = PrologError::Type {
            expected_type: "integer",
            culprit: Term::Atom(foo),
            context: "arithmetic on non-number".into(),
        };
        let term = err.to_term(&mut interner);
        // error(type_error(integer, foo), 'arithmetic on non-number')
        let Term::Compound { args, .. } = term else {
            panic!("expected compound");
        };
        let Term::Compound {
            functor: type_functor,
            args: type_args,
        } = &args[0]
        else {
            panic!("expected formal compound");
        };
        assert_eq!(interner.resolve(*type_functor), "type_error");
        assert_eq!(type_args.len(), 2);
        assert!(matches!(type_args[1], Term::Atom(id) if interner.resolve(id) == "foo"));
    }

    #[test]
    fn existence_error_indicator() {
        let mut interner = StringInterner::new();
        let f = interner.intern("frobnicate");
        let slash = interner.intern("/");
        let indicator = Term::Compound {
            functor: slash,
            args: vec![Term::Atom(f), Term::Integer(2)],
        };
        let err = PrologError::Existence {
            object_type: "procedure",
            culprit: indicator,
            context: "frobnicate/2 is undefined".into(),
        };
        let display = err.to_display(&mut interner);
        assert!(
            display.contains("existence_error(procedure, /(frobnicate, 2))"),
            "got: {display}"
        );
        assert!(display.contains("frobnicate/2 is undefined"));
    }

    #[test]
    fn evaluation_error_zero_divisor_renders() {
        let mut interner = StringInterner::new();
        let err = PrologError::Evaluation {
            kind: "zero_divisor",
            context: "Division by zero".into(),
        };
        let display = err.to_display(&mut interner);
        assert!(
            display.contains("evaluation_error(zero_divisor)"),
            "got: {display}"
        );
        assert!(display.contains("zero"));
    }

    #[test]
    fn resource_steps_is_uncatchable() {
        let err = PrologError::Resource {
            kind: "steps",
            context: "step limit exceeded".into(),
        };
        assert!(err.is_uncatchable());
    }

    #[test]
    fn other_errors_are_catchable() {
        let err = PrologError::Type {
            expected_type: "integer",
            culprit: Term::Integer(0),
            context: String::new(),
        };
        assert!(!err.is_uncatchable());
    }
}
