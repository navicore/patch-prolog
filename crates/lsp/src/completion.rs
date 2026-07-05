//! Completion items.
//!
//! Sources, in order of preference:
//! 1. Built-in predicates (from `plg_shared::builtins` — the shared
//!    vocabulary table; only `completable()` names are offered, so
//!    operators and `!` never appear as completions).
//! 2. Stdlib predicates from the shared `plg_frontend::stdlib_predicates`
//!    (parsed once from `plg_shared::STDLIB_PL`).
//! 3. User-defined predicates discovered by parsing the current buffer.
//!
//! All three are filtered by the prefix the user is typing. Prefix detection
//! walks backward from the cursor over Prolog-identifier characters
//! (alphanumeric + underscore).

use std::collections::BTreeSet;

use plg_frontend::stdlib_predicates;
use plg_shared::builtins::{atom_names, functor_names};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

use crate::buffer::{parse_best_effort, position_to_byte_offset, predicate_indicator};

pub fn compute(content: &str, position: Position) -> Vec<CompletionItem> {
    let prefix = current_word_prefix(content, position);

    // Collected in dedup order: USER overrides STDLIB which overrides BUILT-IN.
    // Key is (name, arity) — different arities of the same name are distinct
    // completions, and the user owning a name should bury both the built-in
    // (if any) and the stdlib copy.
    let user_predicates = user_predicates(content);
    let user_keys: std::collections::HashSet<(String, usize)> =
        user_predicates.iter().cloned().collect();
    let stdlib = stdlib_predicates();
    let stdlib_keys: std::collections::HashSet<(String, usize)> = stdlib.iter().cloned().collect();

    let mut items: Vec<CompletionItem> = Vec::new();

    // 1. Built-in atoms (true, fail, nl, ...) — 0-arity. Skip if shadowed by
    // a stdlib or user predicate of the same (name, 0).
    for name in atom_names() {
        if !name.starts_with(&prefix) {
            continue;
        }
        let key = (name.to_string(), 0);
        if user_keys.contains(&key) || stdlib_keys.contains(&key) {
            continue;
        }
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("built-in {name}/0")),
            ..Default::default()
        });
    }

    // 2. Built-in compound predicates.
    for (name, arity) in functor_names() {
        if !name.starts_with(&prefix) {
            continue;
        }
        let key = (name.to_string(), arity as usize);
        if user_keys.contains(&key) || stdlib_keys.contains(&key) {
            continue;
        }
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("built-in {name}/{arity}")),
            ..Default::default()
        });
    }

    // 3. Stdlib predicates — skip if the user has redefined this `(name, arity)`.
    for (name, arity) in stdlib {
        if !name.starts_with(&prefix) {
            continue;
        }
        if user_keys.contains(&(name.clone(), *arity)) {
            continue;
        }
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("stdlib {name}/{arity}")),
            ..Default::default()
        });
    }

    // 4. User-defined predicates from the current buffer — always present.
    for (name, arity) in &user_predicates {
        if !name.starts_with(&prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("{name}/{arity}")),
            ..Default::default()
        });
    }

    items
}

/// Parse the buffer and collect distinct `(name, arity)` from clause heads.
/// The chunked recovery in `parse_best_effort` surfaces predicates from every
/// complete clause in the buffer, so completion stays useful even when an
/// unfinished clause sits between two well-formed ones.
fn user_predicates(content: &str) -> Vec<(String, usize)> {
    let Some((clauses, _, interner)) = parse_best_effort(content) else {
        return Vec::new();
    };
    let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();
    for clause in &clauses {
        if let Some(pair) = predicate_indicator(&clause.head, &interner) {
            seen.insert(pair);
        }
    }
    seen.into_iter().collect()
}

/// Walk backwards from `position` over Prolog-identifier characters to find
/// the prefix the user is currently typing. Honors LSP's UTF-16 position
/// convention so multibyte chars before the cursor don't slice mid-codepoint.
fn current_word_prefix(content: &str, position: Position) -> String {
    let Some(byte_col) = position_to_byte_offset(content, position) else {
        return String::new();
    };
    let line_start = content[..byte_col]
        .rfind('\n')
        .map(|nl| nl + 1)
        .unwrap_or(0);
    let before = &content[line_start..byte_col];
    let prefix_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(before.len());
    before[prefix_start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, col: u32) -> Position {
        Position {
            line,
            character: col,
        }
    }

    #[test]
    fn prefix_at_end_of_word() {
        assert_eq!(current_word_prefix("foo\nmem", pos(1, 3)), "mem");
    }

    #[test]
    fn prefix_at_start_of_line_is_empty() {
        assert_eq!(current_word_prefix("foo\n", pos(1, 0)), "");
    }

    #[test]
    fn prefix_after_paren_starts_fresh() {
        // member(a, b -> after `b` we should be inside the word `b`.
        assert_eq!(current_word_prefix("member(a, b", pos(0, 11)), "b");
        // After the open paren, no prefix.
        assert_eq!(current_word_prefix("member(", pos(0, 7)), "");
    }

    // Regression for the UTF-16 panic: a buffer with a non-ASCII char before
    // the cursor must not slice mid-codepoint.
    #[test]
    fn prefix_with_multibyte_char_before_cursor_no_panic() {
        // `é` is 2 bytes in UTF-8 but 1 UTF-16 code unit.
        let content = "% é\nmem";
        let result = current_word_prefix(content, pos(1, 3));
        assert_eq!(result, "mem");
    }

    #[test]
    fn completion_finds_stdlib_member() {
        // Type "mem|" → should include `member` from stdlib.
        let items = compute("p :- mem", pos(0, 8));
        let names: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(names.contains(&"member"), "items: {names:?}");
    }

    #[test]
    fn completion_finds_builtin_findall() {
        let items = compute("p :- find", pos(0, 9));
        let names: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(names.contains(&"findall"), "items: {names:?}");
    }

    #[test]
    fn completion_finds_user_predicate_in_buffer() {
        let src = "violation(X, sensitive) :- field(X), sensitive(X).\np :- viol";
        // cursor at the end of "viol" on line 1.
        let items = compute(src, pos(1, 7));
        let names: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(names.contains(&"violation"), "items: {names:?}");
    }

    #[test]
    fn completion_filters_by_prefix() {
        // Empty prefix → many items; specific prefix → fewer.
        let many = compute("p :- ", pos(0, 5));
        let few = compute("p :- mem", pos(0, 8));
        assert!(many.len() > few.len());
    }

    // Operators and `!` must NOT appear as completions (completable() filter
    // in the shared table) — they would be noise.
    #[test]
    fn completion_omits_operators_and_cut() {
        let all = compute("p :- ", pos(0, 5));
        let names: Vec<_> = all.iter().map(|i| i.label.as_str()).collect();
        assert!(!names.contains(&"!"), "cut should not complete: {names:?}");
        assert!(!names.contains(&"=.."), "univ should not complete");
        assert!(!names.contains(&"\\+"), "naf op should not complete");
    }

    // Redefining a stdlib predicate in the buffer: user wins, stdlib copy gone.
    #[test]
    fn completion_user_predicate_shadows_stdlib() {
        // member/2 is in stdlib. Redefine it in the buffer.
        let src = "member(custom, _).\np :- mem";
        let items = compute(src, pos(1, 8));
        let member_entries: Vec<_> = items
            .iter()
            .filter(|i| i.label == "member" && i.detail.as_deref() == Some("member/2"))
            .collect();
        assert_eq!(member_entries.len(), 1, "expected 1 user-source `member`");
        // And the stdlib entry should be absent.
        let stdlib_member = items
            .iter()
            .find(|i| i.label == "member" && i.detail.as_deref() == Some("stdlib member/2"));
        assert!(stdlib_member.is_none(), "stdlib member should be shadowed");
    }
}
