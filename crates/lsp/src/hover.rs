//! Hover: one-liner for built-ins, clause head for user-defined predicates.
//!
//! Built-in descriptions come from `plg_shared::builtins::doc` — the same
//! table codegen and the runtime validate against. The lookup spans every
//! row, so any builtin whose NAME is an identifier docs here even if
//! completion ranks it differently. Pure-operator names (`=..`, `@<`,
//! `\+`) are NOT reachable: `word_at_position` extracts identifier runs
//! only, so the cursor never resolves to an operator token
//! (symbol-token hover would be an additive enhancement).

use plg_shared::{StringInterner, Term, builtins};
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::buffer::{parse_best_effort, predicate_indicator, word_at_position};

pub fn compute(content: &str, position: Position) -> Option<Hover> {
    let word = word_at_position(content, position)?;
    if word.is_empty() {
        return None;
    }

    let body = if let Some(doc) = builtins::doc(&word) {
        Some(format!("**`{word}`** (built-in)\n\n{doc}"))
    } else {
        lookup_user_clause_head(content, &word)
            .map(|head| format!("**`{word}`** (user-defined)\n\n```prolog\n{head}\n```"))
    };

    body.map(|value| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

/// Render the first matching clause head as a string, e.g. `violation(F, sensitive_field)`.
fn lookup_user_clause_head(content: &str, name: &str) -> Option<String> {
    let (clauses, _, interner) = parse_best_effort(content)?;
    for clause in &clauses {
        if let Some((n, _)) = predicate_indicator(&clause.head, &interner)
            && n == name
        {
            return Some(render_head(&clause.head, &interner));
        }
    }
    None
}

fn render_head(head: &Term, interner: &StringInterner) -> String {
    match head {
        Term::Atom(id) => interner.resolve(*id).to_string(),
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            let argv: Vec<String> = args.iter().map(|a| render_arg(a, interner)).collect();
            format!("{}({})", name, argv.join(", "))
        }
        _ => "<non-callable head>".to_string(),
    }
}

/// Render an argument compactly. Variables get an ad-hoc `_N` name; compound
/// args show only their functor + arity so the hover stays one-liner-ish.
fn render_arg(term: &Term, interner: &StringInterner) -> String {
    match term {
        Term::Atom(id) => interner.resolve(*id).to_string(),
        Term::Integer(n) => n.to_string(),
        Term::Float(f) => f.to_string(),
        Term::Var(id) => format!("_{id}"),
        Term::Compound { functor, args } => {
            format!("{}/{}", interner.resolve(*functor), args.len())
        }
        Term::List { .. } => "[...]".to_string(),
    }
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
    fn word_under_cursor_in_middle() {
        let w = word_at_position("hello world", pos(0, 8));
        assert_eq!(w.as_deref(), Some("world"));
    }

    #[test]
    fn word_under_cursor_at_start() {
        let w = word_at_position("findall(X, p(X), L)", pos(0, 0));
        assert_eq!(w.as_deref(), Some("findall"));
    }

    #[test]
    fn hover_on_builtin_shows_doc() {
        let h = compute("p :- findall(X, q(X), L).\n", pos(0, 8)).unwrap();
        match h.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("findall"));
                assert!(m.value.contains("built-in"));
            }
            _ => panic!("expected markup"),
        }
    }

    #[test]
    fn hover_on_user_predicate_shows_clause_head() {
        let src = "violation(F, sensitive_field) :- field(F).\np :- violation(X, R).\n";
        // Hover on `violation` in the second clause.
        let h = compute(src, pos(1, 6)).unwrap();
        match h.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("violation"));
                assert!(m.value.contains("user-defined"));
            }
            _ => panic!("expected markup"),
        }
    }

    #[test]
    fn hover_outside_a_word_returns_none() {
        assert!(compute("foo bar", pos(0, 3)).is_none());
    }

    #[test]
    fn hover_on_unknown_word_returns_none() {
        assert!(compute("zzzxyz nothing here", pos(0, 0)).is_none());
    }

    // Hover serves operator/word builtins that completion omits: `between`
    // is completable, but verify a word-form builtin docs correctly here.
    #[test]
    fn hover_on_between_shows_doc() {
        let h = compute("p :- between(1, 3, X).\n", pos(0, 8)).unwrap();
        match h.contents {
            HoverContents::Markup(m) => assert!(m.value.contains("between")),
            _ => panic!("expected markup"),
        }
    }

    // Regression: UTF-16 column must not be used as a byte index.
    #[test]
    fn word_at_position_with_multibyte_char_before_cursor() {
        // `é` is 2 bytes UTF-8 / 1 UTF-16 code unit. UTF-16 col 5 lands at
        // the second char of "findall" on this line.
        let content = "% é\nfindall(X, p(X), L)";
        let w = word_at_position(content, pos(1, 5));
        assert_eq!(w.as_deref(), Some("findall"));
    }
}
