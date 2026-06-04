//! Compile one clause to LLVM functions: head unification, then the
//! body as a `musttail` chain of goal calls linked by continuation
//! functions whose state lives in a heap frame.
//!
//! Predicate frame layout (built by the entry function, see
//! predicate.rs):   [arg0 .. arg(A-1), k_fn, k_env]
//! Clause body frame layout: [k_fn, k_env, var0 .. var(V-1)]

use super::term_emit::collect_vars;
use super::{CodeGen, GoalTarget};
use plg_shared::atom::ATOM_TRUE;
use plg_shared::term::VarId;
use plg_shared::{AtomId, Clause, Term};
use std::collections::HashMap;
use std::fmt::Write;

impl CodeGen<'_> {
    /// Emit the clause function `@plg_p<F>_<A>_c<j>` plus its body-goal
    /// continuation functions.
    pub fn emit_clause(
        &mut self,
        functor: AtomId,
        arity: u32,
        j: usize,
        clause: &Clause,
    ) -> Result<(), String> {
        let base = format!("plg_p{functor}_{arity}_c{j}");
        let goals = flatten_body(&clause.body, self.interner.lookup(","));

        // Clause variables in deterministic order (head first).
        let mut var_list: Vec<VarId> = Vec::new();
        collect_vars(&clause.head, &mut var_list);
        for g in &goals {
            collect_vars(g, &mut var_list);
        }

        self.reset_temps();
        let mut b = String::new(); // function body text
        let mut vars: HashMap<VarId, String> = HashMap::new();
        let mut label = 0u32;

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
            label += 1;
            writeln!(
                b,
                "  {u} = call i32 @plg_rt_unify(ptr %m, i64 {arg}, i64 {w})"
            )
            .unwrap();
            let c = self.fresh();
            writeln!(b, "  {c} = icmp ne i32 {u}, 0").unwrap();
            writeln!(b, "  br i1 {c}, label %h{label}, label %fail").unwrap();
            writeln!(b, "h{label}:").unwrap();
        }

        // --- Body.
        match goals.len() {
            0 => {
                // Fact: jump straight to the caller's continuation.
                let (kf, ke) = self.load_pred_k(&mut b, "%f", arity);
                self.emit_musttail_k(&mut b, &kf, &ke);
            }
            1 => {
                // Single goal in tail position: keep the caller's k.
                let (kf, ke) = self.load_pred_k(&mut b, "%f", arity);
                writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {kf}, i64 {ke})").unwrap();
                self.emit_goal_tail(&mut b, &goals[0], &vars)?;
            }
            n => {
                // Body frame: [k_fn, k_env, vars...]
                let (kf, ke) = self.load_pred_k(&mut b, "%f", arity);
                let bf = self.fresh();
                writeln!(
                    b,
                    "  {bf} = call i64 @plg_rt_frame_alloc(ptr %m, i32 {})",
                    2 + var_list.len()
                )
                .unwrap();
                writeln!(
                    b,
                    "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 0, i64 {kf})"
                )
                .unwrap();
                writeln!(
                    b,
                    "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 1, i64 {ke})"
                )
                .unwrap();
                for (i, v) in var_list.iter().enumerate() {
                    let w = &vars[v];
                    writeln!(
                        b,
                        "  call void @plg_rt_frame_set(ptr %m, i64 {bf}, i32 {}, i64 {w})",
                        2 + i
                    )
                    .unwrap();
                }
                let k1 = self.fresh();
                writeln!(b, "  {k1} = ptrtoint ptr @{base}_k1 to i64").unwrap();
                writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {k1}, i64 {bf})").unwrap();
                self.emit_goal_tail(&mut b, &goals[0], &vars)?;

                // Continuation functions for goals 1..n-1.
                for (g, goal) in goals.iter().enumerate().skip(1) {
                    self.emit_continuation(&base, g, n, goal, &var_list)?;
                }
            }
        }

        // Assemble the clause function.
        writeln!(
            self.out,
            "; clause {j} of {}/{arity}",
            self.interner.resolve(functor)
        )
        .unwrap();
        writeln!(self.out, "define internal i32 @{base}(ptr %m, i64 %f) {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        self.out.push_str(&b);
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();
        Ok(())
    }

    /// Continuation after goal `g-1` succeeds: run goal `g`.
    fn emit_continuation(
        &mut self,
        base: &str,
        g: usize,
        n: usize,
        goal: &Term,
        var_list: &[VarId],
    ) -> Result<(), String> {
        self.reset_temps();
        let mut b = String::new();
        // Reload clause variables from the body frame.
        let mut vars: HashMap<VarId, String> = HashMap::new();
        for (i, v) in var_list.iter().enumerate() {
            let t = self.fresh();
            writeln!(
                b,
                "  {t} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 {})",
                2 + i
            )
            .unwrap();
            vars.insert(*v, t);
        }
        if g == n - 1 {
            // Last goal: restore the caller's continuation (LCO).
            let kf = self.fresh();
            writeln!(
                b,
                "  {kf} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 0)"
            )
            .unwrap();
            let ke = self.fresh();
            writeln!(
                b,
                "  {ke} = call i64 @plg_rt_frame_get(ptr %m, i64 %bf, i32 1)"
            )
            .unwrap();
            writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {kf}, i64 {ke})").unwrap();
        } else {
            let kn = self.fresh();
            writeln!(b, "  {kn} = ptrtoint ptr @{base}_k{} to i64", g + 1).unwrap();
            writeln!(b, "  call void @plg_rt_set_k(ptr %m, i64 {kn}, i64 %bf)").unwrap();
        }
        self.emit_goal_tail(&mut b, goal, &vars)?;

        writeln!(
            self.out,
            "define internal i32 @{base}_k{g}(ptr %m, i64 %bf) {{"
        )
        .unwrap();
        writeln!(self.out, "entry:").unwrap();
        self.out.push_str(&b);
        writeln!(self.out, "}}").unwrap();
        Ok(())
    }

    /// Load the caller's continuation out of the predicate frame.
    fn load_pred_k(&mut self, b: &mut String, f: &str, arity: u32) -> (String, String) {
        let kf = self.fresh();
        writeln!(
            b,
            "  {kf} = call i64 @plg_rt_frame_get(ptr %m, i64 {f}, i32 {arity})"
        )
        .unwrap();
        let ke = self.fresh();
        writeln!(
            b,
            "  {ke} = call i64 @plg_rt_frame_get(ptr %m, i64 {f}, i32 {})",
            arity + 1
        )
        .unwrap();
        (kf, ke)
    }

    /// `musttail` into a continuation held as a u64 word.
    fn emit_musttail_k(&mut self, b: &mut String, kf: &str, ke: &str) {
        let kp = self.fresh();
        writeln!(b, "  {kp} = inttoptr i64 {kf} to ptr").unwrap();
        let r = self.fresh();
        writeln!(b, "  {r} = musttail call i32 {kp}(ptr %m, i64 {ke})").unwrap();
        writeln!(b, "  ret i32 {r}").unwrap();
    }

    /// Emit a body goal in tail position: load argument registers and
    /// `musttail` into the callee (the installed k is the continuation).
    fn emit_goal_tail(
        &mut self,
        b: &mut String,
        goal: &Term,
        vars: &HashMap<VarId, String>,
    ) -> Result<(), String> {
        let (functor, args): (AtomId, &[Term]) = match goal {
            Term::Atom(id) => (*id, &[]),
            Term::Compound { functor, args } => (*functor, args),
            other => {
                return Err(format!(
                    "unsupported goal (M2 supports user predicates only): {other:?}"
                ));
            }
        };
        // `fail`/`false` compile to an immediate failure return.
        if args.is_empty() {
            let name = self.interner.resolve(functor);
            if name == "fail" || name == "false" {
                writeln!(b, "  ret i32 0").unwrap();
                return Ok(());
            }
        }
        // Builtins and control constructs are reserved. Like v1, binding
        // is late: the program still compiles, and reaching the goal
        // raises a clear runtime error (instead of miscompiling it as an
        // undefined user predicate). Entries leave reserved_builtin() as
        // milestones implement them.
        if reserved_builtin(self.interner.resolve(functor)).is_some() {
            let r = self.fresh();
            writeln!(
                b,
                "  {r} = call i32 @plg_rt_unsupported_builtin(ptr %m, i32 {functor}, i32 {})",
                args.len()
            )
            .unwrap();
            writeln!(b, "  ret i32 {r}").unwrap();
            return Ok(());
        }
        let arity = args.len() as u32;
        if arity as usize > crate::MAX_GOAL_ARITY {
            return Err(format!(
                "goal arity {arity} exceeds the supported maximum of {}",
                crate::MAX_GOAL_ARITY
            ));
        }
        match self.how_to_call(functor, arity) {
            GoalTarget::Undefined => {
                // v1 contract: existence_error raised when the goal runs.
                let r = self.fresh();
                writeln!(
                    b,
                    "  {r} = call i32 @plg_rt_existence_error(ptr %m, i32 {functor}, i32 {arity})"
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

/// v1's builtin vocabulary, reserved so programs using them fail at
/// compile time with a roadmap pointer instead of miscompiling into
/// existence errors. Entries move out of this table as the milestones
/// implement them.
fn reserved_builtin(name: &str) -> Option<&'static str> {
    const M3: &[&str] = &[
        ";", "->", "\\+", "!", "=", "\\=", "==", "\\==", "is", "<", ">", "=<", ">=", "=:=", "=\\=",
        "@<", "@>", "@=<", "@>=", "once", "compare",
    ];
    const M4: &[&str] = &[
        "call",
        "findall",
        "catch",
        "throw",
        "var",
        "nonvar",
        "atom",
        "number",
        "integer",
        "float",
        "atomic",
        "compound",
        "callable",
        "is_list",
        "functor",
        "arg",
        "=..",
        "copy_term",
        "atom_length",
        "atom_concat",
        "atom_chars",
        "atom_codes",
        "char_code",
        "number_chars",
        "number_codes",
        "msort",
        "sort",
        "between",
        "succ",
        "plus",
        "write",
        "writeln",
        "nl",
        "print",
        "halt",
        "unify_with_occurs_check",
    ];
    if M3.contains(&name) {
        Some("M3")
    } else if M4.contains(&name) {
        Some("M4")
    } else {
        None
    }
}

/// Flatten a parsed body (a single `,`-tree per the frontend) into a
/// goal list, dropping bare `true`. `comma` is the interned id of ","
/// (None when the program never interned it — no conjunctions exist).
pub fn flatten_body(body: &[Term], comma: Option<AtomId>) -> Vec<Term> {
    fn walk(t: &Term, comma: Option<AtomId>, out: &mut Vec<Term>) {
        match t {
            Term::Compound { functor, args } if args.len() == 2 && Some(*functor) == comma => {
                walk(&args[0], comma, out);
                walk(&args[1], comma, out);
            }
            Term::Atom(id) if *id == ATOM_TRUE => {}
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for t in body {
        walk(t, comma, &mut out);
    }
    out
}
