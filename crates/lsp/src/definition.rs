//! `textDocument/definition` — jump to the FIRST clause that defines the
//! identifier under the cursor.
//!
//! We return a single `Scalar(Location)` even for multi-clause predicates
//! because Neovim's default handler treats array responses as a multi-result
//! ambiguity: it jumps to the first AND opens a quickfix-style list. The
//! muscle-memory expectation for `gd` is a direct jump, so we return the
//! canonical (first) clause and let users find other clauses via references.
//!
//! Built-ins / stdlib predicates return `None` — their source isn't in the
//! user's buffer.
//!
//! Implementation note: we scan the buffer's text directly rather than going
//! through the parser. The parser doesn't track source byte spans, and
//! definition lookup is a structural-text problem (line-aware "does this
//! line start a clause for `<name>`?") that's both faster and more tolerant
//! of mid-edit buffers without it.

use plg_shared::BUILTINS;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};

use crate::buffer::word_at_position;
use crate::completion::stdlib_predicates;

pub fn compute(content: &str, position: Position, uri: &Url) -> Option<GotoDefinitionResponse> {
    let name = word_at_position(content, position)?;
    if name.is_empty() || is_engine_provided(&name) {
        return None;
    }
    // Always Scalar of the first match. An Array response would make
    // Neovim's default handler open a quickfix buffer alongside the jump,
    // which is jarring for the `gd` flow. Discovering other clauses is what
    // `textDocument/references` is for.
    find_first_clause_head(content, &name, uri).map(GotoDefinitionResponse::Scalar)
}

/// True if `name` is a built-in or stdlib predicate — no source in the
/// user's buffer to jump to. Builtins come from the shared vocabulary
/// table; stdlib names are derived from the embedded `STDLIB_PL` (so the
/// set stays correct as the stdlib evolves — no hardcoded list to drift).
fn is_engine_provided(name: &str) -> bool {
    if BUILTINS.iter().any(|s| s.name == name) {
        return true;
    }
    stdlib_predicates().iter().any(|(n, _)| n == name)
}

/// Walk lines, return the first clause head matching `name`. Stops at the
/// first match — we deliberately don't surface the rest, see `compute`.
fn find_first_clause_head(content: &str, name: &str, uri: &Url) -> Option<Location> {
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(col) = head_match_column(line, name) {
            return Some(Location {
                uri: uri.clone(),
                range: Range {
                    start: Position {
                        line: line_idx as u32,
                        character: col as u32,
                    },
                    end: Position {
                        line: line_idx as u32,
                        character: (col + name.chars().count()) as u32,
                    },
                },
            });
        }
    }
    None
}

/// Returns the byte column where `name` starts on `line`, if and only if
/// the line begins a clause whose head is `name`. Otherwise `None`.
fn head_match_column(line: &str, name: &str) -> Option<usize> {
    let leading_ws = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    if !trimmed.starts_with(name) {
        return None;
    }
    let after = &trimmed[name.len()..];
    let next = after.chars().next();
    let is_head = match next {
        Some('(') => true,
        Some('.') => true,
        Some(' ') | Some('\t') => {
            let rest = after.trim_start();
            rest.starts_with(":-") || rest.starts_with('.') || rest.is_empty()
        }
        None => true, // atom-only clause at EOL
        _ => false,
    };
    if is_head { Some(leading_ws) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri() -> Url {
        Url::parse("file:///tmp/test.pl").unwrap()
    }

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn scalar(resp: GotoDefinitionResponse) -> Location {
        match resp {
            GotoDefinitionResponse::Scalar(loc) => loc,
            other => panic!("expected Scalar, got {other:?}"),
        }
    }

    #[test]
    fn finds_single_compound_definition() {
        let src = "violation(F, sensitive_field) :- field(F).\n";
        let resp = compute(src, pos(0, 3), &uri()).expect("definition");
        let loc = scalar(resp);
        assert_eq!(loc.range.start, pos(0, 0));
        assert_eq!(loc.range.end, pos(0, 9));
    }

    // Multi-clause predicates: only the FIRST head is returned. Returning an
    // array would make Neovim's default `vim.lsp.buf.definition()` handler
    // open a quickfix buffer alongside the jump, breaking the `gd` muscle
    // memory.
    #[test]
    fn returns_first_clause_for_multi_clause_predicate() {
        let src = "sensitive(ssn).\nsensitive(password).\nuse :- sensitive(ssn).\n";
        let resp = compute(src, pos(2, 10), &uri()).expect("definition");
        let loc = scalar(resp);
        assert_eq!(loc.range.start.line, 0, "should jump to FIRST clause");
    }

    #[test]
    fn finds_atom_only_clause() {
        let src = "red.\nblue.\n";
        let resp = compute(src, pos(0, 1), &uri()).expect("definition");
        let loc = scalar(resp);
        assert_eq!(loc.range.start, pos(0, 0));
    }

    #[test]
    fn skips_builtins() {
        // `findall` is built-in; goto-def should return None.
        let src = "p :- findall(X, q(X), L).\n";
        assert!(compute(src, pos(0, 7), &uri()).is_none());
    }

    #[test]
    fn skips_stdlib_predicates() {
        let src = "p :- member(X, [1,2,3]).\n";
        assert!(compute(src, pos(0, 7), &uri()).is_none());
    }

    #[test]
    fn returns_none_when_no_definition_exists() {
        let src = "p :- undefined_predicate(X).\n";
        // `undefined_predicate` has no clause head anywhere in the file.
        assert!(compute(src, pos(0, 10), &uri()).is_none());
    }

    #[test]
    fn skips_uses_inside_clause_bodies() {
        // `violation` used on line 1 but defined only on line 0 → jump to 0.
        let src = "violation(X) :- field(X).\np :- violation(foo).\n";
        let loc = scalar(compute(src, pos(1, 8), &uri()).expect("definition"));
        assert_eq!(loc.range.start.line, 0);
    }

    #[test]
    fn skips_partial_word_matches() {
        let src = "not_violation(X) :- true.\nviolation(X) :- field(X).\n";
        let loc = scalar(compute(src, pos(1, 3), &uri()).expect("definition"));
        assert_eq!(loc.range.start.line, 1);
    }

    #[test]
    fn skips_comments_that_mention_the_name() {
        let src = "% violation(X) — old shape\nviolation(X) :- field(X).\n";
        let loc = scalar(compute(src, pos(1, 3), &uri()).expect("definition"));
        assert_eq!(loc.range.start.line, 1, "should not match the comment");
    }
}
