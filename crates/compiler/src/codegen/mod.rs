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

use plg_shared::{AtomId, Clause, StringInterner};
use std::collections::BTreeMap;

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
    pub predicates: BTreeMap<(AtomId, u32), Vec<Clause>>,
    /// `:- dynamic` declarations with no clauses.
    pub dynamic_only: Vec<(AtomId, u32)>,
    pub out: String,
    tmp: u32,
    label: u32,
}

impl<'a> CodeGen<'a> {
    pub fn new(interner: &'a StringInterner) -> Self {
        CodeGen {
            interner,
            predicates: BTreeMap::new(),
            dynamic_only: Vec::new(),
            out: String::new(),
            tmp: 0,
            label: 0,
        }
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
