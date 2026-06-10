//! plgc compiler library
//!
//! Compiles ISO-subset Prolog (.pl) to standalone native binaries:
//! parse → analyze → codegen (LLVM IR text) → clang link against the
//! embedded `libplg_runtime.a`. Users need clang (≥ 15), never Rust.
//!
//! The embed/link machinery is ported from patch-seq's proven pattern.

pub mod codegen;
pub mod link;

use plg_frontend::{Parser, ProgramDirectives};
use plg_shared::{Clause, StringInterner};
use std::path::Path;

/// Embedded runtime library (built by build.rs from plg-runtime).
pub static RUNTIME_LIB: &[u8] = include_bytes!(env!("PLG_RUNTIME_LIB_PATH"));

/// Arity ceiling for the argument-register ABI (mirrors the runtime's
/// MAX_ARGS).
pub const MAX_GOAL_ARITY: usize = 16;

/// Optimization level passed through to clang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptLevel {
    O0,
    #[default]
    O3,
}

/// The embedded standard library source now lives in `plg-shared`
/// (language definition, shared with the LSP); re-exported here for
/// compatibility with existing `plgc::STDLIB_PL` users.
pub use plg_shared::STDLIB_PL;

/// Parse each source against a shared interner (v1 pattern: line/col
/// reports stay relative to the originating file).
fn parse_sources(
    sources: &[&Path],
) -> Result<(Vec<Clause>, ProgramDirectives, StringInterner), String> {
    if sources.is_empty() {
        return Err("no input files".to_string());
    }
    let mut interner = StringInterner::new();
    let (mut clauses, mut directives) =
        Parser::parse_program_with_directives(STDLIB_PL, &mut interner)
            .map_err(|e| format!("internal: stdlib parse error: {e}"))?;
    for path in sources {
        let mut src = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: cannot read file: {e}", path.display()))?;
        // Script mode: a leading `#!/usr/bin/env plgc` line is not
        // Prolog; blank it out (preserving line numbers in errors).
        if src.starts_with("#!") {
            let eol = src.find('\n').unwrap_or(src.len());
            src.replace_range(..eol, "");
        }
        let (mut cs, ds) = Parser::parse_program_with_directives(&src, &mut interner)
            .map_err(|msg| format_parse_error(path, &msg))?;
        clauses.append(&mut cs);
        directives.dynamic.extend(ds.dynamic);
    }
    Ok((clauses, directives, interner))
}

/// Compile one or more .pl source files to a standalone executable.
pub fn compile_files(
    sources: &[&Path],
    output_path: &Path,
    keep_ir: bool,
    opt: OptLevel,
) -> Result<(), String> {
    let (clauses, directives, interner) = parse_sources(sources)?;
    let ir = codegen::codegen_program(&clauses, &directives, &interner)?;

    let ir_path = output_path.with_extension("ll");
    std::fs::write(&ir_path, &ir).map_err(|e| format!("Failed to write IR file: {e}"))?;

    let result = link::link_ir(&ir_path, output_path, opt);
    if !keep_ir {
        std::fs::remove_file(&ir_path).ok();
    }
    result
}

/// Compile source text to LLVM IR (golden-IR tests; no clang needed).
pub fn compile_to_ir(source: &str) -> Result<String, String> {
    let mut interner = StringInterner::new();
    let (clauses, directives) = Parser::parse_program_with_directives(source, &mut interner)
        .map_err(|e| format!("parse error: {e}"))?;
    codegen::codegen_program(&clauses, &directives, &interner)
}

/// Parse and statically check .pl sources without producing a binary.
///
/// A parse failure is reported as `path:line:col: <message>`; the
/// line/col are extracted from the frontend's error text when present.
/// Returns `Ok(())` only when every file parses cleanly.
pub fn check_files(sources: &[&Path]) -> Result<(), String> {
    parse_sources(sources).map(|_| ())
}

/// Render a frontend parse error as `path:line:col: message`. The
/// frontend embeds source coordinates as `... at line N col M`; lift
/// them into the conventional prefix so editors and CI can jump to the
/// offending token.
fn format_parse_error(path: &Path, msg: &str) -> String {
    if let Some((line, col)) = extract_line_col(msg) {
        format!("{}:{}:{}: {}", path.display(), line, col, msg)
    } else {
        format!("{}: {}", path.display(), msg)
    }
}

/// Pull `(line, col)` out of a frontend error message of the form
/// `... at line N col M`. Returns `None` if the pattern is absent.
fn extract_line_col(msg: &str) -> Option<(usize, usize)> {
    let rest = msg.rsplit_once("at line ")?.1;
    let (line_str, after) = rest.split_once(" col ")?;
    // `col` may be followed by trailing prose; take the leading digits.
    let col_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    let line = line_str.trim().parse().ok()?;
    let col = col_str.parse().ok()?;
    Some((line, col))
}
