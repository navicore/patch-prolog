//! Source spans — byte ranges into a source file, for diagnostics and
//! (later) runtime error provenance.
//!
//! Value types only. A `Span` is **never** embedded in a `Term` variant, so
//! `Term`'s `PartialEq`/identity is unaffected — spans annotate the AST from
//! the outside (`Spanned<T>`), they don't change what a term *is*.
//!
//! Line/column is deliberately not stored here: it is a presentation concern
//! resolved from the source text at format time (see `plg_frontend`'s
//! `SourceMap`). Byte offsets are the stable representation.

/// Identifies a source file. Single-buffer today (always `0`); multi-file
/// mapping arrives with `plgc build`'s concatenated inputs.
pub type FileId = u32;

/// A byte range `[lo, hi)` into a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(file: FileId, lo: u32, hi: u32) -> Self {
        Self { file, lo, hi }
    }

    /// A zero-width span at `offset` — for point diagnostics like
    /// unexpected-end-of-input where there is no lexeme to underline.
    pub fn point(file: FileId, offset: u32) -> Self {
        Self {
            file,
            lo: offset,
            hi: offset,
        }
    }
}

/// A value annotated with its source span — carries spans alongside the AST
/// without putting one inside `Term` itself.
///
/// No in-tree consumer yet: the undefined-call lint currently uses the
/// parser's flat `CallSite` occurrences instead. `Spanned<T>` is reserved
/// for the Layer 3 runtime-provenance path (see `docs/design/SPANS.md`),
/// where spanned goals feed codegen's `@plg_srcmap` side-table.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}
