//! Compile one clause: head unification, then the lowered body via the
//! sequence compiler in body.rs.
//!
//! Predicate frame (built by the entry function, predicate.rs):
//!   [arg0 .. arg(A-1), k_fn, k_env, cut_barrier]
//! Body frame: see body.rs.

use super::body::{After, ClauseCtx};
use super::lower::{self, LGoal, LGoalKind};
use super::term_emit::collect_vars;
use super::{CodeGen, GoalTarget};
use plg_frontend::CgClause;
use plg_shared::term::VarId;
use plg_shared::{AtomId, Span, Term};
use std::collections::HashMap;
use std::fmt::Write;

impl CodeGen<'_> {
    /// Emit the clause function `@plg_p<F>_<A>_c<j>` plus all auxiliary
    /// continuation/branch functions its body needs.
    pub fn emit_clause(
        &mut self,
        functor: AtomId,
        arity: u32,
        j: usize,
        clause: &CgClause,
    ) -> Result<(), String> {
        let base = format!("plg_p{functor}_{arity}_c{j}");
        let goals = lower::lower_body(&clause.body, self.interner)?;

        // Clause variables in deterministic order (head first).
        let mut var_list: Vec<VarId> = Vec::new();
        collect_vars(&clause.head, &mut var_list);
        for g in &goals {
            lower::collect_goal_vars(g, &mut var_list);
        }
        let scratch = lower::count_scratch(&goals);

        self.reset_temps();
        let mut b = String::new();
        let mut vars: HashMap<VarId, String> = HashMap::new();

        // --- Head: load incoming args; alias first-occurrence var
        // patterns, queue everything else for unification.
        let head_args: &[Term] = match &clause.head {
            Term::Compound { args, .. } => args,
            _ => &[], // arity-0 predicate
        };
        let mut to_unify: Vec<(String, &Term)> = Vec::new();
        for (i, pat) in head_args.iter().enumerate() {
            let arg = self.fresh();
            writeln!(
                b,
                "  {arg} = call i64 @plg_rt_frame_get(ptr %m, i64 %f, i32 {i})"
            )
            .unwrap();
            match pat {
                Term::Var(v) if !vars.contains_key(v) => {
                    vars.insert(*v, arg);
                }
                _ => to_unify.push((arg, pat)),
            }
        }
        // Remaining clause variables get fresh cells.
        for v in &var_list {
            if !vars.contains_key(v) {
                let t = self.fresh();
                writeln!(b, "  {t} = call i64 @plg_rt_new_var(ptr %m)").unwrap();
                vars.insert(*v, t);
            }
        }
        // Emit queued head unifications (after all vars exist).
        for (arg, pat) in to_unify {
            let w = self.emit_term(&mut b, pat, &vars)?;
            let u = self.fresh();
            writeln!(
                b,
                "  {u} = call i32 @plg_rt_unify(ptr %m, i64 {arg}, i64 {w})"
            )
            .unwrap();
            self.emit_branch_on(&mut b, &u);
        }

        // --- Body.
        if goals.is_empty() {
            // Fact: jump straight to the caller's continuation.
            let kf = self.fresh();
            writeln!(
                b,
                "  {kf} = call i64 @plg_rt_frame_get(ptr %m, i64 %f, i32 {arity})"
            )
            .unwrap();
            let ke = self.fresh();
            writeln!(
                b,
                "  {ke} = call i64 @plg_rt_frame_get(ptr %m, i64 %f, i32 {})",
                arity + 1
            )
            .unwrap();
            let kp = self.fresh();
            writeln!(b, "  {kp} = inttoptr i64 {kf} to ptr").unwrap();
            let r = self.fresh();
            writeln!(b, "  {r} = musttail call i32 {kp}(ptr %m, i64 {ke})").unwrap();
            writeln!(b, "  ret i32 {r}").unwrap();
            self.write_fn(&base, "%f", &b);
            return Ok(());
        }

        // Build the body frame: [k_fn, k_env, barrier, vars..., scratch...].
        let mut ctx = ClauseCtx::new(base.clone(), var_list.clone(), String::new());
        let bf = self.fresh();
        writeln!(
            b,
            "  {bf} = call i64 @plg_rt_frame_alloc(ptr %m, i32 {})",
            ctx.frame_size(scratch)
        )
        .unwrap();
        for (slot, src) in [(0u32, arity), (1, arity + 1), (2, arity + 2)] {
            let t = self.fresh();
            writeln!(
                b,
                "  {t} = call i64 @plg_rt_frame_get(ptr %m, i64 %f, i32 {src})"
            )
            .unwrap();
            writeln!(
                b,
                "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 {slot}, i64 {t})"
            )
            .unwrap();
        }
        for (i, v) in var_list.iter().enumerate() {
            let w = &vars[v];
            writeln!(
                b,
                "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 {}, i64 {w})",
                3 + i
            )
            .unwrap();
        }
        ctx.bf = bf;
        // Top-level body: `!` targets the predicate barrier (slot 2).
        self.compile_seq(&mut b, &goals, &After::CallerK, &mut ctx, &vars, 2)?;
        writeln!(
            self.out,
            "; clause {j} of {}/{arity}",
            self.interner.resolve(functor)
        )
        .unwrap();
        self.write_fn(&base, "%f", &b);
        self.emit_aux_fns(&mut ctx)?;
        Ok(())
    }

    fn write_fn(&mut self, sym: &str, env_name: &str, body: &str) {
        writeln!(
            self.out,
            "define internal i32 @{sym}(ptr %m, i64 {env_name}) {{"
        )
        .unwrap();
        writeln!(self.out, "entry:").unwrap();
        self.out.push_str(body);
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();
    }

    /// `br` to a fresh continue-label or %fail on an i32 result.
    pub fn emit_branch_on(&mut self, b: &mut String, result: &str) {
        let l = self.fresh_label();
        let c = self.fresh();
        writeln!(b, "  {c} = icmp ne i32 {result}, 0").unwrap();
        writeln!(b, "  br i1 {c}, label %{l}, label %fail").unwrap();
        writeln!(b, "{l}:").unwrap();
    }

    /// Deterministic builtins executed inline within a sequence.
    pub fn emit_inline_builtin(
        &mut self,
        b: &mut String,
        g: &LGoal,
        vars: &HashMap<VarId, String>,
    ) -> Result<(), String> {
        // Call-site span for raising builtins (SPANS.md Layer 3).
        let span = g.span;
        let r = match &g.node {
            LGoalKind::Unify(x, y) => {
                let (wx, wy) = (self.emit_term(b, x, vars)?, self.emit_term(b, y, vars)?);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_unify(ptr %m, i64 {wx}, i64 {wy})"
                )
                .unwrap();
                r
            }
            LGoalKind::NotUnify(x, y) => {
                let (wx, wy) = (self.emit_term(b, x, vars)?, self.emit_term(b, y, vars)?);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_neq(ptr %m, i64 {wx}, i64 {wy})"
                )
                .unwrap();
                r
            }
            LGoalKind::TermCmp(op, x, y) => {
                let (wx, wy) = (self.emit_term(b, x, vars)?, self.emit_term(b, y, vars)?);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_term_cmp(ptr %m, i32 {op}, i64 {wx}, i64 {wy})"
                )
                .unwrap();
                r
            }
            LGoalKind::Compare(o, x, y) => {
                let wo = self.emit_term(b, o, vars)?;
                let (wx, wy) = (self.emit_term(b, x, vars)?, self.emit_term(b, y, vars)?);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_compare(ptr %m, i64 {wo}, i64 {wx}, i64 {wy})"
                )
                .unwrap();
                r
            }
            LGoalKind::Is(x, e) => {
                let wx = self.emit_term(b, x, vars)?;
                let we = self.emit_term(b, e, vars)?;
                let site = self.site_id(span);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_is(ptr %m, i64 {wx}, i64 {we}, i32 {site})"
                )
                .unwrap();
                r
            }
            LGoalKind::ArithCmp(op, x, y) => {
                let (wx, wy) = (self.emit_term(b, x, vars)?, self.emit_term(b, y, vars)?);
                let site = self.site_id(span);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_arith_cmp(ptr %m, i32 {op}, i64 {wx}, i64 {wy}, i32 {site})"
                )
                .unwrap();
                r
            }
            LGoalKind::RtDet { sym, args, raises } => {
                let mut words = Vec::with_capacity(args.len());
                for a in args {
                    words.push(self.emit_term(b, a, vars)?);
                }
                let mut arglist: Vec<String> = words.iter().map(|w| format!(", i64 {w}")).collect();
                // Raising det builtins take a trailing site_id (SPANS Layer 3).
                if *raises {
                    let site = self.site_id(span);
                    arglist.push(format!(", i32 {site}"));
                }
                let r = self.fresh();
                writeln!(b, "  {r} = call i32 @{sym}(ptr %m{})", arglist.join("")).unwrap();
                r
            }
            _ => unreachable!("not an inline builtin"),
        };
        self.emit_branch_on(b, &r);
        Ok(())
    }

    /// Emit a runtime metacall in tail position (`call/N`, or a variable
    /// goal). Fast path: resolve the goal to a compiled predicate entry and
    /// `musttail` into it, so `call(pred(...))` tail recursion keeps constant
    /// C stack (#23) — exactly like a direct call. Slow path (builtins,
    /// control constructs, `call/N` with extras, errors): fall back to the
    /// full Rust walker via `plg_rt_metacall`, which the runtime depth guard
    /// bounds. Both branches `ret`; emit only in tail position. `g` is the
    /// SSA value already holding the goal term.
    pub(crate) fn emit_metacall(&mut self, b: &mut String, g: &str) {
        let fp = self.fresh();
        let z = self.fresh();
        let jump = self.fresh_label();
        let complex = self.fresh_label();
        let fpp = self.fresh();
        let r = self.fresh();
        let r2 = self.fresh();
        writeln!(
            b,
            "  {fp} = call i64 @plg_rt_metacall_resolve(ptr %m, i64 {g})"
        )
        .unwrap();
        writeln!(b, "  {z} = icmp eq i64 {fp}, 0").unwrap();
        writeln!(b, "  br i1 {z}, label %{complex}, label %{jump}").unwrap();
        writeln!(b, "{jump}:").unwrap();
        writeln!(b, "  {fpp} = inttoptr i64 {fp} to ptr").unwrap();
        writeln!(b, "  {r} = musttail call i32 {fpp}(ptr %m, i64 0)").unwrap();
        writeln!(b, "  ret i32 {r}").unwrap();
        writeln!(b, "{complex}:").unwrap();
        writeln!(b, "  {r2} = call i32 @plg_rt_metacall(ptr %m, i64 {g})").unwrap();
        writeln!(b, "  ret i32 {r2}").unwrap();
    }

    /// Emit a predicate call in tail position: load argument registers
    /// and `musttail` into the callee (the installed k continues).
    pub fn emit_call_tail(
        &mut self,
        b: &mut String,
        functor: AtomId,
        args: &[Term],
        vars: &HashMap<VarId, String>,
        span: Span,
    ) -> Result<(), String> {
        let arity = args.len() as u32;
        if arity as usize > crate::MAX_GOAL_ARITY {
            return Err(format!(
                "goal arity {arity} exceeds the supported maximum of {}",
                crate::MAX_GOAL_ARITY
            ));
        }
        // Control builtins taking goal/term arguments: the installed k
        // is the continuation; the runtime walks the goal terms.
        let name = self.interner.resolve(functor).to_string();
        match (name.as_str(), arity) {
            ("throw", 1) => {
                let w = self.emit_term(b, &args[0], vars)?;
                let r = self.fresh();
                writeln!(b, "  {r} = call i32 @plg_rt_b_throw_1(ptr %m, i64 {w})").unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
                return Ok(());
            }
            ("catch", 3) => {
                let g = self.emit_term(b, &args[0], vars)?;
                let c = self.emit_term(b, &args[1], vars)?;
                let rec = self.emit_term(b, &args[2], vars)?;
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_catch_3(ptr %m, i64 {g}, i64 {c}, i64 {rec})"
                )
                .unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
                return Ok(());
            }
            ("findall", 3) => {
                let t = self.emit_term(b, &args[0], vars)?;
                let g = self.emit_term(b, &args[1], vars)?;
                let bag = self.emit_term(b, &args[2], vars)?;
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_b_findall_3(ptr %m, i64 {t}, i64 {g}, i64 {bag})"
                )
                .unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
                return Ok(());
            }
            ("call", n) if n >= 1 => {
                // Build the call/N structure; the runtime extends and
                // dispatches it (try_builtin "call").
                let goal = Term::Compound {
                    functor,
                    args: args.to_vec(),
                };
                let g = self.emit_term(b, &goal, vars)?;
                self.emit_metacall(b, &g);
                return Ok(());
            }
            ("between", 3) => {
                // Nondeterministic builtin with a uniform predicate
                // signature: dispatched exactly like a user predicate.
                let mut words = Vec::with_capacity(args.len());
                for a in args {
                    words.push(self.emit_term(b, a, vars)?);
                }
                for (i, w) in words.iter().enumerate() {
                    writeln!(b, "  call void @plg_rt_areg_set(ptr %m, i32 {i}, i64 {w})").unwrap();
                }
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = musttail call i32 @plg_rt_pred_between_3(ptr %m, i64 0)"
                )
                .unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
                return Ok(());
            }
            _ => {}
        }
        match self.how_to_call(functor, arity) {
            GoalTarget::Undefined => {
                // v1 contract: existence_error raised when the goal runs.
                // The site_id carries source provenance (SPANS.md Layer 3).
                // It is emitted as an `i32` (the runtime ABI is `u32`); the
                // two's-complement bit pattern matches, so `NO_SITE`
                // (`u32::MAX`) reads as `i32 -1` here and back to `u32::MAX`
                // in the runtime.
                let site = self.site_id(span);
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_existence_error(ptr %m, i32 {functor}, i32 {arity}, i32 {site})"
                )
                .unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
            }
            target => {
                let mut words = Vec::with_capacity(args.len());
                for a in args {
                    words.push(self.emit_term(b, a, vars)?);
                }
                for (i, w) in words.iter().enumerate() {
                    writeln!(b, "  call void @plg_rt_areg_set(ptr %m, i32 {i}, i64 {w})").unwrap();
                }
                let callee = if target == GoalTarget::Defined {
                    self.pred_symbol(functor, arity)
                } else {
                    "plg_rt_pred_fail".to_string()
                };
                let r = self.fresh();
                writeln!(b, "  {r} = musttail call i32 @{callee}(ptr %m, i64 0)").unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
            }
        }
        Ok(())
    }
}
