//! Buffer-walking helpers shared by `completion` and `hover`.
//!
//! - LSP-position ↔ byte-offset conversion that respects the UTF-16 code-unit
//!   convention.
//! - Heuristic "where did the last complete clause end?" so mid-edit buffers
//!   can still produce useful results when the trailing clause is unfinished.
//! - Tiny clause-head ↔ `(name, arity)` projector used by both consumers.

use plg_frontend::{Parser, ProgramDirectives};
use plg_shared::{Clause, StringInterner, Term};
use tower_lsp::lsp_types::Position;

/// Map an LSP `(line, character)` Position to a byte offset within `content`.
///
/// LSP positions default to UTF-16 code units, so naively using `character`
/// as a byte index panics on any line containing multibyte chars before the
/// cursor. This walks chars, accumulating `len_utf16()` per char, and stops
/// when reaching `position.character`. Past-end-of-line clamps to the line's
/// end; landing inside a surrogate pair clamps to the char boundary.
///
/// Returns `None` only if `position.line` exceeds the buffer's line count.
pub fn position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
    let line_start = nth_line_byte_offset(content, position.line as usize)?;
    let line_end = content[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(content.len());
    let line = &content[line_start..line_end];

    let target = position.character as usize;
    let mut consumed_utf16: usize = 0;
    for (offset, ch) in line.char_indices() {
        if consumed_utf16 >= target {
            return Some(line_start + offset);
        }
        consumed_utf16 += ch.len_utf16();
    }
    Some(line_end)
}

/// Byte offset of the start of the `n`-th line (0-indexed). Returns `None`
/// if the line index exceeds the buffer's line count.
fn nth_line_byte_offset(content: &str, n: usize) -> Option<usize> {
    if n == 0 {
        return Some(0);
    }
    let mut count = 0;
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            count += 1;
            if count == n {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Parse `content` as a Prolog program, recovering as many clauses as
/// possible even when the buffer is mid-edit.
///
/// Strategy:
/// 1. Fast path — try the whole buffer. If it parses, return everything.
/// 2. Otherwise, walk clause-terminator boundaries (`.` followed by
///    whitespace or EOF) and parse each `[prev_terminator..next_terminator]`
///    chunk independently. Chunks that don't parse (the one the user is
///    currently typing in) are silently skipped; successfully-parsed chunks
///    contribute their clauses.
///
/// This handles the mid-buffer completion case (a half-typed clause anywhere
/// in the file no longer hides the rest from completion sources) without
/// needing to know where the cursor is. It also tolerates multiple broken
/// clauses, not just one.
pub fn parse_best_effort(
    content: &str,
) -> Option<(Vec<Clause>, ProgramDirectives, StringInterner)> {
    let mut interner = StringInterner::new();
    if let Ok((c, d)) = Parser::parse_program_with_directives(content, &mut interner) {
        return Some((c, d, interner));
    }

    let mut interner = StringInterner::new();
    let mut all_clauses: Vec<Clause> = Vec::new();
    let mut all_directives = ProgramDirectives::default();

    for chunk in clause_chunks(content) {
        if let Ok((c, d)) = Parser::parse_program_with_directives(chunk, &mut interner) {
            all_clauses.extend(c);
            all_directives.dynamic.extend(d.dynamic);
        }
    }

    if all_clauses.is_empty() && all_directives.dynamic.is_empty() {
        None
    } else {
        Some((all_clauses, all_directives, interner))
    }
}

/// Split `content` into candidate-clause chunks. A chunk ends at either:
/// - A clause terminator (`.` followed by whitespace or EOF), inclusive, OR
/// - The position immediately before what looks like a new clause head:
///   a `\n` followed by a column-0 lowercase letter, `_`, or `:` (covers
///   normal clause heads and `:-` directives).
///
/// The second boundary is what prevents a broken clause and the next
/// well-formed one from being glued into a single failing chunk — without
/// it, mid-buffer recovery would only work when the broken clause stood
/// alone between two real ones.
fn clause_chunks(content: &str) -> Vec<&str> {
    let bytes = content.as_bytes();
    let mut chunks: Vec<&str> = Vec::new();
    let mut chunk_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        // (A) Clause terminator `.\s` or `.<EOF>`.
        if bytes[i] == b'.' {
            let is_terminator = matches!(
                bytes.get(i + 1),
                None | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
            );
            if is_terminator {
                let chunk_end = i + 1;
                if chunk_end > chunk_start {
                    chunks.push(&content[chunk_start..chunk_end]);
                }
                chunk_start = chunk_end;
                i += 1;
                continue;
            }
        }

        // (B) Heuristic clause-start: `\n` followed at column 0 by a
        // lowercase identifier or `:` (directive prefix). Splits a broken
        // clause off from the next well-formed one.
        if bytes[i] == b'\n'
            && let Some(&next) = bytes.get(i + 1)
            && (next.is_ascii_lowercase() || next == b'_' || next == b':')
        {
            let chunk_end = i + 1;
            if chunk_end > chunk_start {
                chunks.push(&content[chunk_start..chunk_end]);
            }
            chunk_start = chunk_end;
        }

        i += 1;
    }
    chunks
}

/// Find the word under the cursor — the maximal identifier-character run
/// (alphanumeric + underscore) spanning the cursor's byte position. Returns
/// `None` if the cursor lands on a non-identifier character. Honors LSP's
/// UTF-16 position convention so multibyte chars on the line don't produce
/// wrong words.
pub fn word_at_position(content: &str, position: Position) -> Option<String> {
    let byte_pos = position_to_byte_offset(content, position)?;
    let line_start = content[..byte_pos]
        .rfind('\n')
        .map(|nl| nl + 1)
        .unwrap_or(0);
    let line_end = content[byte_pos..]
        .find('\n')
        .map(|i| byte_pos + i)
        .unwrap_or(content.len());
    let line = &content[line_start..line_end];
    let col = byte_pos - line_start;

    let is_id = |c: char| c.is_alphanumeric() || c == '_';

    let prefix = &line[..col];
    let start_offset_in_line = prefix
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_id(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(col);

    let suffix = &line[col..];
    let end_offset_in_line = suffix
        .char_indices()
        .take_while(|(_, c)| is_id(*c))
        .last()
        .map(|(i, c)| col + i + c.len_utf8())
        .unwrap_or(col);

    if start_offset_in_line == end_offset_in_line {
        return None;
    }
    Some(line[start_offset_in_line..end_offset_in_line].to_string())
}

/// Project a clause head to its `(name, arity)`. `None` for non-callable
/// heads (variables, numbers, lists), which shouldn't normally appear but
/// might in malformed user input.
pub fn predicate_indicator(head: &Term, interner: &StringInterner) -> Option<(String, usize)> {
    match head {
        Term::Atom(id) => Some((interner.resolve(*id).to_string(), 0)),
        Term::Compound { functor, args } => {
            Some((interner.resolve(*functor).to_string(), args.len()))
        }
        _ => None,
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
    fn position_to_byte_offset_clamps_past_line_end() {
        let off = position_to_byte_offset("hello\nworld", pos(0, 99));
        assert_eq!(off, Some(5));
    }

    #[test]
    fn position_to_byte_offset_handles_emoji_surrogate_pair() {
        // 🐱 is 4 bytes in UTF-8, 2 UTF-16 code units (surrogate pair).
        let content = "🐱x";
        assert_eq!(position_to_byte_offset(content, pos(0, 2)), Some(4));
        assert_eq!(position_to_byte_offset(content, pos(0, 3)), Some(5));
    }

    #[test]
    fn position_to_byte_offset_handles_multibyte_ascii_mix() {
        // `é` is 2 bytes in UTF-8, 1 UTF-16 code unit.
        let content = "% é\nfoo";
        let line1_start = position_to_byte_offset(content, pos(1, 0)).unwrap();
        let at_3 = position_to_byte_offset(content, pos(1, 3)).unwrap();
        assert_eq!(at_3 - line1_start, 3);
    }

    #[test]
    fn position_to_byte_offset_returns_none_for_line_past_end() {
        assert_eq!(position_to_byte_offset("one line", pos(5, 0)), None);
    }

    #[test]
    fn parse_best_effort_recovers_after_unfinished_trailing_clause() {
        // First clause complete; second is mid-edit at the end.
        let src = "p(foo).\nq(bar) :- mem";
        let (clauses, _, interner) = parse_best_effort(src).expect("recovery");
        let names: Vec<_> = clauses
            .iter()
            .filter_map(|c| predicate_indicator(&c.head, &interner))
            .map(|(n, _)| n)
            .collect();
        assert!(names.iter().any(|n| n == "p"));
    }

    // Regression for the mid-buffer completion bug: an unfinished clause in
    // the middle of a buffer no longer hides predicates defined above OR
    // below it. The chunk-based recovery in `parse_best_effort` handles each
    // complete clause independently.
    #[test]
    fn parse_best_effort_recovers_above_and_below_a_broken_middle_clause() {
        let src = "above(X) :- field(X).\nbroken :- mem\nbelow(Y).\n";
        let (clauses, _, interner) = parse_best_effort(src).expect("recovery");
        let names: Vec<_> = clauses
            .iter()
            .filter_map(|c| predicate_indicator(&c.head, &interner))
            .map(|(n, _)| n)
            .collect();
        assert!(
            names.iter().any(|n| n == "above"),
            "expected `above`, got {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n == "below"),
            "expected `below` (below the broken clause), got {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == "broken"),
            "broken clause must not parse, got {:?}",
            names
        );
    }

    #[test]
    fn parse_best_effort_handles_multiple_broken_clauses() {
        let src = "a(1).\nb :- ?\nc(2).\nd :- $\ne(3).\n";
        let (clauses, _, interner) = parse_best_effort(src).expect("recovery");
        let names: Vec<_> = clauses
            .iter()
            .filter_map(|c| predicate_indicator(&c.head, &interner))
            .map(|(n, _)| n)
            .collect();
        // The three valid clauses round-trip; the two broken middles fall away.
        assert!(names.iter().any(|n| n == "a"), "got {:?}", names);
        assert!(names.iter().any(|n| n == "c"), "got {:?}", names);
        assert!(names.iter().any(|n| n == "e"), "got {:?}", names);
    }
}
