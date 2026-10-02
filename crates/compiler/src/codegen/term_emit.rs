//! Emit IR that materializes a Term as a tagged 64-bit word.
//!
//! Atoms and small integers are compile-time constant words; compounds,
//! lists, and floats allocate on the machine heap via `plg_rt_put_*`.
//! Variables resolve through the clause's var map (every clause variable
//! is either aliased to an incoming argument or freshly allocated at
//! clause start, so lookup never misses).

use super::CodeGen;
use plg_shared::Term;
use plg_shared::term::VarId;
use std::collections::HashMap;
use std::fmt::Write;

// The word/cell encoding is the ABI shared with the runtime; use the one
// source of truth in `plg-shared` rather than re-deriving the layout here.
pub use plg_shared::cell::{INT_MAX as IMM_INT_MAX, INT_MIN as IMM_INT_MIN};

pub fn atom_word(id: u32) -> u64 {
    plg_shared::cell::make_atom(id)
}

pub fn int_word(n: i64) -> Result<u64, String> {
    if !(IMM_INT_MIN..=IMM_INT_MAX).contains(&n) {
        return Err(format!(
            "integer literal {n} is outside the immediate range (boxed at runtime)"
        ));
    }
    Ok(plg_shared::cell::make_int(n))
}

impl CodeGen<'_> {
    /// Emit code (into `body`) producing `term` as an i64 word; returns
    /// the SSA name or literal constant.
    pub fn emit_term(
        &mut self,
        body: &mut String,
        term: &Term,
        vars: &HashMap<VarId, String>,
    ) -> Result<String, String> {
        match term {
            Term::Atom(id) => Ok(atom_word(*id).to_string()),
            Term::Integer(n) if (IMM_INT_MIN..=IMM_INT_MAX).contains(n) => {
                Ok(int_word(*n)?.to_string())
            }
            Term::Integer(n) => {
                // Beyond the i61 immediate: box at runtime (BIG cell).
                let t = self.fresh();
                writeln!(body, "  {t} = call i64 @plg_rt_put_big(ptr %m, i64 {n})").unwrap();
                Ok(t)
            }
            Term::Float(f) => {
                let t = self.fresh();
                writeln!(
                    body,
                    "  {t} = call i64 @plg_rt_put_float(ptr %m, i64 {})",
                    f.to_bits()
                )
                .unwrap();
                Ok(t)
            }
            Term::Var(v) => vars
                .get(v)
                .cloned()
                .ok_or_else(|| format!("internal: unmapped variable _{v}")),
            Term::Compound { functor, args } => {
                // Children first (they may use breg themselves), then
                // load this level's breg slots and build.
                let mut words = Vec::with_capacity(args.len());
                for a in args {
                    words.push(self.emit_term(body, a, vars)?);
                }
                for (i, w) in words.iter().enumerate() {
                    writeln!(
                        body,
                        "  call void @plg_rt_breg_set(ptr %m, i32 {i}, i64 {w})"
                    )
                    .unwrap();
                }
                let t = self.fresh();
                writeln!(
                    body,
                    "  {t} = call i64 @plg_rt_put_struct(ptr %m, i32 {functor}, i32 {})",
                    args.len()
                )
                .unwrap();
                Ok(t)
            }
            Term::List { head, tail } => {
                let h = self.emit_term(body, head, vars)?;
                let tl = self.emit_term(body, tail, vars)?;
                let t = self.fresh();
                writeln!(
                    body,
                    "  {t} = call i64 @plg_rt_put_list(ptr %m, i64 {h}, i64 {tl})"
                )
                .unwrap();
                Ok(t)
            }
        }
    }
}

/// Collect every variable id in a term, in first-appearance order.
pub fn collect_vars(term: &Term, out: &mut Vec<VarId>) {
    match term {
        Term::Var(v) if !out.contains(v) => out.push(*v),
        Term::Var(_) => {}
        Term::Compound { args, .. } => {
            for a in args {
                collect_vars(a, out);
            }
        }
        Term::List { head, tail } => {
            collect_vars(head, out);
            collect_vars(tail, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_words_match_runtime_tags() {
        // Mirrors runtime cell.rs: ATOM=1, INT=2, 3-bit shift.
        assert_eq!(atom_word(0), 1);
        assert_eq!(atom_word(7), (7 << 3) | 1);
        assert_eq!(int_word(5).unwrap(), (5 << 3) | 2);
        // Negative payloads survive the shift round-trip.
        assert_eq!((int_word(-1).unwrap() as i64) >> 3, -1);
        assert!(int_word(i64::MAX).is_err(), "immediates only; big ints box");
    }

    #[test]
    fn collect_vars_first_appearance_order() {
        let t = Term::Compound {
            functor: 0,
            args: vec![Term::Var(3), Term::Var(1), Term::Var(3)],
        };
        let mut vars = Vec::new();
        collect_vars(&t, &mut vars);
        assert_eq!(vars, vec![3, 1]);
    }

    use crate::codegen::CodeGen;
    use plg_shared::StringInterner;
    use std::collections::HashMap;

    /// Run `emit_term` on a term with a fresh CodeGen; returns (result,
    /// emitted IR). `emit_term` never touches the interner, so an empty one
    /// is fine.
    fn emit_ir(term: &Term, vars: &HashMap<VarId, String>) -> (Result<String, String>, String) {
        let interner = StringInterner::new();
        let mut cg = CodeGen::new(&interner, &[]);
        let mut body = String::new();
        let w = cg.emit_term(&mut body, term, vars);
        (w, body)
    }

    #[test]
    fn emit_term_immediates_are_constant_words() {
        let (w, body) = emit_ir(&Term::Atom(7), &HashMap::new());
        assert_eq!(w.unwrap(), atom_word(7).to_string());
        assert!(body.is_empty(), "immediates emit no code");

        let (w, body) = emit_ir(&Term::Integer(5), &HashMap::new());
        assert_eq!(w.unwrap(), int_word(5).unwrap().to_string());
        assert!(body.is_empty());
    }

    #[test]
    fn emit_term_boxes_big_ints_and_floats() {
        let (w, body) = emit_ir(&Term::Integer(i64::MAX), &HashMap::new());
        assert_eq!(w.unwrap(), "%t1");
        assert_eq!(
            body,
            "  %t1 = call i64 @plg_rt_put_big(ptr %m, i64 9223372036854775807)\n"
        );

        let (w, body) = emit_ir(&Term::Float(1.5), &HashMap::new());
        assert_eq!(w.unwrap(), "%t1");
        assert_eq!(
            body,
            format!(
                "  %t1 = call i64 @plg_rt_put_float(ptr %m, i64 {})\n",
                1.5f64.to_bits()
            )
        );
    }

    #[test]
    fn emit_term_compound_loads_bregs_then_put_struct() {
        let t = Term::Compound {
            functor: 3,
            args: vec![Term::Atom(1), Term::Var(0)],
        };
        let mut vars = HashMap::new();
        vars.insert(0, "%t9".to_string());
        let (w, body) = emit_ir(&t, &vars);
        // Children emit nothing (constant + mapped var); the only fresh
        // temporary is the put_struct result.
        assert_eq!(w.unwrap(), "%t1");
        assert_eq!(
            body,
            format!(
                "  call void @plg_rt_breg_set(ptr %m, i32 0, i64 {})\n  \
                 call void @plg_rt_breg_set(ptr %m, i32 1, i64 %t9)\n  \
                 %t1 = call i64 @plg_rt_put_struct(ptr %m, i32 3, i32 2)\n",
                atom_word(1)
            )
        );
    }

    #[test]
    fn emit_term_lists_and_unmapped_vars() {
        let t = Term::List {
            head: Box::new(Term::Atom(1)),
            tail: Box::new(Term::Atom(2)),
        };
        let (w, body) = emit_ir(&t, &HashMap::new());
        assert_eq!(w.unwrap(), "%t1");
        assert_eq!(
            body,
            format!(
                "  %t1 = call i64 @plg_rt_put_list(ptr %m, i64 {}, i64 {})\n",
                atom_word(1),
                atom_word(2)
            )
        );

        // A variable missing from the map is an internal error, not a panic.
        let (w, _) = emit_ir(&Term::Var(4), &HashMap::new());
        assert!(w.unwrap_err().contains("unmapped variable _4"));
    }
}
