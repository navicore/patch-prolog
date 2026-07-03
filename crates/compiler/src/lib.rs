//! plgc compiler library
//!
//! Compiles ISO-subset Prolog (.pl) to standalone native binaries:
//! parse → analyze → codegen (LLVM IR text) → clang link against the
//! embedded `libplg_runtime.a`. Users need clang (≥ 15), never Rust.
//!
//! The embed/link machinery is ported from patch-seq's proven pattern.

pub mod codegen;
pub mod link;
pub mod worker_glue;

use codegen::CgSource;
use plg_frontend::{CgClause, ParseError, Parser, ProgramDirectives, SourceMap};
use plg_shared::{Clause, Span, Spanned, StringInterner};
use std::path::Path;

/// Embedded runtime library (built by build.rs from plg-runtime).
pub static RUNTIME_LIB: &[u8] = include_bytes!(env!("PLG_RUNTIME_LIB_PATH"));

/// Embedded `wasm32-wasip1` runtime archive — present only when plgc is built
/// `--features wasm` (build.rs sets `PLG_WASM_RUNTIME_LIB_PATH`). `None`
/// otherwise, so a `--target wasm32-wasi` request fails with a clear "rebuild
/// with --features wasm" message instead of a missing symbol.
#[cfg(feature = "wasm")]
pub static WASM_RUNTIME_LIB: Option<&[u8]> =
    Some(include_bytes!(env!("PLG_WASM_RUNTIME_LIB_PATH")));
#[cfg(not(feature = "wasm"))]
pub static WASM_RUNTIME_LIB: Option<&[u8]> = None;

/// Embedded `wasm32-unknown-unknown` reactor archive (Tier 2). Per D5 the one
/// `wasm` feature embeds *both* wasm archives, so this is `Some` exactly when
/// [`WASM_RUNTIME_LIB`] is — a `--target worker` request on a non-wasm build
/// fails with the same "rebuild with --features wasm" message.
#[cfg(feature = "wasm")]
pub static WORKER_RUNTIME_LIB: Option<&[u8]> =
    Some(include_bytes!(env!("PLG_WORKER_RUNTIME_LIB_PATH")));
#[cfg(not(feature = "wasm"))]
pub static WORKER_RUNTIME_LIB: Option<&[u8]> = None;

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

/// Compilation target. `Native` (the host) is the default; `Wasm` emits a
/// standalone `wasm32-wasi` CLI module (Tier 1 — see docs/design/done/WASM.md);
/// `Worker` emits a `wasm32-unknown-unknown` *reactor* module (Tier 2 —
/// no `main`, exports a `plg_init` + the `plg_rt_*` buffer ABI a JS host
/// drives, see docs/design/done/WASM_TIER2_PLAN.md). Both wasm targets require
/// plgc built `--features wasm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Native,
    Wasm,
    Worker,
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
        let src = read_source(path)?;
        let (mut cs, ds) = Parser::parse_program_with_directives(&src, &mut interner)
            .map_err(|e| format_parse_error(path, &src, &e))?;
        clauses.append(&mut cs);
        directives.dynamic.extend(ds.dynamic);
    }
    Ok((clauses, directives, interner))
}

/// Read a source file, blanking a leading `#!/usr/bin/env plgc` shebang line
/// (script mode) while preserving line numbers for diagnostics. Shared by the
/// lint/check path ([`parse_sources`]) and the codegen path
/// ([`parse_sources_cg`]) so the two can't drift on shebang handling.
fn read_source(path: &Path) -> Result<String, String> {
    let mut src = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: cannot read file: {e}", path.display()))?;
    if src.starts_with("#!") {
        let eol = src.find('\n').unwrap_or(src.len());
        src.replace_range(..eol, "");
    }
    Ok(src)
}

/// Codegen variant of [`parse_sources`]: clause bodies carry per-goal source
/// spans (SPANS.md Layer 3) and each user file becomes a `CgSource` (indexed
/// by `FileId`) so codegen can resolve call-site spans to `file:line:col`.
/// Stdlib clauses carry no provenance (their calls are all defined).
fn parse_sources_cg(
    sources: &[&Path],
) -> Result<
    (
        Vec<CgClause>,
        ProgramDirectives,
        StringInterner,
        Vec<CgSource>,
    ),
    String,
> {
    if sources.is_empty() {
        return Err("no input files".to_string());
    }
    let mut interner = StringInterner::new();
    let (stdlib, mut directives) = Parser::parse_program_with_directives(STDLIB_PL, &mut interner)
        .map_err(|e| format!("internal: stdlib parse error: {e}"))?;
    let mut clauses: Vec<CgClause> = stdlib.into_iter().map(clause_without_provenance).collect();
    let mut cg_sources: Vec<CgSource> = Vec::new();
    for path in sources {
        let src = read_source(path)?;
        let file_id = cg_sources.len() as u32;
        let (mut cs, ds) = Parser::parse_program_cg(&src, &mut interner, file_id)
            .map_err(|e| format_parse_error(path, &src, &e))?;
        clauses.append(&mut cs);
        directives.dynamic.extend(ds.dynamic);
        cg_sources.push(CgSource {
            path: path.display().to_string(),
            text: src,
        });
    }
    Ok((clauses, directives, interner, cg_sources))
}

/// Wrap a plain clause as a `CgClause` with no usable provenance: file id
/// `u32::MAX` is never a valid source index, so codegen emits `NO_SITE`.
fn clause_without_provenance(c: Clause) -> CgClause {
    let dummy = Span::point(u32::MAX, 0);
    CgClause {
        head: c.head,
        body: c.body.into_iter().map(|t| Spanned::new(t, dummy)).collect(),
    }
}

/// Compile one or more .pl source files to a standalone executable.
pub fn compile_files(
    sources: &[&Path],
    output_path: &Path,
    keep_ir: bool,
    opt: OptLevel,
    target: Target,
) -> Result<(), String> {
    let (clauses, directives, interner, cg_sources) = parse_sources_cg(sources)?;
    let ir = codegen::codegen_program(&clauses, &directives, &interner, &cg_sources, target)?;

    let ir_path = output_path.with_extension("ll");
    std::fs::write(&ir_path, &ir).map_err(|e| format!("Failed to write IR file: {e}"))?;

    let result = match target {
        Target::Native => link::link_ir(&ir_path, output_path, opt),
        Target::Wasm => link::link_wasm(&ir_path, output_path, opt),
        Target::Worker => link::link_wasm_reactor(&ir_path, output_path, opt),
    };
    if !keep_ir {
        std::fs::remove_file(&ir_path).ok();
    }
    result
}

/// Compile source text to native LLVM IR (golden-IR tests; no clang needed).
pub fn compile_to_ir(source: &str) -> Result<String, String> {
    compile_to_ir_target(source, Target::Native)
}

/// Compile source text to LLVM IR for a specific [`Target`] (golden-IR tests;
/// no clang/wasm-ld needed). Lets the reactor entry shape be pinned the same
/// way the native entry is.
pub fn compile_to_ir_target(source: &str, target: Target) -> Result<String, String> {
    let mut interner = StringInterner::new();
    let (clauses, directives) = Parser::parse_program_cg(source, &mut interner, 0)
        .map_err(|e| format!("parse error: {e}"))?;
    let cg_sources = vec![CgSource {
        path: "<source>".to_string(),
        text: source.to_string(),
    }];
    codegen::codegen_program(&clauses, &directives, &interner, &cg_sources, target)
}

/// Parse and statically check .pl sources without producing a binary.
///
/// A parse failure is reported as `path:line:col: <message>`; the
/// line/col are extracted from the frontend's error text when present.
/// Returns `Ok(())` only when every file parses cleanly.
pub fn check_files(sources: &[&Path]) -> Result<(), String> {
    parse_sources(sources).map(|_| ())
}

/// Run the undefined-predicate lint over `sources`, returning one rendered
/// message per distinct undefined `(caller, callee)`. Spans the full
/// compilation unit (stdlib included) so stdlib calls aren't flagged.
/// `Err` only on a parse failure. Callers decide warning vs. error.
pub fn undefined_predicate_lints(sources: &[&Path]) -> Result<Vec<String>, String> {
    use plg_frontend::lint;
    let (clauses, directives, interner) = parse_sources(sources)?;
    Ok(lint::undefined_calls(&clauses, &directives, &interner)
        .iter()
        .map(lint::message)
        .collect())
}

/// Render a frontend parse error as `path:line:col: message`, resolving the
/// error's byte span against the source via `SourceMap` so editors and CI
/// can jump to the offending token.
fn format_parse_error(path: &Path, src: &str, err: &ParseError) -> String {
    let (line, col) = SourceMap::new(src).line_col(err.span.lo);
    format!("{}:{}:{}: {}", path.display(), line, col, err.message)
}
