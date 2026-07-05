//! Completion vocabulary, sourced from the shared builtin table, the embedded
//! stdlib, and the session's own predicates — the same vocabulary the LSP
//! offers, so there is no third copy. Operators / `!` are excluded by
//! `completable()` upstream (only `atom_names`/`functor_names` are read).
//!
//! Two vocabularies, selected by the caller based on input context:
//! - [`candidates`] — predicate/atom names (builtins + stdlib + session).
//! - [`command_candidates`] — canonical REPL `:`-command names.

use crate::session::COMMANDS;
use plg_frontend::stdlib_predicates;
use plg_shared::builtins::{atom_names, functor_names};

/// Predicate/atom names matching `prefix`, drawn from builtins, the embedded
/// stdlib, and any extra session predicate names supplied by the caller.
pub fn candidates(prefix: &str, session_preds: &[String]) -> Vec<String> {
    let mut out: Vec<String> = atom_names()
        .map(str::to_string)
        .chain(functor_names().map(|(n, _)| n.to_string()))
        .chain(stdlib_predicates().iter().map(|(n, _)| n.clone()))
        .chain(session_preds.iter().cloned())
        .filter(|c| c.starts_with(prefix))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Canonical REPL command names (without the leading `:`) matching `prefix`,
/// sorted. Empty `prefix` ⇒ all commands. Aliases are for typing/parsing, not
/// completion output, so `:l` completes to `list`/`load` (canonical), not `ls`.
pub fn command_candidates(prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = COMMANDS
        .iter()
        .map(|c| c.name)
        .filter(|n| n.starts_with(prefix))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_candidates_by_prefix() {
        assert_eq!(command_candidates("l"), ["list", "load"]);
        assert_eq!(command_candidates("e"), ["edit"]);
        assert_eq!(command_candidates("sav"), ["save"]);
        assert_eq!(command_candidates("x"), Vec::<String>::new());
    }

    #[test]
    fn command_candidates_empty_prefix_offers_all_sorted() {
        assert_eq!(
            command_candidates(""),
            ["edit", "help", "list", "load", "quit", "reset", "save"]
        );
    }

    #[test]
    fn command_candidates_return_canonical_not_aliases() {
        // `:l` completes to canonical `list`/`load`, never the alias `ls`.
        assert!(!command_candidates("l").contains(&"ls".to_string()));
    }

    #[test]
    fn predicate_candidates_include_stdlib() {
        // The stdlib gap this change closes: append/member/length/reverse/
        // nth0/nth1/last now complete without the user defining them.
        let cands = candidates("", &[]);
        for name in [
            "append", "member", "length", "reverse", "nth0", "nth1", "last",
        ] {
            assert!(cands.iter().any(|c| c == name), "stdlib {name} missing");
        }
    }

    #[test]
    fn predicate_candidates_filter_by_prefix() {
        let cands = candidates("app", &[]);
        assert!(cands.iter().any(|c| c == "append"));
        assert!(cands.iter().all(|c| c.starts_with("app")));
    }
}
