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

#[derive(Clone)]
pub enum LGoal {
    /// User predicate (or dynamic / undefined / reserved-M4 builtin).
    Call {
        functor: AtomId,
        args: Vec<Term>,
    },
    /// A variable goal (`p :- X.`) — metacall, lands with call/1 in M4.
    Metacall(Term),
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
        LGoal::True | LGoal::Fail | LGoal::Cut => {}
    }
}
