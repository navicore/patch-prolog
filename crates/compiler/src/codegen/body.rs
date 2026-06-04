//! Clause-body compilation: a goal sequence becomes straight-line IR
//! for deterministic builtins, `musttail` dispatch for predicate calls,
//! and choice-point machinery for control constructs.
//!
//! Body frame layout (heap cells, per clause activation):
//!   [0] k_fn   [1] k_env   [2] cut barrier
//!   [3 .. 3+V)        clause variables
//!   [3+V .. 3+V+S)    scratch slots (one per ITE/once/NAF site,
//!                     holding the choice-point height to commit to)
//!
//! Control lowering (all heights captured BEFORE the related push_cp):
//! - `(A ; B)` — push CP retrying B, fall into A.
//! - `(C->T ; E)` — capture h, push CP retrying E, run C with a
//!   continuation that cuts to h (killing E and C's alternatives) and
//!   runs T.
//! - `(C -> T)` — same minus the E choice point.
//! - `once(G)` — `(G -> rest)`: cut to h, continue.
//! - `\+ G` — capture h, push CP that CONTINUES the body (G's bindings
//!   undone by the driver's rewind), run G with a continuation that
//!   cuts to h and returns 0.

use super::CodeGen;
use super::lower::LGoal;
use plg_shared::term::VarId;
use std::collections::HashMap;
use std::fmt::Write;

/// What runs after the current goal sequence succeeds.
#[derive(Clone)]
pub enum After {
    /// The caller's continuation, stored in bf slots 0/1.
    CallerK,
    /// A generated function `@sym(ptr %m, i64 %bf)`.
    Fn(String),
}

/// Work item for a function generated during body compilation.
enum AuxKind {
    /// Reload vars, run a goal sequence.
    Seq {
        goals: Vec<LGoal>,
        after: After,
        cut_slot: usize,
    },
    /// Cut to the height in `slot`, then run a sequence (ITE-then,
    /// if-then commit, once commit).
    CutThenSeq {
        slot: usize,
        goals: Vec<LGoal>,
        after: After,
        cut_slot: usize,
    },
    /// NAF inner goal succeeded: cut to `slot`'s height, fail the NAF.
    NafFound { slot: usize },
    /// Trampoline: jump to the caller's continuation (used when a CP
    /// retry needs a function but the rest-continuation is CallerK).
    CallerKJump,
}

pub struct ClauseCtx {
    /// Function-name prefix for this clause (`plg_p<F>_<A>_c<j>`).
    pub base: String,
    /// Clause variables in frame order.
    pub var_list: Vec<VarId>,
    /// Frame index of the next free scratch slot.
    next_scratch: usize,
    aux_counter: u32,
    /// SSA name (or `%bf`) holding the body-frame index in the
    /// function currently being emitted.
    pub bf: String,
    work: Vec<(String, AuxKind)>,
    callerk_jump: Option<String>,
}

impl ClauseCtx {
    pub fn new(base: String, var_list: Vec<VarId>, bf: String) -> Self {
        let n_vars = var_list.len();
        ClauseCtx {
            base,
            var_list,
            next_scratch: 3 + n_vars,
            aux_counter: 0,
            bf,
            work: Vec::new(),
            callerk_jump: None,
        }
    }

    pub fn frame_size(&self, scratch: usize) -> usize {
        3 + self.var_list.len() + scratch
    }

    fn alloc_scratch(&mut self) -> usize {
        let s = self.next_scratch;
        self.next_scratch += 1;
        s
    }

    fn queue(&mut self, kind: AuxKind) -> String {
        self.aux_counter += 1;
        let sym = format!("{}_a{}", self.base, self.aux_counter);
        self.work.push((sym.clone(), kind));
        sym
    }

    fn callerk_jump(&mut self) -> String {
        if let Some(s) = &self.callerk_jump {
            return s.clone();
        }
        let sym = self.queue(AuxKind::CallerKJump);
        self.callerk_jump = Some(sym.clone());
        sym
    }
}

impl CodeGen<'_> {
    /// Compile a goal sequence into `b`. Emits `ret` on every exit path.
    pub fn compile_seq(
        &mut self,
        b: &mut String,
        goals: &[LGoal],
        after: &After,
        ctx: &mut ClauseCtx,
        vars: &HashMap<VarId, String>,
        cut_slot: usize,
    ) -> Result<(), String> {
        let bf = ctx.bf.clone();
        let mut i = 0;
        while i < goals.len() {
            let rest = &goals[i + 1..];
            match &goals[i] {
                LGoal::True => {}
                LGoal::Fail => {
                    writeln!(b, "  ret i32 0").unwrap();
                    return Ok(()); // rest unreachable
                }
                LGoal::Cut => {
                    // The barrier slot depends on context: slot 2 is the
                    // predicate barrier; call-like constructs (`->`
                    // conditions, `\+`, `once`) pass a local slot, making
                    // the cut opaque there per ISO.
                    let h = self.fresh();
                    writeln!(
                        b,
                        "  {h} = call i64 @plg_rt_frame_get(ptr %m, i64 {bf}, i32 {cut_slot})"
                    )
                    .unwrap();
                    writeln!(b, "  call void @plg_rt_cut(ptr %m, i64 {h})").unwrap();
                }
                g @ (LGoal::Unify(..)
                | LGoal::NotUnify(..)
                | LGoal::TermCmp(..)
                | LGoal::Compare(..)
                | LGoal::Is(..)
                | LGoal::ArithCmp(..)
                | LGoal::RtDet { .. }) => self.emit_inline_builtin(b, g, vars)?,
                LGoal::Call { functor, args } => {
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    self.emit_set_k(b, &rest_after, &bf);
                    return self.emit_call_tail(b, *functor, args, vars);
                }
                LGoal::Metacall(t) => {
                    // Runtime goal walker; the installed k is the
                    // continuation, exactly like a predicate call.
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    self.emit_set_k(b, &rest_after, &bf);
                    let g = self.emit_term(b, t, vars)?;
                    let r = self.fresh();
                    writeln!(b, "  {r} = call i32 @plg_rt_metacall(ptr %m, i64 {g})").unwrap();
                    writeln!(b, "  ret i32 {r}").unwrap();
                    return Ok(());
                }
                LGoal::Disj(a, b2) => {
                    // Cut is transparent in both branches.
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    let bsym = ctx.queue(AuxKind::Seq {
                        goals: goals_of(b2),
                        after: rest_after.clone(),
                        cut_slot,
                    });
                    let t = self.fresh();
                    writeln!(b, "  {t} = ptrtoint ptr @{bsym} to i64").unwrap();
                    writeln!(b, "  call void @plg_rt_push_cp(ptr %m, i64 {t}, i64 {bf})").unwrap();
                    return self.compile_seq(b, &goals_of(a), &rest_after, ctx, vars, cut_slot);
                }
                LGoal::IfThenElse(c, t, e) => {
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    let slot = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, slot);
                    // Cut is transparent in T and E (outer cut_slot)...
                    let else_sym = ctx.queue(AuxKind::Seq {
                        goals: goals_of(e),
                        after: rest_after.clone(),
                        cut_slot,
                    });
                    let then_sym = ctx.queue(AuxKind::CutThenSeq {
                        slot,
                        goals: goals_of(t),
                        after: rest_after,
                        cut_slot,
                    });
                    let p = self.fresh();
                    writeln!(b, "  {p} = ptrtoint ptr @{else_sym} to i64").unwrap();
                    writeln!(b, "  call void @plg_rt_push_cp(ptr %m, i64 {p}, i64 {bf})").unwrap();
                    // ...but call-like (opaque) in the condition: its local
                    // barrier is the height AFTER the else CP.
                    let local = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, local);
                    return self.compile_seq(
                        b,
                        &goals_of(c),
                        &After::Fn(then_sym),
                        ctx,
                        vars,
                        local,
                    );
                }
                LGoal::IfThen(c, t) => {
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    let slot = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, slot);
                    let then_sym = ctx.queue(AuxKind::CutThenSeq {
                        slot,
                        goals: goals_of(t),
                        after: rest_after,
                        cut_slot,
                    });
                    // No CP pushed: the commit slot doubles as C's local
                    // cut barrier.
                    return self.compile_seq(
                        b,
                        &goals_of(c),
                        &After::Fn(then_sym),
                        ctx,
                        vars,
                        slot,
                    );
                }
                LGoal::Once(g) => {
                    // once(G) = commit to G's first solution, continue.
                    let slot = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, slot);
                    let then_sym = ctx.queue(AuxKind::CutThenSeq {
                        slot,
                        goals: rest.to_vec(),
                        after: after.clone(),
                        cut_slot,
                    });
                    // call-like: cut inside G is local (commit slot).
                    return self.compile_seq(
                        b,
                        &goals_of(g),
                        &After::Fn(then_sym),
                        ctx,
                        vars,
                        slot,
                    );
                }
                LGoal::Naf(g) => {
                    let rest_after = self.rest_after(rest, after, ctx, cut_slot);
                    let cont_sym = match &rest_after {
                        After::Fn(s) => s.clone(),
                        After::CallerK => ctx.callerk_jump(),
                    };
                    let slot = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, slot);
                    let p = self.fresh();
                    writeln!(b, "  {p} = ptrtoint ptr @{cont_sym} to i64").unwrap();
                    writeln!(b, "  call void @plg_rt_push_cp(ptr %m, i64 {p}, i64 {bf})").unwrap();
                    let found = ctx.queue(AuxKind::NafFound { slot });
                    // call-like: cut inside G is local — barrier is the
                    // height AFTER the continue-CP.
                    let local = ctx.alloc_scratch();
                    self.emit_capture_height(b, &bf, local);
                    return self.compile_seq(b, &goals_of(g), &After::Fn(found), ctx, vars, local);
                }
                LGoal::Conj(gs) => {
                    let mut combined = gs.clone();
                    combined.extend_from_slice(rest);
                    return self.compile_seq(b, &combined, after, ctx, vars, cut_slot);
                }
            }
            i += 1;
        }
        // Pure-inline sequence (or empty): jump to the continuation.
        self.emit_jump_after(b, after, &bf);
        Ok(())
    }

    /// Drain and emit all functions queued during compile_seq.
    pub fn emit_aux_fns(&mut self, ctx: &mut ClauseCtx) -> Result<(), String> {
        while let Some((sym, kind)) = ctx.work.pop() {
            self.reset_temps();
            ctx.bf = "%bf".to_string();
            let mut b = String::new();
            // Reload clause variables from the body frame.
            let mut vars: HashMap<VarId, String> = HashMap::new();
            for (i, v) in ctx.var_list.clone().into_iter().enumerate() {
                let t = self.fresh();
                writeln!(
                    b,
                    "  {t} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 {})",
                    3 + i
                )
                .unwrap();
                vars.insert(v, t);
            }
            match kind {
                AuxKind::Seq {
                    goals,
                    after,
                    cut_slot,
                } => {
                    self.compile_seq(&mut b, &goals, &after, ctx, &vars, cut_slot)?;
                }
                AuxKind::CutThenSeq {
                    slot,
                    goals,
                    after,
                    cut_slot,
                } => {
                    let h = self.fresh();
                    writeln!(
                        b,
                        "  {h} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 {slot})"
                    )
                    .unwrap();
                    writeln!(b, "  call void @plg_rt_cut(ptr %m, i64 {h})").unwrap();
                    self.compile_seq(&mut b, &goals, &after, ctx, &vars, cut_slot)?;
                }
                AuxKind::NafFound { slot } => {
                    let h = self.fresh();
                    writeln!(
                        b,
                        "  {h} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 {slot})"
                    )
                    .unwrap();
                    writeln!(b, "  call void @plg_rt_cut(ptr %m, i64 {h})").unwrap();
                    writeln!(b, "  ret i32 0").unwrap();
                }
                AuxKind::CallerKJump => {
                    self.emit_jump_after(&mut b, &After::CallerK, "%bf");
                }
            }
            writeln!(self.out, "define internal i32 @{sym}(ptr %m, i64 %bf) {{").unwrap();
            writeln!(self.out, "entry:").unwrap();
            self.out.push_str(&b);
            writeln!(self.out, "fail:").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();
            writeln!(self.out, "}}").unwrap();
        }
        Ok(())
    }

    /// Continuation for "the goals after this control construct".
    fn rest_after(
        &mut self,
        rest: &[LGoal],
        after: &After,
        ctx: &mut ClauseCtx,
        cut_slot: usize,
    ) -> After {
        if rest.is_empty() {
            after.clone()
        } else {
            After::Fn(ctx.queue(AuxKind::Seq {
                goals: rest.to_vec(),
                after: after.clone(),
                cut_slot,
            }))
        }
    }

    /// Store the current choice-point height into a frame scratch slot.
    fn emit_capture_height(&mut self, b: &mut String, bf: &str, slot: usize) {
        let h = self.fresh();
        writeln!(b, "  {h} = call i64 @plg_rt_cp_top(ptr %m)").unwrap();
        writeln!(
            b,
            "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 {slot}, i64 {h})"
        )
        .unwrap();
    }

    /// Install `after` as the machine continuation (before a call).
    pub fn emit_set_k(&mut self, b: &mut String, after: &After, bf: &str) {
        match after {
            After::CallerK => {
                let kf = self.fresh();
                writeln!(
                    b,
                    "  {kf} = call i64 @plg_rt_frame_get(ptr %m, i64 {bf}, i32 0)"
                )
                .unwrap();
                let ke = self.fresh();
                writeln!(
                    b,
                    "  {ke} = call i64 @plg_rt_frame_get(ptr %m, i64 {bf}, i32 1)"
                )
                .unwrap();
                writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {kf}, i64 {ke})").unwrap();
            }
            After::Fn(sym) => {
                let t = self.fresh();
                writeln!(b, "  {t} = ptrtoint ptr @{sym} to i64").unwrap();
                writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {t}, i64 {bf})").unwrap();
            }
        }
    }

    /// Terminate a pure-inline path by transferring to the continuation.
    fn emit_jump_after(&mut self, b: &mut String, after: &After, bf: &str) {
        match after {
            After::CallerK => {
                let kf = self.fresh();
                writeln!(
                    b,
                    "  {kf} = call i64 @plg_rt_frame_get(ptr %m, i64 {bf}, i32 0)"
                )
                .unwrap();
                let ke = self.fresh();
                writeln!(
                    b,
                    "  {ke} = call i64 @plg_rt_frame_get(ptr %m, i64 {bf}, i32 1)"
                )
                .unwrap();
                let kp = self.fresh();
                writeln!(b, "  {kp} = inttoptr i64 {kf} to ptr").unwrap();
                let r = self.fresh();
                writeln!(b, "  {r} = musttail call i32 {kp}(ptr %m, i64 {ke})").unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
            }
            After::Fn(sym) => {
                let r = self.fresh();
                writeln!(b, "  {r} = musttail call i32 @{sym}(ptr %m, i64 {bf})").unwrap();
                writeln!(b, "  ret i32 {r}").unwrap();
            }
        }
    }
}

/// A control-construct branch as a goal list.
fn goals_of(g: &LGoal) -> Vec<LGoal> {
    match g {
        LGoal::Conj(v) => v.clone(),
        other => vec![other.clone()],
    }
}
