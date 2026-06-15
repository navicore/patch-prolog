//! Completion vocabulary, sourced from the shared builtin table plus the
//! session's own predicates — the same `plg-shared` vocabulary the LSP
//! offers, so there is no third copy. Operators / `!` are excluded by
//! `completable()` upstream (only `atom_names`/`functor_names` are read).

use plg_shared::builtins::{atom_names, functor_names};

/// Names matching `prefix`, drawn from builtins, stdlib-via-builtins, and
/// any extra session predicate names supplied by the caller.
pub fn candidates(prefix: &str, session_preds: &[String]) -> Vec<String> {
    let mut out: Vec<String> = atom_names()
        .map(str::to_string)
        .chain(functor_names().map(|(n, _)| n.to_string()))
        .chain(session_preds.iter().cloned())
        .filter(|c| c.starts_with(prefix))
        .collect();
    out.sort();
    out.dedup();
    out
}
