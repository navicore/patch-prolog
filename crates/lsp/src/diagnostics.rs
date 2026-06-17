//! Buffer → LSP diagnostics. Reuses `plg_frontend::Parser` so the source
//! positions the parser already tracks flow straight into editor squiggles
//! without a shadow parser (the rule carried from v1).
//!
//! Parse errors carry a byte `Span`; we resolve it to an LSP `Range` via
//! `SourceMap` and underline the offending lexeme directly — no string
//! trailer to parse back out.

use std::collections::BTreeMap;

use plg_frontend::{CallSite, ParseError, Parser, SourceMap, lint};
use plg_shared::{STDLIB_PL, Span, StringInterner};
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
    match Parser::parse_program_with_spans(content, &mut interner) {
        // Parse OK → run the undefined-predicate lint. These are WARNINGS
        // in the editor (yellow), distinct from red parse errors: the
        // program still compiles and raises a catchable existence_error at
        // runtime per ISO; the warning just flags the likely typo. Strict
        // failure lives in `plgc --deny-undefined`, not here.
        Ok((clauses, directives, call_sites)) => {
            let mut all = stdlib;
            all.extend(clauses);
            undefined_warnings(content, &all, &directives, &interner, &call_sites)
        }
        Err(err) => vec![parse_error_to_diagnostic(&err, content)],
    }
}

/// One warning per call site of a predicate that is defined nowhere. Call
/// sites come from the parser's recorded atom-functor occurrences (real AST
/// nodes), so a name appearing only in a comment never gets a squiggle.
fn undefined_warnings(
    content: &str,
    clauses: &[plg_shared::Clause],
    directives: &plg_frontend::ProgramDirectives,
    interner: &StringInterner,
    call_sites: &[CallSite],
) -> Vec<Diagnostic> {
    // Distinct callee → its suggestion (the lint may report it from
    // several callers; the squiggle goes on the call sites, not callers).
    let mut callees: BTreeMap<(String, usize), Option<String>> = BTreeMap::new();
    for u in lint::undefined_calls(clauses, directives, interner) {
        callees.entry(u.callee).or_insert(u.suggestion);
    }

    let sm = SourceMap::new(content);
    let mut diags = Vec::new();
    for ((name, arity), suggestion) in callees {
        let mut message = format!("undefined predicate {name}/{arity}");
        if let Some(s) = &suggestion {
            message.push_str(&format!(" — did you mean {s}?"));
        }
        for cs in call_sites {
            if cs.arity == arity && interner.resolve(cs.functor) == name {
                diags.push(Diagnostic {
                    range: span_to_range(&sm, cs.span),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some(SOURCE.to_string()),
                    message: message.clone(),
                    ..Default::default()
                });
            }
        }
    }
    diags
}

/// Map a byte `Span` to an LSP `Range` (UTF-16 columns) via the `SourceMap`.
fn span_to_range(sm: &SourceMap, span: Span) -> Range {
    let (start_line, start_char) = sm.utf16_position(span.lo);
    let (end_line, end_char) = sm.utf16_position(span.hi);
    Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    }
}

/// Map a `ParseError`'s byte span to an LSP `Range` via `SourceMap`,
/// underlining the offending lexeme (or a point at end-of-input).
fn parse_error_to_diagnostic(err: &ParseError, content: &str) -> Diagnostic {
    Diagnostic {
        range: span_to_range(&SourceMap::new(content), err.span),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(SOURCE.to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn comment_mention_does_not_squiggle_only_the_real_call() {
        // Checkpoint 2: `xarent` appears in a comment AND as a real undefined
        // call. Squiggles come from parsed AST occurrences, so the comment
        // mention is invisible — exactly one warning, on the call.
        let src = "parent(tom).\n% xarent is a typo for parent\nq :- xarent(tom).\n";
        let diags = compute(src);
        assert_eq!(diags.len(), 1, "{diags:?}");
        let d = &diags[0];
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
        // Line 2 (0-indexed) is the call; the comment is line 1.
        assert_eq!(d.range.start.line, 2, "squiggle on the call, not the comment");
        // `q :- ` is 5 chars, so `xarent` is 5..11.
        assert_eq!(d.range.start.character, 5);
        assert_eq!(d.range.end.character, 11);
        assert!(d.message.contains("xarent/1"), "{}", d.message);
    }
}
