//! plg-frontend: ISO Prolog tokenizer, parser, and source-level static
//! analysis, ported from patch-prolog.
//!
//! Consumed by the compiler and the LSP. Compiled Prolog binaries carry a
//! minimal goal-only parser inside the runtime instead.

pub mod error;
pub mod lint;
pub mod parse_error;
pub mod parser;
pub mod source_map;
pub mod tokenizer;

pub use error::{PrologError, ThrownError, format_term};
pub use parse_error::ParseError;
pub use parser::{CallSite, CgClause, Parser, ProgramDirectives};
pub use source_map::SourceMap;
pub use tokenizer::{Token, TokenKind, Tokenizer};

use std::sync::OnceLock;

/// The `(name, arity)` of every predicate defined in the embedded stdlib
/// (`plg_shared::STDLIB_PL`), parsed once and cached. Lets the REPL (and,
/// eventually, the LSP) offer stdlib predicates — `member`/`append`/`length`/
/// `reverse`/`nth0`/`nth1`/`last` — in completion from one source, alongside
/// the builtin table. Returns an empty slice if the stdlib ever fails to parse
/// (it can't in practice; the stdlib is fixed).
pub fn stdlib_predicates() -> &'static [(String, usize)] {
    static CACHE: OnceLock<Vec<(String, usize)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut interner = plg_shared::StringInterner::new();
        let Ok((clauses, _)) =
            Parser::parse_program_with_directives(plg_shared::STDLIB_PL, &mut interner)
        else {
            return Vec::new();
        };
        let seen: std::collections::BTreeSet<(String, usize)> = clauses
            .iter()
            .filter_map(|c| c.head.functor_arity())
            .map(|(id, arity)| (interner.resolve(id).to_string(), arity))
            .collect();
        seen.into_iter().collect()
    })
}

#[cfg(test)]
mod tests {
    use super::stdlib_predicates;

    #[test]
    fn stdlib_predicates_include_list_helpers() {
        let preds = stdlib_predicates();
        for (name, arity) in [("append", 3), ("member", 2), ("length", 2), ("reverse", 2)] {
            assert!(
                preds.iter().any(|(n, a)| n == name && *a == arity),
                "stdlib missing {name}/{arity}"
            );
        }
    }

    #[test]
    fn stdlib_predicates_is_cached_and_stable() {
        // Same static slice on every call (OnceLock).
        let a = stdlib_predicates() as *const _;
        let b = stdlib_predicates() as *const _;
        assert_eq!(a, b);
    }
}
