//! Lower parsed body terms into a goal IR (`LGoal`) the clause compiler
//! consumes. Control constructs and deterministic builtins are
//! recognized here by functor name; everything else is a `Call`.

use plg_shared::{AtomId, Span, Spanned, StringInterner, Term};

/// Arithmetic comparison op codes — ABI contract with
/// `plg_rt_b_arith_cmp` (docs/design/RUNTIME_ABI.md).
pub const ARITH_OPS: &[(&str, i32)] = &[
    ("<", 0),
    (">", 1),
    ("=<", 2),
    (">=", 3),
    ("=:=", 4),
    ("=\\=", 5),
];

/// Term-order op codes — ABI contract with `plg_rt_b_term_cmp`.
pub const ORDER_OPS: &[(&str, i32)] = &[
    ("==", 0),
    ("\\==", 1),
    ("@<", 2),
    ("@>", 3),
    ("@=<", 4),
    ("@>=", 5),
];

/// Deterministic runtime builtins: `(name, arity, C symbol, raises)`. The
/// exact v1 builtin vocabulary — names NOT here (and not control) fall
/// through to existence_error, like v1. Mirrored by the runtime's
/// query-side dispatch (control.rs); the diff corpus guards the pair.
///
/// `raises` (SPANS.md Layer 3): true for builtins that can raise an ISO
/// error (type/instantiation/etc.). Those take a trailing `site_id` arg so
/// the runtime can append `at file:line:col`; pure ones (type checks, I/O)
/// do not. Keep in sync with the runtime signatures — the golden-IR /
/// integration tests catch a mismatch.
pub const DET_BUILTINS: &[(&str, u32, &str, bool)] = &[
    ("var", 1, "plg_rt_b_var_1", false),
    ("nonvar", 1, "plg_rt_b_nonvar_1", false),
    ("atom", 1, "plg_rt_b_atom_1", false),
    ("number", 1, "plg_rt_b_number_1", false),
    ("integer", 1, "plg_rt_b_integer_1", false),
    ("float", 1, "plg_rt_b_float_1", false),
    ("compound", 1, "plg_rt_b_compound_1", false),
    ("is_list", 1, "plg_rt_b_is_list_1", false),
    ("functor", 3, "plg_rt_b_functor_3", true),
    ("arg", 3, "plg_rt_b_arg_3", true),
    ("=..", 2, "plg_rt_b_univ_2", true),
    ("copy_term", 2, "plg_rt_b_copy_term_2", false),
    ("atom_length", 2, "plg_rt_b_atom_length_2", true),
    ("atom_concat", 3, "plg_rt_b_atom_concat_3", true),
    ("atom_chars", 2, "plg_rt_b_atom_chars_2", true),
    ("number_chars", 2, "plg_rt_b_number_chars_2", true),
    ("number_codes", 2, "plg_rt_b_number_codes_2", true),
    ("msort", 2, "plg_rt_b_msort_2", true),
    ("sort", 2, "plg_rt_b_sort_2", true),
    ("succ", 2, "plg_rt_b_succ_2", true),
    ("plus", 3, "plg_rt_b_plus_3", true),
    (
        "unify_with_occurs_check",
        2,
        "plg_rt_b_unify_with_occurs_check_2",
        false,
    ),
    ("write", 1, "plg_rt_b_write_1", false),
    ("writeln", 1, "plg_rt_b_writeln_1", false),
    ("nl", 0, "plg_rt_b_nl_0", false),
];

/// Whether the det builtin with C symbol `sym` raises (and so takes a
/// trailing `site_id` arg). Used by codegen to emit the extra argument.
pub fn det_builtin_raises(sym: &str) -> bool {
    DET_BUILTINS
        .iter()
        .any(|&(_, _, s, raises)| s == sym && raises)
}

/// A lowered goal: its kind plus the source span every goal carries (SPANS.md
/// Layer 3). Reusing `plg_shared::Spanned` keeps provenance uniform — a
/// raising kind reads `g.span` with no per-variant plumbing, and finer
/// granularity later is a parser change, not an IR one.
pub type LGoal = Spanned<LGoalKind>;

#[derive(Clone)]
pub enum LGoalKind {
    /// User predicate (or dynamic / undefined / control builtin routed
    /// through emit_call_tail). The enclosing `LGoal`'s span is the call
    /// site; codegen turns it into a `site_id` for existence_error.
    Call {
        functor: AtomId,
        args: Vec<Term>,
    },
    /// A variable goal (`p :- X.`) — runtime metacall.
    Metacall(Term),
    /// Deterministic runtime builtin executed inline: call the C
    /// symbol with the argument words, branch on the i32 result.
    RtDet {
        sym: &'static str,
        args: Vec<Term>,
    },
    True,
    Fail,
    Cut,
    Unify(Term, Term),
    NotUnify(Term, Term),
    /// `==`, `\==`, `@<`, `@>`, `@=<`, `@>=` (op code per ORDER_OPS).
    TermCmp(i32, Term, Term),
    Compare(Term, Term, Term),
    Is(Term, Term),
    /// `<`, `>`, `=<`, `>=`, `=:=`, `=\=` (op code per ARITH_OPS).
    ArithCmp(i32, Term, Term),
    Disj(Box<LGoal>, Box<LGoal>),
    IfThenElse(Box<LGoal>, Box<LGoal>, Box<LGoal>),
    /// `(C -> T)` with no else: fails when C fails.
    IfThen(Box<LGoal>, Box<LGoal>),
    Naf(Box<LGoal>),
    Once(Box<LGoal>),
    Conj(Vec<LGoal>),
}

pub fn lower_goal(t: &Term, span: Span, interner: &StringInterner) -> Result<LGoal, String> {
    let (name, args): (&str, &[Term]) = match t {
        Term::Atom(id) => (interner.resolve(*id), &[]),
        Term::Compound { functor, args } => (interner.resolve(*functor), args),
        Term::Var(_) => return Ok(Spanned::new(LGoalKind::Metacall(t.clone()), span)),
        other => return Err(format!("goal is not callable: {other:?}")),
    };
    let kind = match (name, args.len()) {
        ("true", 0) => LGoalKind::True,
        ("fail", 0) | ("false", 0) => LGoalKind::Fail,
        ("!", 0) => LGoalKind::Cut,
        (",", 2) => {
            let mut goals = Vec::new();
            flatten_conj(t, span, interner, &mut goals)?;
            LGoalKind::Conj(goals)
        }
        (";", 2) => {
            // `(C -> T ; E)` is if-then-else, not a plain disjunction.
            if let Term::Compound {
                functor,
                args: ite_args,
            } = &args[0]
                && interner.resolve(*functor) == "->"
                && ite_args.len() == 2
            {
                LGoalKind::IfThenElse(
                    Box::new(lower_goal(&ite_args[0], span, interner)?),
                    Box::new(lower_goal(&ite_args[1], span, interner)?),
                    Box::new(lower_goal(&args[1], span, interner)?),
                )
            } else {
                LGoalKind::Disj(
                    Box::new(lower_goal(&args[0], span, interner)?),
                    Box::new(lower_goal(&args[1], span, interner)?),
                )
            }
        }
        ("->", 2) => LGoalKind::IfThen(
            Box::new(lower_goal(&args[0], span, interner)?),
            Box::new(lower_goal(&args[1], span, interner)?),
        ),
        ("\\+", 1) => LGoalKind::Naf(Box::new(lower_goal(&args[0], span, interner)?)),
        ("once", 1) if !matches!(args[0], Term::Var(_)) => {
            LGoalKind::Once(Box::new(lower_goal(&args[0], span, interner)?))
        }
        // once(Var): route through the runtime metacall (the goal walker
        // implements once over runtime-built goals).
        ("once", 1) => LGoalKind::Metacall(t.clone()),
        ("=", 2) => LGoalKind::Unify(args[0].clone(), args[1].clone()),
        ("\\=", 2) => LGoalKind::NotUnify(args[0].clone(), args[1].clone()),
        ("compare", 3) => LGoalKind::Compare(args[0].clone(), args[1].clone(), args[2].clone()),
        ("is", 2) => LGoalKind::Is(args[0].clone(), args[1].clone()),
        _ => {
            if let Some(&(_, op)) = ARITH_OPS.iter().find(|(n, _)| *n == name)
                && args.len() == 2
            {
                LGoalKind::ArithCmp(op, args[0].clone(), args[1].clone())
            } else if let Some(&(_, op)) = ORDER_OPS.iter().find(|(n, _)| *n == name)
                && args.len() == 2
            {
                LGoalKind::TermCmp(op, args[0].clone(), args[1].clone())
            } else if let Some(&(_, _, sym, _)) = DET_BUILTINS
                .iter()
                .find(|(n, a, _, _)| *n == name && *a as usize == args.len())
            {
                LGoalKind::RtDet {
                    sym,
                    args: args.to_vec(),
                }
            } else {
                let functor = match t {
                    Term::Atom(id) => *id,
                    Term::Compound { functor, .. } => *functor,
                    _ => unreachable!(),
                };
                LGoalKind::Call {
                    functor,
                    args: args.to_vec(),
                }
            }
        }
    };
    Ok(Spanned::new(kind, span))
}

/// Flatten a `,`-tree into a goal list (right-associated per the parser).
/// All leaves inherit the enclosing conjunct's `span` (SPANS.md Layer 3:
/// nested goals get coarser-but-present provenance).
fn flatten_conj(
    t: &Term,
    span: Span,
    interner: &StringInterner,
    out: &mut Vec<LGoal>,
) -> Result<(), String> {
    if let Term::Compound { functor, args } = t
        && args.len() == 2
        && interner.resolve(*functor) == ","
    {
        flatten_conj(&args[0], span, interner, out)?;
        flatten_conj(&args[1], span, interner, out)?;
        return Ok(());
    }
    splice_lowered(lower_goal(t, span, interner)?, out);
    Ok(())
}

/// Push a lowered goal, splicing a `Conj` into the flat sequence and dropping
/// a bare `true` — preserving the IR shape the single-`,`-tree path produced.
///
/// Note: splicing a `Conj` discards the wrapper's own span. That is
/// intentional and lossless for provenance — `flatten_conj` already
/// propagated the same span down to every inner leaf, so the per-leaf spans
/// are the source of truth after splicing. The only thing lost is "this whole
/// parenthesized group appeared at X" as a distinct fact; no current consumer
/// needs it (a nested-goal stack trace would be the first that might).
fn splice_lowered(lowered: LGoal, out: &mut Vec<LGoal>) {
    let Spanned { node, span } = lowered;
    match node {
        LGoalKind::True => {}
        LGoalKind::Conj(inner) => out.extend(inner),
        kind => out.push(Spanned::new(kind, span)),
    }
}

/// Lower a clause body: the codegen parser yields top-level conjuncts, each
/// carrying its source span. A conjunct that is itself a `,`-tree (e.g. a
/// parenthesized `(b, c)`) lowers to a `Conj`; splicing it preserves the flat
/// goal sequence the old single-`,`-tree path produced (IR unchanged).
pub fn lower_body(body: &[Spanned<Term>], interner: &StringInterner) -> Result<Vec<LGoal>, String> {
    let mut goals = Vec::new();
    for sp in body {
        splice_lowered(lower_goal(&sp.node, sp.span, interner)?, &mut goals);
    }
    Ok(goals)
}

/// Count the scratch slots a goal tree needs. Commit sites store a
/// choice-point height; ITE and NAF need a SECOND slot for the
/// condition/argument's local cut barrier (cut is opaque there).
pub fn count_scratch(goals: &[LGoal]) -> usize {
    goals.iter().map(scratch_in).sum()
}

fn scratch_in(g: &LGoal) -> usize {
    match &g.node {
        LGoalKind::IfThenElse(c, t, e) => 2 + scratch_in(c) + scratch_in(t) + scratch_in(e),
        LGoalKind::Naf(g) => 2 + scratch_in(g),
        LGoalKind::IfThen(c, t) => 1 + scratch_in(c) + scratch_in(t),
        LGoalKind::Once(g) => 1 + scratch_in(g),
        LGoalKind::Disj(a, b) => scratch_in(a) + scratch_in(b),
        LGoalKind::Conj(gs) => gs.iter().map(scratch_in).sum(),
        _ => 0,
    }
}

/// Collect variables mentioned anywhere in a goal tree (first-appearance
/// order), so the clause frame can carry them.
pub fn collect_goal_vars(g: &LGoal, out: &mut Vec<plg_shared::term::VarId>) {
    use super::term_emit::collect_vars;
    match &g.node {
        LGoalKind::Call { args, .. } => {
            for a in args {
                collect_vars(a, out);
            }
        }
        LGoalKind::Unify(a, b) | LGoalKind::NotUnify(a, b) | LGoalKind::Is(a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoalKind::TermCmp(_, a, b) | LGoalKind::ArithCmp(_, a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoalKind::Compare(o, a, b) => {
            collect_vars(o, out);
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoalKind::Disj(a, b) | LGoalKind::IfThen(a, b) => {
            collect_goal_vars(a, out);
            collect_goal_vars(b, out);
        }
        LGoalKind::IfThenElse(c, t, e) => {
            collect_goal_vars(c, out);
            collect_goal_vars(t, out);
            collect_goal_vars(e, out);
        }
        LGoalKind::Naf(g) | LGoalKind::Once(g) => collect_goal_vars(g, out),
        LGoalKind::Conj(gs) => {
            for g in gs {
                collect_goal_vars(g, out);
            }
        }
        LGoalKind::Metacall(t) => collect_vars(t, out),
        LGoalKind::RtDet { args, .. } => {
            for a in args {
                collect_vars(a, out);
            }
        }
        LGoalKind::True | LGoalKind::Fail | LGoalKind::Cut => {}
    }
}

#[cfg(test)]
mod vocab_invariant {
    //! Codegen half of the `plg-shared::builtins` invariant
    //! (docs/design/BUILTIN_VOCAB.md): the names this crate recognizes —
    //! `DET_BUILTINS` + `ARITH_OPS` + `ORDER_OPS` + the structural
    //! match-arms of `lower_goal`/`clause.rs` — must be EXACTLY the
    //! `BUILTINS` vocabulary. Adding a row to one side without the other
    //! turns red here.
    use super::{ARITH_OPS, DET_BUILTINS, ORDER_OPS};
    use plg_shared::{BUILTINS, builtins::BuiltinKind};
    use std::collections::BTreeSet;

    /// Names recognized by structural match arms in `lower_goal` (and
    /// `clause.rs` for `catch`/`throw`/`findall`/`call`/`between`) — the
    /// only hand-maintained mirror; everything else below is const data.
    #[rustfmt::skip]
    const STRUCTURAL: &[(&str, u32)] = &[
        // inline specials (own LGoal variant)
        ("=", 2), ("\\=", 2), ("is", 2), ("compare", 3),
        // control constructs
        (",", 2), (";", 2), ("->", 2), ("\\+", 1), ("once", 1),
        ("catch", 3), ("throw", 1), ("findall", 3), ("call", 1), ("between", 3),
        // reserved atoms
        ("true", 0), ("fail", 0), ("false", 0), ("!", 0),
    ];

    #[test]
    fn det_builtins_are_det_rows_in_shared() {
        for &(name, arity, _sym, _raises) in DET_BUILTINS {
            let row = BUILTINS
                .iter()
                .find(|s| s.name == name && s.arity == arity)
                .unwrap_or_else(|| panic!("DET_BUILTINS {name}/{arity} missing from BUILTINS"));
            assert_eq!(
                row.kind,
                BuiltinKind::Det,
                "{name}/{arity} is in DET_BUILTINS but not kind Det in BUILTINS"
            );
        }
    }

    #[test]
    fn recognized_names_equal_shared_vocabulary() {
        let mut covered: BTreeSet<(&str, u32)> = BTreeSet::new();
        for &(n, a, _, _) in DET_BUILTINS {
            covered.insert((n, a));
        }
        for &(n, _) in ARITH_OPS {
            covered.insert((n, 2));
        }
        for &(n, _) in ORDER_OPS {
            covered.insert((n, 2));
        }
        covered.extend(STRUCTURAL.iter().copied());

        let vocab: BTreeSet<(&str, u32)> = BUILTINS.iter().map(|s| (s.name, s.arity)).collect();

        assert_eq!(
            covered, vocab,
            "codegen-recognized names diverge from BUILTINS \
             (left = codegen, right = shared table)"
        );
    }
}

#[cfg(test)]
mod span_invariant {
    //! Every lowered top-level goal carries a real source span (PR #16
    //! review #2). A construction path that forgot to set the span would
    //! compile clean but yield a degenerate `Span::point`; this pins it so
    //! Stage 2+ raising builtins reading `g.span` get a meaningful site.
    use super::lower_body;
    use plg_frontend::Parser;
    use plg_shared::StringInterner;

    #[test]
    fn lowered_goals_have_non_degenerate_spans() {
        let mut interner = StringInterner::new();
        let (clauses, _) =
            Parser::parse_program_cg("p :- a, b(X), X is 1 + 2.\n", &mut interner, 0).unwrap();
        let goals = lower_body(&clauses[0].body, &interner).unwrap();
        assert_eq!(goals.len(), 3);
        for g in &goals {
            assert!(g.span.hi > g.span.lo, "degenerate span: {:?}", g.span);
            assert_eq!(g.span.file, 0);
        }
    }
}
