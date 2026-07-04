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
    pub fn emit_atom_table(&mut self) {
        let count = self.interner.len();
        for (i, name) in self.interner.iter().enumerate() {
            let bytes = name.as_bytes();
            writeln!(
                self.out,
                "@plg_atom_{i} = private unnamed_addr constant [{} x i8] c\"{}\\00\"",
                bytes.len() + 1,
                escape_ir_string(bytes)
            )
            .unwrap();
        }
        let refs: Vec<String> = (0..count).map(|i| format!("ptr @plg_atom_{i}")).collect();
        writeln!(
            self.out,
            "@plg_atom_strs = internal constant [{count} x ptr] [{}]",
            refs.join(", ")
        )
        .unwrap();
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
        let arr = if n == 0 {
            String::new()
        } else {
            refs.join(", ")
        };
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
        for i in 0..self.files.len() {
            let bytes = self.files[i].as_bytes();
            writeln!(
                self.out,
                "@plg_file_{i} = private unnamed_addr constant [{} x i8] c\"{}\\00\"",
                bytes.len() + 1,
                escape_ir_string(bytes)
            )
            .unwrap();
        }
        let frefs: Vec<String> = (0..self.files.len())
            .map(|i| format!("ptr @plg_file_{i}"))
            .collect();
        writeln!(
            self.out,
            "@plg_files = internal constant [{} x ptr] [{}]",
            self.files.len(),
            frefs.join(", ")
        )
        .unwrap();

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
}
