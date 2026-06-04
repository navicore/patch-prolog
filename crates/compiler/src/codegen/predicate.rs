//! Per-predicate entry function with first-argument indexing, plus the
//! clause-chaining ("try next clause") retry functions.
//!
//! Entry: bump the step counter, snapshot argument registers + caller
//! continuation + cut barrier into a predicate frame, then dispatch on
//! the dereferenced first argument:
//!
//!   REF (unbound)            → all clauses
//!   ATOM/INT (switch on word)→ clauses keyed to that constant + var-keyed
//!   STR (switch on functor)  → clauses keyed to that functor + var-keyed
//!   LST / FLT / unmatched    → var-keyed clauses only
//!
//! Each distinct candidate list becomes a chain: single candidate ⇒
//! direct musttail with NO choice point (first-arg determinism);
//! multiple ⇒ lazily linked choice points exactly like M2's chains.

use super::CodeGen;
use super::term_emit::{atom_word, int_word};
use plg_shared::{AtomId, Clause, FirstArgKey};
use std::collections::HashMap;
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

        // --- Candidate chains from first-argument keys.
        let keys: Vec<Option<FirstArgKey>> =
            clauses.iter().map(|c| c.head.first_arg_key()).collect();
        let indexable = arity > 0 && keys.iter().any(|k| k.is_some());

        let mut chains: Vec<Vec<usize>> = Vec::new();
        let chain_id = |list: Vec<usize>, chains: &mut Vec<Vec<usize>>| -> usize {
            if let Some(i) = chains.iter().position(|c| *c == list) {
                i
            } else {
                chains.push(list);
                chains.len() - 1
            }
        };
        let all_chain = chain_id((0..n).collect(), &mut chains);
        let (var_chain, key_chains) = if indexable {
            let var_bucket: Vec<usize> = (0..n).filter(|&i| keys[i].is_none()).collect();
            let vc = chain_id(var_bucket, &mut chains);
            // Distinct keys in first-appearance order.
            let mut key_chains: Vec<(FirstArgKey, usize)> = Vec::new();
            for k in keys.iter().flatten() {
                if key_chains.iter().any(|(kk, _)| kk == k) {
                    continue;
                }
                let list: Vec<usize> = (0..n)
                    .filter(|&i| keys[i].is_none() || keys[i].as_ref() == Some(k))
                    .collect();
                let id = chain_id(list, &mut chains);
                key_chains.push((k.clone(), id));
            }
            (vc, key_chains)
        } else {
            (all_chain, Vec::new())
        };

        // --- Entry function.
        self.reset_temps();
        writeln!(
            self.out,
            "; {}/{arity} ({n} clauses{})",
            self.interner.resolve(functor),
            if indexable { ", indexed" } else { "" }
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
        // Predicate frame: [args..., k_fn, k_env, barrier]
        let f = self.fresh();
        writeln!(
            self.out,
            "  {f} = call i64 @plg_rt_frame_alloc(ptr %m, i32 {})",
            arity + 3
        )
        .unwrap();
        let mut arg0 = String::new();
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
            if i == 0 {
                arg0 = a;
            }
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
        // Cut barrier: choice-point height at entry, BEFORE any push_cp.
        let bar = self.fresh();
        writeln!(self.out, "  {bar} = call i64 @plg_rt_cp_top(ptr %m)").unwrap();
        writeln!(
            self.out,
            "  call void @plg_rt_frame_set(ptr %m, i64 {f}, i32 {}, i64 {bar})",
            arity + 2
        )
        .unwrap();

        // --- Dispatch.
        if !indexable {
            self.emit_chain_jump(&base, all_chain, &chains[all_chain], &f);
        } else {
            let d = self.fresh();
            writeln!(
                self.out,
                "  {d} = call i64 @plg_rt_deref(ptr %m, i64 {arg0})"
            )
            .unwrap();
            let tag = self.fresh();
            writeln!(self.out, "  {tag} = and i64 {d}, 7").unwrap();

            // Group key chains by tag.
            let mut atom_cases: Vec<(u64, usize)> = Vec::new();
            let mut int_cases: Vec<(u64, usize)> = Vec::new();
            let mut str_cases: Vec<(u64, usize)> = Vec::new();
            for (k, id) in &key_chains {
                match k {
                    FirstArgKey::Atom(a) => atom_cases.push((atom_word(*a), *id)),
                    FirstArgKey::Integer(i) => int_cases.push((int_word(*i)?, *id)),
                    FirstArgKey::Functor(fu, ar) => {
                        str_cases.push((((*fu as u64) << 32) | *ar as u64, *id))
                    }
                }
            }
            let asw = if atom_cases.is_empty() {
                format!("ch{var_chain}")
            } else {
                "asw".into()
            };
            let isw = if int_cases.is_empty() {
                format!("ch{var_chain}")
            } else {
                "isw".into()
            };
            let ssw = if str_cases.is_empty() {
                format!("ch{var_chain}")
            } else {
                "ssw".into()
            };
            writeln!(
                self.out,
                "  switch i64 {tag}, label %ch{var_chain} [ i64 0, label %ch{all_chain}\n    \
                 i64 1, label %{asw}\n    i64 2, label %{isw}\n    i64 3, label %{ssw} ]"
            )
            .unwrap();
            if !atom_cases.is_empty() {
                writeln!(self.out, "asw:").unwrap();
                self.emit_word_switch(&d, &atom_cases, var_chain);
            }
            if !int_cases.is_empty() {
                writeln!(self.out, "isw:").unwrap();
                self.emit_word_switch(&d, &int_cases, var_chain);
            }
            if !str_cases.is_empty() {
                writeln!(self.out, "ssw:").unwrap();
                let k = self.fresh();
                writeln!(
                    self.out,
                    "  {k} = call i64 @plg_rt_str_key(ptr %m, i64 {d})"
                )
                .unwrap();
                self.emit_word_switch(&k, &str_cases, var_chain);
            }
            // Chain blocks (deduped — emit each used chain once).
            let mut used: Vec<usize> = vec![all_chain, var_chain];
            used.extend(key_chains.iter().map(|(_, id)| *id));
            used.sort_unstable();
            used.dedup();
            for id in used {
                writeln!(self.out, "ch{id}:").unwrap();
                self.emit_chain_jump(&base, id, &chains[id], &f);
            }
        }
        writeln!(self.out, "fail:").unwrap();
        writeln!(self.out, "  ret i32 0").unwrap();
        writeln!(self.out, "}}").unwrap();

        // --- Chain retry functions (for chains with > 1 candidate).
        for (id, list) in chains.iter().enumerate() {
            for p in 1..list.len() {
                self.reset_temps();
                writeln!(
                    self.out,
                    "define internal i32 @{base}_x{id}_t{p}(ptr %m, i64 %f) {{"
                )
                .unwrap();
                writeln!(self.out, "entry:").unwrap();
                if p + 1 < list.len() {
                    let t = self.fresh();
                    writeln!(
                        self.out,
                        "  {t} = ptrtoint ptr @{base}_x{id}_t{} to i64",
                        p + 1
                    )
                    .unwrap();
                    writeln!(
                        self.out,
                        "  call void @plg_rt_push_cp(ptr %m, i64 {t}, i64 %f)"
                    )
                    .unwrap();
                }
                let r = self.fresh();
                writeln!(
                    self.out,
                    "  {r} = musttail call i32 @{base}_c{}(ptr %m, i64 %f)",
                    list[p]
                )
                .unwrap();
                writeln!(self.out, "  ret i32 {r}").unwrap();
                writeln!(self.out, "}}").unwrap();
            }
        }

        // --- Clause functions (shared across chains).
        for (j, clause) in clauses.iter().enumerate() {
            self.emit_clause(functor, arity, j, clause)?;
        }
        Ok(())
    }

    /// Inside the entry function: enter chain `id` (terminates the block).
    fn emit_chain_jump(&mut self, base: &str, id: usize, list: &[usize], f: &str) {
        match list.len() {
            0 => {
                writeln!(self.out, "  ret i32 0").unwrap();
            }
            1 => {
                // Deterministic dispatch: no choice point at all.
                let r = self.fresh();
                writeln!(
                    self.out,
                    "  {r} = musttail call i32 @{base}_c{}(ptr %m, i64 {f})",
                    list[0]
                )
                .unwrap();
                writeln!(self.out, "  ret i32 {r}").unwrap();
            }
            _ => {
                let t = self.fresh();
                writeln!(self.out, "  {t} = ptrtoint ptr @{base}_x{id}_t1 to i64").unwrap();
                writeln!(
                    self.out,
                    "  call void @plg_rt_push_cp(ptr %m, i64 {t}, i64 {f})"
                )
                .unwrap();
                let r = self.fresh();
                writeln!(
                    self.out,
                    "  {r} = musttail call i32 @{base}_c{}(ptr %m, i64 {f})",
                    list[0]
                )
                .unwrap();
                writeln!(self.out, "  ret i32 {r}").unwrap();
            }
        }
    }

    /// `switch` on a tagged word over constant cases.
    fn emit_word_switch(&mut self, scrut: &str, cases: &[(u64, usize)], default_chain: usize) {
        // Dedup case VALUES (two keys can't share a word, but belt+braces).
        let mut seen = HashMap::new();
        let body: Vec<String> = cases
            .iter()
            .filter(|(w, _)| seen.insert(*w, ()).is_none())
            .map(|(w, id)| format!("i64 {w}, label %ch{id}"))
            .collect();
        writeln!(
            self.out,
            "  switch i64 {scrut}, label %ch{default_chain} [ {} ]",
            body.join("\n    ")
        )
        .unwrap();
    }
}
