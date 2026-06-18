//! Fact-table compilation (FACT_TABLE.md, Stages A–C).
//!
//! A predicate whose clauses are all bodyless facts with **ground** head args
//! compiles to one `.rodata` table of words plus two tiny functions — a
//! generic runtime lookup, instead of one function per clause. Data scales
//! where code doesn't: O(1) IR emission, near-instant `clang` at 100k+ facts.
//!
//! Each column is one table word. Immediate columns (atom / i61-int) are
//! stored inline. Non-immediate columns (compound, list, float, big-int) are
//! serialized into a per-predicate `.rodata` blob in the `copyterm`/`TermBuf`
//! cell format; the table cell holds a `STR`/`LST`/`FLT`/`BIG` word whose
//! payload indexes the blob, which the runtime restores onto the heap before
//! unifying. A clause with a body or a NON-ground head arg disqualifies the
//! predicate → it falls back to per-clause codegen unchanged.
//!
//! Delivery to the continuation is a `musttail` in the generated `deliver:`
//! block (the runtime helper returns first), so recursion through a fact
//! predicate keeps a constant C stack — same discipline as compiled clauses.

use super::CodeGen;
use super::term_emit::{IMM_INT_MAX, IMM_INT_MIN, atom_word, int_word};
use plg_frontend::CgClause;
use plg_shared::{AtomId, Term};
use std::fmt::Write;

// Tag constants + word/header packing, mirrored from the runtime's cell.rs —
// the ABI contract for the serialized blob the runtime restores.
const TAG_ATOM: u64 = 1;
const TAG_INT: u64 = 2;
const TAG_STR: u64 = 3;
const TAG_LST: u64 = 4;
const TAG_FLT: u64 = 5;
const TAG_BIG: u64 = 6;

fn mk(tag: u64, payload: u64) -> u64 {
    (payload << 3) | tag
}

fn pack_functor(functor: u32, arity: u32) -> u64 {
    ((functor as u64) << 32) | arity as u64
}

/// A fact table iff every clause is a bodyless fact whose head args are all
/// ground (no variables, however deeply nested). Empty predicates and any
/// clause with a body or a non-ground head arg disqualify it.
pub fn is_fact_predicate(clauses: &[CgClause]) -> bool {
    // A 0-row table would scan fine, but a predicate with no clauses is
    // already routed elsewhere (dynamic fail-stub / nothing to emit), so we
    // don't duplicate that path here.
    !clauses.is_empty()
        && clauses
            .iter()
            .all(|c| c.body.is_empty() && head_args_ground(&c.head))
}

/// Every head argument is ground (the head is an atom, or a compound whose
/// args are all ground terms).
fn head_args_ground(head: &Term) -> bool {
    match head {
        Term::Atom(_) => true,
        Term::Compound { args, .. } => args.iter().all(is_ground),
        _ => false,
    }
}

fn is_ground(t: &Term) -> bool {
    match t {
        Term::Atom(_) | Term::Integer(_) | Term::Float(_) => true,
        Term::Var(_) => false,
        Term::Compound { args, .. } => args.iter().all(is_ground),
        Term::List { head, tail } => is_ground(head) && is_ground(tail),
    }
}

/// Serialize one ground head argument to its table word, appending any
/// non-immediate structure to `blob` (the `copyterm`/`TermBuf` cell format:
/// pointer payloads index `blob`). Immediates (atom / i61-int) return their
/// word with nothing appended. Iterative (a work queue) so a deep ground term
/// — e.g. a long list — never overflows the compiler stack.
fn serialize_arg(root: &Term, blob: &mut Vec<u64>) -> u64 {
    let mut work: Vec<(&Term, usize)> = Vec::new();
    let r = xlate(root, blob, &mut work);
    while let Some((t, dst)) = work.pop() {
        let w = xlate(t, blob, &mut work);
        blob[dst] = w;
    }
    r
}

fn xlate<'a>(t: &'a Term, blob: &mut Vec<u64>, work: &mut Vec<(&'a Term, usize)>) -> u64 {
    match t {
        Term::Atom(id) => atom_word(*id),
        Term::Integer(n) if (IMM_INT_MIN..=IMM_INT_MAX).contains(n) => {
            int_word(*n).expect("immediate range checked")
        }
        Term::Integer(n) => {
            let b = blob.len();
            blob.push(*n as u64); // BIG cell: raw i64 bits
            mk(TAG_BIG, b as u64)
        }
        Term::Float(f) => {
            let b = blob.len();
            blob.push(f.to_bits()); // FLT cell: f64 bits
            mk(TAG_FLT, b as u64)
        }
        Term::Compound { functor, args } => {
            let b = blob.len();
            blob.push(pack_functor(*functor, args.len() as u32)); // [functor|arity]
            blob.resize(b + 1 + args.len(), 0); // reserve arg cells
            for (k, a) in args.iter().enumerate() {
                work.push((a, b + 1 + k));
            }
            mk(TAG_STR, b as u64)
        }
        Term::List { head, tail } => {
            let b = blob.len();
            blob.push(0); // [head]
            blob.push(0); // [tail]
            work.push((head, b));
            work.push((tail, b + 1));
            mk(TAG_LST, b as u64)
        }
        Term::Var(_) => unreachable!("is_fact_predicate guarantees ground args"),
    }
}

/// Emit a `private` `.rodata` `[N x i64]` global (or an empty
/// `zeroinitializer` for N = 0).
fn emit_words_global(out: &mut String, name: &str, words: &[u64]) {
    if words.is_empty() {
        writeln!(
            out,
            "@{name} = private unnamed_addr constant [0 x i64] zeroinitializer"
        )
        .unwrap();
    } else {
        let body = words
            .iter()
            .map(|w| format!("i64 {w}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "@{name} = private unnamed_addr constant [{} x i64] [{body}]",
            words.len()
        )
        .unwrap();
    }
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
        let acount = arity as usize;

        // --- Serialize each row, column-major within a row: immediate columns
        //     inline, non-immediate columns into the blob (table cell = a
        //     blob-ref word).
        let mut table: Vec<u64> = Vec::with_capacity(nrows * acount);
        let mut blob: Vec<u64> = Vec::new();
        for c in clauses {
            let args: &[Term] = match &c.head {
                Term::Compound { args, .. } => args,
                _ => &[], // arity 0
            };
            for a in args {
                let cell = serialize_arg(a, &mut blob);
                table.push(cell);
            }
        }

        writeln!(
            self.out,
            "; {}/{arity} ({nrows} facts \u{2192} table)",
            self.interner.resolve(functor)
        )
        .unwrap();
        emit_words_global(&mut self.out, &tbl, &table);

        // --- Serialized-term blob for non-immediate columns (Stage C). Absent
        //     when every column is an immediate; the entry passes a null blob
        //     pointer then.
        let has_blob = !blob.is_empty();
        if has_blob {
            emit_words_global(&mut self.out, &format!("{tbl}_blob"), &blob);
        }
        let blob_len = blob.len();

        // --- First-argument index (Stage B): only when column 0 is
        //     all-immediate (a blob-ref column 0 can't be u64-key-sorted, and
        //     arity-0 has no first column). Row indices sorted by column 0,
        //     ties keeping program order, for an O(log n) bound-key lookup.
        let has_index =
            acount >= 1 && (0..nrows).all(|r| matches!(table[r * acount] & 7, TAG_ATOM | TAG_INT));
        if has_index {
            let mut order: Vec<usize> = (0..nrows).collect();
            order.sort_by_key(|&r| (table[r * acount], r));
            let idxwords: Vec<u64> = order.iter().map(|&r| r as u64).collect();
            emit_words_global(&mut self.out, &format!("{tbl}_idx"), &idxwords);
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
        // Index pointer: the sorted index when column 0 is all-immediate, else
        // null (full scan).
        let ip = if has_index {
            let ip = self.fresh();
            writeln!(self.out, "  {ip} = ptrtoint ptr @{tbl}_idx to i64").unwrap();
            ip
        } else {
            "0".to_string()
        };
        // Blob pointer: the serialized-term section when any column is
        // non-immediate, else null.
        let bp = if has_blob {
            let bp = self.fresh();
            writeln!(self.out, "  {bp} = ptrtoint ptr @{tbl}_blob to i64").unwrap();
            bp
        } else {
            "0".to_string()
        };
        let rp = self.fresh();
        writeln!(self.out, "  {rp} = ptrtoint ptr @{sym}_ftr to i64").unwrap();
        let ok = self.fresh();
        writeln!(
            self.out,
            "  {ok} = call i32 @plg_rt_fact_first(ptr %m, i64 {tp}, i64 {ip}, i64 {bp}, i64 {blob_len}, i64 {nrows}, i64 {arity}, i64 {rp})"
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
