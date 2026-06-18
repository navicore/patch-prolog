//! Fact-table compilation (FACT_TABLE.md, Stage A).
//!
//! A predicate whose clauses are all bodyless facts with immediate columns
//! (atom or i61-range integer) compiles to one `.rodata` table of words plus
//! two tiny functions — a generic runtime lookup, instead of one function per
//! clause. Data scales where code doesn't: O(1) IR emission, near-instant
//! `clang` at 100k+ facts. Compound / float / big-int columns are deferred —
//! those predicates fall back to the per-clause path unchanged.
//!
//! Delivery to the continuation is a `musttail` in the generated `deliver:`
//! block (the runtime helper returns first), so recursion through a fact
//! predicate keeps a constant C stack — same discipline as compiled clauses.

use super::CodeGen;
use super::term_emit::{IMM_INT_MAX, IMM_INT_MIN, atom_word, int_word};
use plg_frontend::CgClause;
use plg_shared::{AtomId, Term};
use std::fmt::Write;

/// A Stage-A fact table iff every clause is a bodyless fact whose head args
/// are all immediate (atom or i61-range integer). Empty predicates and any
/// clause with a body or a non-immediate arg disqualify it.
pub fn is_fact_predicate(clauses: &[CgClause]) -> bool {
    // A 0-row table would scan fine, but a predicate with no clauses is
    // already routed elsewhere (dynamic fail-stub / nothing to emit), so we
    // don't duplicate that path here.
    !clauses.is_empty()
        && clauses
            .iter()
            .all(|c| c.body.is_empty() && fact_columns(c).is_some())
}

/// The immediate column words for one fact's head, or `None` if any arg isn't
/// an immediate (→ the predicate falls back to per-clause codegen).
fn fact_columns(clause: &CgClause) -> Option<Vec<u64>> {
    let args: &[Term] = match &clause.head {
        Term::Atom(_) => &[],
        Term::Compound { args, .. } => args,
        _ => return None,
    };
    let mut cols = Vec::with_capacity(args.len());
    for a in args {
        let w = match a {
            Term::Atom(id) => atom_word(*id),
            Term::Integer(n) if (IMM_INT_MIN..=IMM_INT_MAX).contains(n) => int_word(*n).ok()?,
            _ => return None,
        };
        cols.push(w);
    }
    Some(cols)
}

impl CodeGen<'_> {
    /// Emit a fact predicate as a `.rodata` table + generic-lookup entry and
    /// choice-point retry functions. The entry keeps the name `pred_symbol`
    /// expects, so the registry points at it like any other predicate.
    pub fn emit_fact_predicate(
        &mut self,
        functor: AtomId,
        arity: u32,
        clauses: &[CgClause],
    ) -> Result<(), String> {
        let sym = self.pred_symbol(functor, arity);
        let tbl = format!("plg_facts_{functor}_{arity}");
        let nrows = clauses.len();
        let total = nrows * arity as usize;

        // --- Table: rows of immediate words, row-major, in program order.
        let mut words: Vec<u64> = Vec::with_capacity(total);
        for c in clauses {
            // is_fact_predicate guaranteed every clause yields columns.
            words.extend(fact_columns(c).expect("fact predicate columns"));
        }
        writeln!(
            self.out,
            "; {}/{arity} ({nrows} facts \u{2192} table)",
            self.interner.resolve(functor)
        )
        .unwrap();
        if total == 0 {
            // Arity-0 facts (or none): an empty table; nrows drives the count.
            writeln!(
                self.out,
                "@{tbl} = private unnamed_addr constant [0 x i64] zeroinitializer"
            )
            .unwrap();
        } else {
            let body = words
                .iter()
                .map(|w| format!("i64 {w}"))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                self.out,
                "@{tbl} = private unnamed_addr constant [{total} x i64] [{body}]"
            )
            .unwrap();
        }

        // --- Entry: step, then find the first matching row (runtime), then
        //     musttail the continuation.
        self.reset_temps();
        writeln!(self.out, "define i32 @{sym}(ptr %m, i64 %env) {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        let s = self.fresh();
        writeln!(self.out, "  {s} = call i32 @plg_rt_step(ptr %m)").unwrap();
        let c = self.fresh();
        writeln!(self.out, "  {c} = icmp ne i32 {s}, 0").unwrap();
        writeln!(self.out, "  br i1 {c}, label %go, label %fail").unwrap();
        writeln!(self.out, "go:").unwrap();
        let tp = self.fresh();
        writeln!(self.out, "  {tp} = ptrtoint ptr @{tbl} to i64").unwrap();
        let rp = self.fresh();
        writeln!(self.out, "  {rp} = ptrtoint ptr @{sym}_ftr to i64").unwrap();
        let ok = self.fresh();
        writeln!(
            self.out,
            "  {ok} = call i32 @plg_rt_fact_first(ptr %m, i64 {tp}, i64 {nrows}, i64 {arity}, i64 {rp})"
        )
        .unwrap();
        let d = self.fresh();
        writeln!(self.out, "  {d} = icmp ne i32 {ok}, 0").unwrap();
        writeln!(self.out, "  br i1 {d}, label %deliver, label %fail").unwrap();
        writeln!(self.out, "deliver:").unwrap();
        self.emit_fact_deliver();
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();

        // --- Retry: the choice-point continuation — find the next match.
        self.reset_temps();
        writeln!(
            self.out,
            "define internal i32 @{sym}_ftr(ptr %m, i64 %f) {{"
        )
        .unwrap();
        writeln!(self.out, "entry:").unwrap();
        let ok = self.fresh();
        writeln!(
            self.out,
            "  {ok} = call i32 @plg_rt_fact_next(ptr %m, i64 %f)"
        )
        .unwrap();
        let d = self.fresh();
        writeln!(self.out, "  {d} = icmp ne i32 {ok}, 0").unwrap();
        writeln!(self.out, "  br i1 {d}, label %deliver, label %fail").unwrap();
        writeln!(self.out, "deliver:").unwrap();
        self.emit_fact_deliver();
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();

        Ok(())
    }

    /// `deliver:` block — musttail the machine's current continuation. The
    /// runtime helper has already set `m`'s continuation to the caller's `k`.
    fn emit_fact_deliver(&mut self) {
        let kf = self.fresh();
        writeln!(self.out, "  {kf} = call i64 @plg_rt_k_fn(ptr %m)").unwrap();
        let ke = self.fresh();
        writeln!(self.out, "  {ke} = call i64 @plg_rt_k_env(ptr %m)").unwrap();
        let kp = self.fresh();
        writeln!(self.out, "  {kp} = inttoptr i64 {kf} to ptr").unwrap();
        let r = self.fresh();
        writeln!(self.out, "  {r} = musttail call i32 {kp}(ptr %m, i64 {ke})").unwrap();
        writeln!(self.out, "  ret i32 {r}").unwrap();
    }
}
