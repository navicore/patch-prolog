//! Emit the per-predicate entry function and clause-chaining ("try next
//! clause") functions.
//!
//! Entry: bump the step counter, snapshot argument registers + the
//! caller's continuation into a predicate frame, push a choice point
//! for the remaining clauses, and `musttail` into clause 0. Each chain
//! function is the retry target stored in the choice point: it pushes
//! the next chain link (if any) and tries its clause.

use super::CodeGen;
use plg_shared::{AtomId, Clause};
use std::fmt::Write;

impl CodeGen<'_> {
    pub fn emit_predicate(
        &mut self,
        functor: AtomId,
        arity: u32,
        clauses: &[Clause],
    ) -> Result<(), String> {
        let name = self.pred_symbol(functor, arity);
        let base = format!("plg_p{functor}_{arity}");
        let n = clauses.len();

        // --- Entry function (registry target; callable by name).
        self.reset_temps();
        writeln!(
            self.out,
            "; {}/{arity} ({n} clauses)",
            self.interner.resolve(functor)
        )
        .unwrap();
        writeln!(self.out, "define i32 @{name}(ptr %m, i64 %env) {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        let s = self.fresh();
        writeln!(self.out, "  {s} = call i32 @plg_rt_step(ptr %m)").unwrap();
        let c = self.fresh();
        writeln!(self.out, "  {c} = icmp ne i32 {s}, 0").unwrap();
        writeln!(self.out, "  br i1 {c}, label %go, label %fail").unwrap();
        writeln!(self.out, "go:").unwrap();
        // Predicate frame: [args..., k_fn, k_env]
        let f = self.fresh();
        writeln!(
            self.out,
            "  {f} = call i64 @plg_rt_frame_alloc(ptr %m, i32 {})",
            arity + 2
        )
        .unwrap();
        for i in 0..arity {
            let a = self.fresh();
            writeln!(
                self.out,
                "  {a} = call i64 @plg_rt_areg_get(ptr %m, i32 {i})"
            )
            .unwrap();
            writeln!(
                self.out,
                "  call void @plg_rt_frame_set(ptr %m, i64 {f}, i32 {i}, i64 {a})"
            )
            .unwrap();
        }
        let kf = self.fresh();
        writeln!(self.out, "  {kf} = call i64 @plg_rt_k_fn(ptr %m)").unwrap();
        writeln!(
            self.out,
            "  call void @plg_rt_frame_set(ptr %m, i64 {f}, i32 {arity}, i64 {kf})"
        )
        .unwrap();
        let ke = self.fresh();
        writeln!(self.out, "  {ke} = call i64 @plg_rt_k_env(ptr %m)").unwrap();
        writeln!(
            self.out,
            "  call void @plg_rt_frame_set(ptr %m, i64 {f}, i32 {}, i64 {ke})",
            arity + 1
        )
        .unwrap();
        if n > 1 {
            let t = self.fresh();
            writeln!(self.out, "  {t} = ptrtoint ptr @{base}_t1 to i64").unwrap();
            writeln!(
                self.out,
                "  call void @plg_rt_push_cp(ptr %m, i64 {t}, i64 {f})"
            )
            .unwrap();
        }
        let r = self.fresh();
        writeln!(
            self.out,
            "  {r} = musttail call i32 @{base}_c0(ptr %m, i64 {f})"
        )
        .unwrap();
        writeln!(self.out, "  ret i32 {r}").unwrap();
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();

        // --- Chain functions: retry targets for clauses 1..n-1.
        for j in 1..n {
            self.reset_temps();
            writeln!(
                self.out,
                "define internal i32 @{base}_t{j}(ptr %m, i64 %f) {{"
            )
            .unwrap();
            writeln!(self.out, "entry:").unwrap();
            if j + 1 < n {
                let t = self.fresh();
                writeln!(self.out, "  {t} = ptrtoint ptr @{base}_t{} to i64", j + 1).unwrap();
                writeln!(
                    self.out,
                    "  call void @plg_rt_push_cp(ptr %m, i64 {t}, i64 %f)"
                )
                .unwrap();
            }
            let r = self.fresh();
            writeln!(
                self.out,
                "  {r} = musttail call i32 @{base}_c{j}(ptr %m, i64 %f)"
            )
            .unwrap();
            writeln!(self.out, "  ret i32 {r}").unwrap();
            writeln!(self.out, "}}").unwrap();
        }

        // --- Clause functions.
        for (j, clause) in clauses.iter().enumerate() {
            self.emit_clause(functor, arity, j, clause)?;
        }
        Ok(())
    }
}
