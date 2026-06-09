//! Lower parsed body terms into a goal IR (`LGoal`) the clause compiler
//! consumes. Control constructs and deterministic builtins are
//! recognized here by functor name; everything else is a `Call`.

use plg_shared::{AtomId, StringInterner, Term};

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

/// Deterministic runtime builtins: (name, arity, C symbol). The exact
/// v1 builtin vocabulary — names NOT here (and not control) fall
/// through to existence_error, like v1. Mirrored by the runtime's
/// query-side dispatch (control.rs); the diff corpus guards the pair.
pub const DET_BUILTINS: &[(&str, u32, &str)] = &[
    ("var", 1, "plg_rt_b_var_1"),
    ("nonvar", 1, "plg_rt_b_nonvar_1"),
    ("atom", 1, "plg_rt_b_atom_1"),
    ("number", 1, "plg_rt_b_number_1"),
    ("integer", 1, "plg_rt_b_integer_1"),
    ("float", 1, "plg_rt_b_float_1"),
    ("compound", 1, "plg_rt_b_compound_1"),
    ("is_list", 1, "plg_rt_b_is_list_1"),
    ("functor", 3, "plg_rt_b_functor_3"),
    ("arg", 3, "plg_rt_b_arg_3"),
    ("=..", 2, "plg_rt_b_univ_2"),
    ("copy_term", 2, "plg_rt_b_copy_term_2"),
    ("atom_length", 2, "plg_rt_b_atom_length_2"),
    ("atom_concat", 3, "plg_rt_b_atom_concat_3"),
    ("atom_chars", 2, "plg_rt_b_atom_chars_2"),
    ("number_chars", 2, "plg_rt_b_number_chars_2"),
    ("number_codes", 2, "plg_rt_b_number_codes_2"),
    ("msort", 2, "plg_rt_b_msort_2"),
    ("sort", 2, "plg_rt_b_sort_2"),
    ("succ", 2, "plg_rt_b_succ_2"),
    ("plus", 3, "plg_rt_b_plus_3"),
    (
        "unify_with_occurs_check",
        2,
        "plg_rt_b_unify_with_occurs_check_2",
    ),
    ("write", 1, "plg_rt_b_write_1"),
    ("writeln", 1, "plg_rt_b_writeln_1"),
    ("nl", 0, "plg_rt_b_nl_0"),
];

#[derive(Clone)]
pub enum LGoal {
    /// User predicate (or dynamic / undefined / control builtin routed
    /// through emit_call_tail).
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

pub fn lower_goal(t: &Term, interner: &StringInterner) -> Result<LGoal, String> {
    let (name, args): (&str, &[Term]) = match t {
        Term::Atom(id) => (interner.resolve(*id), &[]),
        Term::Compound { functor, args } => (interner.resolve(*functor), args),
        Term::Var(_) => return Ok(LGoal::Metacall(t.clone())),
        other => return Err(format!("goal is not callable: {other:?}")),
    };
    let g = match (name, args.len()) {
        ("true", 0) => LGoal::True,
        ("fail", 0) | ("false", 0) => LGoal::Fail,
        ("!", 0) => LGoal::Cut,
        (",", 2) => {
            let mut goals = Vec::new();
            flatten_conj(t, interner, &mut goals)?;
            LGoal::Conj(goals)
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
                LGoal::IfThenElse(
                    Box::new(lower_goal(&ite_args[0], interner)?),
                    Box::new(lower_goal(&ite_args[1], interner)?),
                    Box::new(lower_goal(&args[1], interner)?),
                )
            } else {
                LGoal::Disj(
                    Box::new(lower_goal(&args[0], interner)?),
                    Box::new(lower_goal(&args[1], interner)?),
                )
            }
        }
        ("->", 2) => LGoal::IfThen(
            Box::new(lower_goal(&args[0], interner)?),
            Box::new(lower_goal(&args[1], interner)?),
        ),
        ("\\+", 1) => LGoal::Naf(Box::new(lower_goal(&args[0], interner)?)),
        ("once", 1) if !matches!(args[0], Term::Var(_)) => {
            LGoal::Once(Box::new(lower_goal(&args[0], interner)?))
        }
        // once(Var): route through the runtime metacall (the goal walker
        // implements once over runtime-built goals).
        ("once", 1) => LGoal::Metacall(t.clone()),
        ("=", 2) => LGoal::Unify(args[0].clone(), args[1].clone()),
        ("\\=", 2) => LGoal::NotUnify(args[0].clone(), args[1].clone()),
        ("compare", 3) => LGoal::Compare(args[0].clone(), args[1].clone(), args[2].clone()),
        ("is", 2) => LGoal::Is(args[0].clone(), args[1].clone()),
        _ => {
            if let Some(&(_, op)) = ARITH_OPS.iter().find(|(n, _)| *n == name)
                && args.len() == 2
            {
                LGoal::ArithCmp(op, args[0].clone(), args[1].clone())
            } else if let Some(&(_, op)) = ORDER_OPS.iter().find(|(n, _)| *n == name)
                && args.len() == 2
            {
                LGoal::TermCmp(op, args[0].clone(), args[1].clone())
            } else if let Some(&(_, _, sym)) = DET_BUILTINS
                .iter()
                .find(|(n, a, _)| *n == name && *a as usize == args.len())
            {
                LGoal::RtDet {
                    sym,
                    args: args.to_vec(),
                }
            } else {
                let functor = match t {
                    Term::Atom(id) => *id,
                    Term::Compound { functor, .. } => *functor,
                    _ => unreachable!(),
                };
                LGoal::Call {
                    functor,
                    args: args.to_vec(),
                }
            }
        }
    };
    Ok(g)
}

/// Flatten a `,`-tree into a goal list (right-associated per the parser).
fn flatten_conj(t: &Term, interner: &StringInterner, out: &mut Vec<LGoal>) -> Result<(), String> {
    if let Term::Compound { functor, args } = t
        && args.len() == 2
        && interner.resolve(*functor) == ","
    {
        flatten_conj(&args[0], interner, out)?;
        flatten_conj(&args[1], interner, out)?;
        return Ok(());
    }
    match lower_goal(t, interner)? {
        LGoal::True => {}                                 // drop bare true
        LGoal::Conj(mut inner) => out.append(&mut inner), // shouldn't occur, but flatten
        g => out.push(g),
    }
    Ok(())
}

/// Lower a clause body (the parser yields a single `,`-tree per body).
pub fn lower_body(body: &[Term], interner: &StringInterner) -> Result<Vec<LGoal>, String> {
    let mut goals = Vec::new();
    for t in body {
        flatten_conj(t, interner, &mut goals)?;
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
    match g {
        LGoal::IfThenElse(c, t, e) => 2 + scratch_in(c) + scratch_in(t) + scratch_in(e),
        LGoal::Naf(g) => 2 + scratch_in(g),
        LGoal::IfThen(c, t) => 1 + scratch_in(c) + scratch_in(t),
        LGoal::Once(g) => 1 + scratch_in(g),
        LGoal::Disj(a, b) => scratch_in(a) + scratch_in(b),
        LGoal::Conj(gs) => gs.iter().map(scratch_in).sum(),
        _ => 0,
    }
}

/// Collect variables mentioned anywhere in a goal tree (first-appearance
/// order), so the clause frame can carry them.
pub fn collect_goal_vars(g: &LGoal, out: &mut Vec<plg_shared::term::VarId>) {
    use super::term_emit::collect_vars;
    match g {
        LGoal::Call { args, .. } => {
            for a in args {
                collect_vars(a, out);
            }
        }
        LGoal::Unify(a, b) | LGoal::NotUnify(a, b) | LGoal::Is(a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoal::TermCmp(_, a, b) | LGoal::ArithCmp(_, a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoal::Compare(o, a, b) => {
            collect_vars(o, out);
            collect_vars(a, out);
            collect_vars(b, out);
        }
        LGoal::Disj(a, b) | LGoal::IfThen(a, b) => {
            collect_goal_vars(a, out);
            collect_goal_vars(b, out);
        }
        LGoal::IfThenElse(c, t, e) => {
            collect_goal_vars(c, out);
            collect_goal_vars(t, out);
            collect_goal_vars(e, out);
        }
        LGoal::Naf(g) | LGoal::Once(g) => collect_goal_vars(g, out),
        LGoal::Conj(gs) => {
            for g in gs {
                collect_goal_vars(g, out);
            }
        }
        LGoal::Metacall(t) => collect_vars(t, out),
        LGoal::RtDet { args, .. } => {
            for a in args {
                collect_vars(a, out);
            }
        }
        LGoal::True | LGoal::Fail | LGoal::Cut => {}
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
        for &(name, arity, _sym) in DET_BUILTINS {
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
        for &(n, a, _) in DET_BUILTINS {
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
