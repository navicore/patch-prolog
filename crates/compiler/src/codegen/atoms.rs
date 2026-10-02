//! Emit the atom table and predicate registry as global IR data.
//!
//! The runtime rebuilds the compiler's exact atom-id space from
//! `@plg_atom_strs` at startup, and dispatches `--query` goals through
//! `@plg_registry` (sorted by functor id, then arity, for binary
//! search). This is what lets fully compiled predicates answer
//! arbitrary runtime queries.

use super::{CodeGen, GoalTarget};
use std::fmt::Write;

impl CodeGen<'_> {
    /// The atom table: per-atom string globals plus `@plg_atom_strs`,
    /// from which the runtime rebuilds the compiler's exact atom-id
    /// space at startup.
    pub fn emit_atom_table(&mut self) {
        let interner = self.interner;
        emit_string_table(&mut self.out, "plg_atom", "plg_atom_strs", interner.iter());
    }

    /// Registry rows: every defined predicate plus `:- dynamic`
    /// declarations with no clauses (silent-fail stubs).
    pub fn emit_registry(&mut self) -> usize {
        let mut rows: Vec<(u32, u32, String)> = Vec::new();
        let keys: Vec<_> = self.predicates.keys().copied().collect();
        for (f, a) in keys {
            match self.how_to_call(f, a) {
                GoalTarget::Defined => rows.push((f, a, format!("@{}", self.pred_symbol(f, a)))),
                _ => unreachable!("predicates map holds defined entries"),
            }
        }
        for &(f, a) in &self.dynamic_only {
            rows.push((f, a, "@plg_rt_pred_fail".to_string()));
        }
        rows.sort_by_key(|(f, a, _)| (*f, *a));
        rows.dedup_by_key(|(f, a, _)| (*f, *a));

        writeln!(self.out, "%RegEntry = type {{ i32, i32, ptr }}").unwrap();
        let entries: Vec<String> = rows
            .iter()
            .map(|(f, a, sym)| format!("%RegEntry {{ i32 {f}, i32 {a}, ptr {sym} }}"))
            .collect();
        writeln!(
            self.out,
            "@plg_registry = internal constant [{} x %RegEntry] [{}]",
            rows.len(),
            entries.join(", ")
        )
        .unwrap();
        rows.len()
    }

    /// The wire-encoding capability table. A `@plg_caps`
    /// array of pointers to runtime `EncoderDesc` statics — one per encoding
    /// the program declared via `:- io_format([...])`, default `[text]`. The
    /// runtime scans it to resolve `--format`; encoders NOT listed here are
    /// unreferenced by this binary, so link-time `--gc-sections` strips their
    /// code. The default advertises BOTH core formats (`[text, bson]`) — bson
    /// is a first-class engine format, not an opt-in feature, so a freshly-built
    /// binary speaks it without ceremony. A program declares `io_format` to
    /// RESTRICT (e.g. `[text]` for a deliberately text-only minimal binary, or
    /// `[bson]` to force bson-only); that restriction is what sheds the other
    /// encoder via dead-stripping. Returns the table length for the `plg_rt_init`
    /// handoff.
    pub fn emit_capabilities(&mut self, declared: &[String]) -> Result<usize, String> {
        let defaults: [&str; 2] = ["text", "bson"];
        let names: Vec<&str> = if declared.is_empty() {
            defaults.to_vec()
        } else {
            declared.iter().map(|s| s.as_str()).collect()
        };
        // Map declared names to descriptor symbols, validating + deduping as
        // we go (codegen owns dedup since it sees the cross-file merge).
        let mut syms: Vec<&str> = Vec::new();
        for n in &names {
            let sym = match *n {
                "text" => "@PLG_ENC_TEXT",
                "bson" => "@PLG_ENC_BSON",
                other => {
                    return Err(format!(
                        "io_format: unknown encoder `{other}` (known: text, bson)"
                    ));
                }
            };
            if !syms.contains(&sym) {
                syms.push(sym);
            }
        }
        let refs: Vec<String> = syms.iter().map(|s| format!("ptr {s}")).collect();
        let n = syms.len();
        let arr = refs.join(", ");
        writeln!(
            self.out,
            "@plg_caps = internal constant [{n} x ptr] [{arr}]"
        )
        .unwrap();
        Ok(n)
    }

    /// Source-location side-table (SPANS.md Layer 3). Emitted AFTER the
    /// predicates, since `site_id` accumulates the rows during clause
    /// emission. Returns `(srcmap_len, files_len)` for the `plg_rt_init`
    /// handoff. Both are `0` when nothing raises with provenance — the empty
    /// tables cost ~0 bytes.
    pub fn emit_provenance(&mut self) -> (usize, usize) {
        emit_string_table(
            &mut self.out,
            "plg_file",
            "plg_files",
            self.files.iter().map(|s| s.as_str()),
        );

        writeln!(self.out, "%SrcLoc = type {{ i32, i32, i32 }}").unwrap();
        let rows: Vec<String> = self
            .srcmap
            .iter()
            .map(|(f, l, c)| format!("%SrcLoc {{ i32 {f}, i32 {l}, i32 {c} }}"))
            .collect();
        writeln!(
            self.out,
            "@plg_srcmap = internal constant [{} x %SrcLoc] [{}]",
            rows.len(),
            rows.join(", ")
        )
        .unwrap();
        (self.srcmap.len(), self.files.len())
    }
}

/// Emit one NUL-terminated string per index as `@{prefix}_{i}` globals,
/// plus the `@{array}` pointer table indexing them.
fn emit_string_table<'a>(
    out: &mut String,
    prefix: &str,
    array: &str,
    names: impl Iterator<Item = &'a str>,
) {
    let mut refs = Vec::new();
    for (i, name) in names.enumerate() {
        let bytes = name.as_bytes();
        writeln!(
            out,
            "@{prefix}_{i} = private unnamed_addr constant [{} x i8] c\"{}\\00\"",
            bytes.len() + 1,
            escape_ir_string(bytes)
        )
        .unwrap();
        refs.push(format!("ptr @{prefix}_{i}"));
    }
    writeln!(
        out,
        "@{array} = internal constant [{} x ptr] [{}]",
        refs.len(),
        refs.join(", ")
    )
    .unwrap();
}

/// LLVM IR c"..." escaping: printable ASCII except `"` and `\` stays
/// literal; everything else becomes \\HH.
fn escape_ir_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_string_escaping() {
        assert_eq!(escape_ir_string(b"abc"), "abc");
        assert_eq!(escape_ir_string(b"a\"b\\c\n"), "a\\22b\\5Cc\\0A");
    }

    use crate::codegen::CodeGen;
    use plg_shared::StringInterner;

    /// Run `emit_capabilities` on a fresh CodeGen; returns (result,
    /// emitted IR).
    fn caps_ir(declared: &[String]) -> (Result<usize, String>, String) {
        let interner = StringInterner::new();
        let mut cg = CodeGen::new(&interner, &[]);
        let r = cg.emit_capabilities(declared);
        (r, cg.out)
    }

    #[test]
    fn caps_default_advertises_both_core_formats() {
        let (r, out) = caps_ir(&[]);
        assert_eq!(r.unwrap(), 2);
        assert_eq!(
            out,
            "@plg_caps = internal constant [2 x ptr] [ptr @PLG_ENC_TEXT, ptr @PLG_ENC_BSON]\n"
        );
    }

    #[test]
    fn caps_io_format_restricts() {
        let (r, out) = caps_ir(&["text".to_string()]);
        assert_eq!(r.unwrap(), 1);
        assert_eq!(
            out,
            "@plg_caps = internal constant [1 x ptr] [ptr @PLG_ENC_TEXT]\n"
        );

        let (r, out) = caps_ir(&["bson".to_string()]);
        assert_eq!(r.unwrap(), 1);
        assert_eq!(
            out,
            "@plg_caps = internal constant [1 x ptr] [ptr @PLG_ENC_BSON]\n"
        );
    }

    #[test]
    fn caps_dedups_repeated_names_in_first_seen_order() {
        let (r, out) = caps_ir(&["bson".to_string(), "text".to_string(), "bson".to_string()]);
        assert_eq!(r.unwrap(), 2);
        // Dedup keeps first appearance: bson was seen before text.
        assert_eq!(
            out,
            "@plg_caps = internal constant [2 x ptr] [ptr @PLG_ENC_BSON, ptr @PLG_ENC_TEXT]\n"
        );
    }

    #[test]
    fn caps_unknown_encoder_is_an_error() {
        let (r, _) = caps_ir(&["cbor".to_string()]);
        let err = r.unwrap_err();
        assert!(err.contains("unknown encoder `cbor`"), "{err}");
        assert!(err.contains("known: text, bson"), "{err}");
    }
}
