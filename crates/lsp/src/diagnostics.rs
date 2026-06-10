//! Buffer → LSP diagnostics. Reuses `plg_frontend::Parser` so the
//! line/col data the parser already emits flows straight into editor
//! squiggles without a shadow parser (the rule carried from v1).
//!
//! The parser returns a single `String` of the form
//! `<message> at line N col M`. We parse that back out into a `Range` and
//! a stripped message. Structured spans in `plg-frontend` errors would
//! let us skip the string parse (and widen the squiggle to the lexeme),
//! but that is a separate, additive frontend change — see
//! docs/design/LSP_PORT.md delta-3.

use std::collections::BTreeMap;

use plg_frontend::{Parser, lint};
use plg_shared::{STDLIB_PL, StringInterner};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

const SOURCE: &str = "plgl";

pub fn compute(content: &str) -> Vec<Diagnostic> {
    let mut interner = StringInterner::new();
    // Seed the interner with the stdlib so its predicates (member/2,
    // append/3, …) count as defined — the compiler prepends stdlib for the
    // same reason. Parsed SEPARATELY so the buffer keeps its own line
    // numbers for parse-error positions.
    let stdlib = Parser::parse_program_with_directives(STDLIB_PL, &mut interner)
        .map(|(c, _)| c)
        .unwrap_or_default();
    match Parser::parse_program_with_directives(content, &mut interner) {
        // Parse OK → run the undefined-predicate lint. These are WARNINGS
        // in the editor (yellow), distinct from red parse errors: the
        // program still compiles and raises a catchable existence_error at
        // runtime per ISO; the warning just flags the likely typo. Strict
        // failure lives in `plgc --deny-undefined`, not here.
        Ok((clauses, directives)) => {
            let mut all = stdlib;
            all.extend(clauses);
            undefined_warnings(content, &all, &directives, &interner)
        }
        Err(msg) => vec![error_string_to_diagnostic(&msg, content)],
    }
}

/// One warning per call site of a predicate that is defined nowhere.
fn undefined_warnings(
    content: &str,
    clauses: &[plg_shared::Clause],
    directives: &plg_frontend::ProgramDirectives,
    interner: &StringInterner,
) -> Vec<Diagnostic> {
    // Distinct callee → its suggestion (the lint may report it from
    // several callers; the squiggle goes on the call sites, not callers).
    let mut callees: BTreeMap<(String, usize), Option<String>> = BTreeMap::new();
    for u in lint::undefined_calls(clauses, directives, interner) {
        callees.entry(u.callee).or_insert(u.suggestion);
    }

    let mut diags = Vec::new();
    for ((name, arity), suggestion) in callees {
        let mut message = format!("undefined predicate {name}/{arity}");
        if let Some(s) = &suggestion {
            message.push_str(&format!(" — did you mean {s}?"));
        }
        for range in call_site_ranges(content, &name, arity) {
            diags.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some(SOURCE.to_string()),
                message: message.clone(),
                ..Default::default()
            });
        }
    }
    diags
}

/// Find the ranges where `name` is used as a callable: an identifier run
/// equal to `name` that (for arity > 0) is immediately followed by `(`.
/// Columns are UTF-16 code units per the LSP convention.
fn call_site_ranges(content: &str, name: &str, arity: usize) -> Vec<Range> {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    let mut ranges = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let mut col_u16: u32 = 0;
        let mut word_start_u16: Option<u32> = None;
        let mut word_start_byte = 0usize;
        let mut emit = |start: u32, end_u16: u32, end_byte: usize, word: &str| {
            if word == name {
                // arity > 0 must be a compound: next non-space char is `(`.
                let follows_paren = line[end_byte..].trim_start().starts_with('(');
                if arity == 0 || follows_paren {
                    ranges.push(Range {
                        start: Position {
                            line: line_idx as u32,
                            character: start,
                        },
                        end: Position {
                            line: line_idx as u32,
                            character: end_u16,
                        },
                    });
                }
            }
        };
        for (b, ch) in line.char_indices() {
            if is_id(ch) {
                if word_start_u16.is_none() {
                    word_start_u16 = Some(col_u16);
                    word_start_byte = b;
                }
            } else if let Some(ws) = word_start_u16.take() {
                emit(ws, col_u16, b, &line[word_start_byte..b]);
            }
            col_u16 += ch.len_utf16() as u32;
        }
        if let Some(ws) = word_start_u16 {
            emit(ws, col_u16, line.len(), &line[word_start_byte..]);
        }
    }
    ranges
}

/// Parse `<message> at line N col M` into a `Diagnostic`. If the suffix is
/// missing for any reason, fall back to a whole-file diagnostic so the user
/// still sees the message rather than nothing.
fn error_string_to_diagnostic(msg: &str, content: &str) -> Diagnostic {
    if let Some((message, line, col)) = parse_at_line_col(msg) {
        let line_0 = line.saturating_sub(1) as u32;
        let col_0 = col.saturating_sub(1) as u32;
        // Single-char range. We could widen to the lexeme length once the
        // parser exposes the offending token's span, but a positional squiggle
        // is more useful than nothing.
        let range = Range {
            start: Position {
                line: line_0,
                character: col_0,
            },
            end: Position {
                line: line_0,
                character: col_0 + 1,
            },
        };
        return Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some(SOURCE.to_string()),
            message: message.to_string(),
            ..Default::default()
        };
    }
    Diagnostic {
        range: whole_file_range(content),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(SOURCE.to_string()),
        message: msg.to_string(),
        ..Default::default()
    }
}

/// Extract the `(message, line, col)` triple from the parser's
/// `... at line N col M` convention. Returns `None` if the trailer is absent.
fn parse_at_line_col(msg: &str) -> Option<(&str, usize, usize)> {
    // Walk back from the end looking for the last " at line " substring.
    let idx = msg.rfind(" at line ")?;
    let (head, tail) = msg.split_at(idx);
    let tail = tail.trim_start_matches(" at line ");
    let (line_s, rest) = tail.split_once(" col ")?;
    let line: usize = line_s.trim().parse().ok()?;
    let col: usize = rest.trim().parse().ok()?;
    Some((head, line, col))
}

fn whole_file_range(content: &str) -> Range {
    let lines: Vec<&str> = content.lines().collect();
    let last_line = lines.len().saturating_sub(1) as u32;
    let last_col = lines.last().map(|l| l.len()).unwrap_or(0) as u32;
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: last_line,
            character: last_col,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_at_line_col_trailer() {
        let (msg, line, col) = parse_at_line_col("unexpected `]` at line 2 col 11").unwrap();
        assert_eq!(msg, "unexpected `]`");
        assert_eq!(line, 2);
        assert_eq!(col, 11);
    }

    #[test]
    fn parses_expected_form() {
        let (msg, line, col) =
            parse_at_line_col("expected `)`, got end of input at line 1 col 4").unwrap();
        assert_eq!(msg, "expected `)`, got end of input");
        assert_eq!(line, 1);
        assert_eq!(col, 4);
    }

    #[test]
    fn falls_back_when_no_trailer() {
        assert!(parse_at_line_col("some other error").is_none());
    }

    #[test]
    fn good_buffer_has_no_diagnostics() {
        assert!(compute("p(foo).\np(bar).\n").is_empty());
    }

    #[test]
    fn syntax_error_produces_positioned_diagnostic() {
        let diags = compute("p(foo).\ngo :- bar(]).\n");
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        // line 2 col 11 (1-indexed) → line 1 col 10 (0-indexed).
        assert_eq!(d.range.start.line, 1);
        assert_eq!(d.range.start.character, 10);
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert!(d.message.contains("`]`"), "message: {}", d.message);
    }

    #[test]
    fn diagnostic_uses_surface_lexeme_not_internal_variant() {
        // Regression: error messages must not leak TokenKind variant names.
        let diags = compute("go :- bar(]).\n");
        assert!(!diags[0].message.contains("RBracket"));
    }

    #[test]
    fn undefined_predicate_is_a_warning_on_the_call_site() {
        // parent/1 defined; ancestor's body calls the typo xarent/1.
        let src = "parent(tom).\nancestor(X) :- xarent(X).\n";
        let diags = compute(src);
        assert_eq!(diags.len(), 1, "{diags:?}");
        let d = &diags[0];
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(d.range.start.line, 1, "squiggle on the call site line");
        // Range covers `xarent` — `ancestor(X) :- ` is 15 chars, so 15..21.
        assert_eq!(d.range.start.character, 15);
        assert_eq!(d.range.end.character, 21);
        assert!(d.message.contains("xarent/1"), "{}", d.message);
        assert!(
            d.message.contains("did you mean parent/1?"),
            "{}",
            d.message
        );
    }

    #[test]
    fn defined_and_builtin_calls_produce_no_warnings() {
        // member/2 is stdlib... but compute() parses only the buffer (no
        // stdlib), so use a self-defined predicate + a builtin here.
        let src = "greet(X) :- helper(X), write(X).\nhelper(_).\n";
        assert!(compute(src).is_empty(), "{:?}", compute(src));
    }
}
