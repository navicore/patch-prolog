//! LLVM IR (text) generation.
//!
//! Each predicate compiles to native functions in continuation-passing
//! style over a uniform `i32 (ptr, i64)` C-ABI signature; all control
//! transfers are `musttail` so Prolog recursion never grows the C
//! stack. See docs/design/COMPILATION_MODEL.md.

mod atoms;
mod body;
mod clause;
mod lower;
mod predicate;
mod program;
mod term_emit;

pub use program::codegen_program;

use plg_frontend::{CgClause, SourceMap};
use plg_shared::{AtomId, Span, StringInterner};
use std::collections::BTreeMap;

/// `site_id` sentinel for a call with no resolvable source location (mirrors
/// `plg_runtime::machine::NO_SITE`); emitted for stdlib/synthetic sources.
pub const NO_SITE: u32 = u32::MAX;

/// A source file available to codegen for resolving spans to `file:line:col`.
/// `FileId` indexes a slice of these.
pub struct CgSource {
    pub path: String,
    pub text: String,
}

/// Where a body goal dispatches to at compile time.
#[derive(Clone, Copy, PartialEq)]
pub enum GoalTarget {
    /// Defined in this program: direct musttail to its entry function.
    Defined,
    /// Declared `:- dynamic` with no clauses: silent fail.
    DynamicFail,
    /// Not defined anywhere: existence_error at call time (v1 contract).
    Undefined,
}

pub struct CodeGen<'a> {
    pub interner: &'a StringInterner,
    /// (functor, arity) -> clauses, in program order.
    pub predicates: BTreeMap<(AtomId, u32), Vec<CgClause>>,
    /// `:- dynamic` declarations with no clauses.
    pub dynamic_only: Vec<(AtomId, u32)>,
    pub out: String,
    tmp: u32,
    label: u32,
    /// Source files, indexed by `FileId`, for resolving call-site spans to
    /// `file:line:col` (SPANS.md Layer 3).
    sources: &'a [CgSource],
    /// Emitted `@plg_srcmap` rows: `(file_idx, line, col)`, indexed by
    /// `site_id`.
    srcmap: Vec<(u32, u32, u32)>,
    /// Emitted `@plg_files` entries (deduplicated filenames), indexed by the
    /// `file_idx` stored in `srcmap` rows.
    files: Vec<String>,
}

impl<'a> CodeGen<'a> {
    pub fn new(interner: &'a StringInterner, sources: &'a [CgSource]) -> Self {
        CodeGen {
            interner,
            predicates: BTreeMap::new(),
            dynamic_only: Vec::new(),
            out: String::new(),
            tmp: 0,
            label: 0,
            sources,
            srcmap: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Assign a `site_id` for a call-site span: resolve it to `file:line:col`
    /// against `sources`, append a `@plg_srcmap` row, and return its index.
    /// Returns `NO_SITE` when the span's file isn't a real source (stdlib or
    /// synthetic), so no provenance suffix is emitted for it.
    pub fn site_id(&mut self, span: Span) -> u32 {
        let Some(src) = self.sources.get(span.file as usize) else {
            return NO_SITE;
        };
        let (line, col) = SourceMap::new(&src.text).line_col(span.lo);
        let file_idx = match self.files.iter().position(|p| p == &src.path) {
            Some(i) => i as u32,
            None => {
                self.files.push(src.path.clone());
                (self.files.len() - 1) as u32
            }
        };
        let id = self.srcmap.len() as u32;
        self.srcmap.push((file_idx, line as u32, col as u32));
        id
    }

    /// Fresh SSA temporary name.
    pub fn fresh(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }

    /// Fresh basic-block label.
    pub fn fresh_label(&mut self) -> String {
        self.label += 1;
        format!("L{}", self.label)
    }

    /// Reset the SSA/label counters (names are function-local).
    pub fn reset_temps(&mut self) {
        self.tmp = 0;
        self.label = 0;
    }

    /// Symbol-safe predicate entry name: `plg_pred_<id>_<arity>__<sane>`.
    /// The atom id disambiguates; the sanitized name keeps IR readable.
    pub fn pred_symbol(&self, functor: AtomId, arity: u32) -> String {
        format!(
            "plg_pred_{functor}_{arity}__{}",
            sanitize(self.interner.resolve(functor))
        )
    }

    pub fn how_to_call(&self, functor: AtomId, arity: u32) -> GoalTarget {
        if self.predicates.contains_key(&(functor, arity)) {
            GoalTarget::Defined
        } else if self.dynamic_only.contains(&(functor, arity)) {
            GoalTarget::DynamicFail
        } else {
            GoalTarget::Undefined
        }
    }
}

/// Keep `[A-Za-z0-9_]`, hex-escape the rest, cap the length.
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().take(24) {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("x{:02x}", c as u32 & 0xff));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_escapes_symbols() {
        assert_eq!(sanitize("foo_bar9"), "foo_bar9");
        assert_eq!(sanitize("=.."), "x3dx2ex2e");
    }
}
