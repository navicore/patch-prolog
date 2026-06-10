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

use plg_frontend::Parser;
use plg_shared::StringInterner;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

const SOURCE: &str = "plgl";

pub fn compute(content: &str) -> Vec<Diagnostic> {
    let mut interner = StringInterner::new();
    match Parser::parse_program_with_directives(content, &mut interner) {
        Ok(_) => Vec::new(),
        Err(msg) => vec![error_string_to_diagnostic(&msg, content)],
    }
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
}
