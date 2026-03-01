use crate::term::{StringInterner, Term};
use crate::unify::Substitution;

/// Check if a goal is a built-in predicate.
pub fn is_builtin(goal: &Term, interner: &StringInterner) -> bool {
    match goal {
        Term::Atom(id) => {
            let name = interner.resolve(*id);
            matches!(name, "true" | "fail" | "false" | "!")
        }
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            match (name, args.len()) {
                ("=", 2) | ("\\=", 2) | ("is", 2) => true,
                ("<", 2) | (">", 2) | ("=<", 2) | (">=", 2) => true,
                ("=:=", 2) | ("=\\=", 2) => true,
                ("\\+", 1) => true,
                // Type-checking predicates
                ("var", 1) | ("nonvar", 1) | ("atom", 1) | ("number", 1) => true,
                ("integer", 1) | ("float", 1) | ("compound", 1) | ("is_list", 1) => true,
                // Control flow
                (";", 2) | ("->", 2) | (",", 2) => true,
                // Solution collection
                ("findall", 3) => true,
                _ => false,
            }
        }
        _ => false,
    }
}

/// Result of executing a builtin.
#[derive(Debug)]
pub enum BuiltinResult {
    /// The builtin succeeded (substitution may have been modified).
    Success,
    /// The builtin failed.
    Failure,
    /// Cut: succeed and signal cut to the solver.
    Cut,
    /// Negation-as-failure: the solver needs to try the inner goal.
    NegationAsFailure(Term),
    /// Disjunction: try left, then right on backtracking.
    Disjunction(Term, Term),
    /// If-then-else: ;(->(Cond, Then), Else)
    IfThenElse(Term, Term, Term),
    /// If-then (no else): ->(Cond, Then)
    IfThen(Term, Term),
    /// Conjunction: ','(A, B) — flatten into goal list.
    Conjunction(Term, Term),
    /// findall/3: Template, Goal, Result list.
    FindAll(Term, Term, Term),
}

/// Execute a built-in predicate.
pub fn exec_builtin(
    goal: &Term,
    subst: &mut Substitution,
    interner: &StringInterner,
) -> Result<BuiltinResult, String> {
    match goal {
        Term::Atom(id) => {
            let name = interner.resolve(*id);
            match name {
                "true" => Ok(BuiltinResult::Success),
                "fail" | "false" => Ok(BuiltinResult::Failure),
                "!" => Ok(BuiltinResult::Cut),
                _ => Err(format!("Unknown builtin atom: {}", name)),
            }
        }
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            match (name, args.len()) {
                ("=", 2) => {
                    if subst.unify(&args[0], &args[1]) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("\\=", 2) => {
                    let mark = subst.trail_mark();
                    if subst.unify(&args[0], &args[1]) {
                        subst.undo_to(mark);
                        Ok(BuiltinResult::Failure)
                    } else {
                        subst.undo_to(mark);
                        Ok(BuiltinResult::Success)
                    }
                }
                ("is", 2) => {
                    let result = eval_arith(&args[1], subst, interner)?;
                    let result_term = arith_to_term(result);
                    if subst.unify(&args[0], &result_term) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("<", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if arith_lt(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                (">", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if arith_gt(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("=<", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if !arith_gt(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                (">=", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if !arith_lt(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("=:=", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if arith_eq(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("=\\=", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    if !arith_eq(&l, &r) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("\\+", 1) => Ok(BuiltinResult::NegationAsFailure(args[0].clone())),
                // Type-checking predicates
                ("var", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Var(_)) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("nonvar", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Var(_)) {
                        Ok(BuiltinResult::Failure)
                    } else {
                        Ok(BuiltinResult::Success)
                    }
                }
                ("atom", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Atom(_)) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("number", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Integer(_) | Term::Float(_)) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("integer", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Integer(_)) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("float", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Float(_)) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("compound", 1) => {
                    let walked = subst.walk(&args[0]);
                    if matches!(walked, Term::Compound { .. } | Term::List { .. }) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("is_list", 1) => {
                    let walked = subst.walk(&args[0]);
                    if is_proper_list(&walked, interner) {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                // Control flow
                (";", 2) => {
                    // Check if left arg is ->(Cond, Then) => if-then-else
                    let left = subst.walk(&args[0]);
                    if let Term::Compound { functor, args: inner_args } = &left {
                        if interner.resolve(*functor) == "->" && inner_args.len() == 2 {
                            return Ok(BuiltinResult::IfThenElse(
                                inner_args[0].clone(),
                                inner_args[1].clone(),
                                args[1].clone(),
                            ));
                        }
                    }
                    // Plain disjunction
                    Ok(BuiltinResult::Disjunction(args[0].clone(), args[1].clone()))
                }
                ("->", 2) => {
                    Ok(BuiltinResult::IfThen(args[0].clone(), args[1].clone()))
                }
                (",", 2) => {
                    Ok(BuiltinResult::Conjunction(args[0].clone(), args[1].clone()))
                }
                ("findall", 3) => {
                    Ok(BuiltinResult::FindAll(
                        args[0].clone(),
                        args[1].clone(),
                        args[2].clone(),
                    ))
                }
                _ => Err(format!("Unknown builtin: {}/{}", name, args.len())),
            }
        }
        _ => Err(format!("Cannot execute as builtin: {:?}", goal)),
    }
}

/// Arithmetic value: either integer or float.
#[derive(Debug, Clone)]
enum ArithVal {
    Int(i64),
    Float(f64),
}

fn arith_to_term(val: ArithVal) -> Term {
    match val {
        ArithVal::Int(n) => Term::Integer(n),
        ArithVal::Float(f) => Term::Float(f),
    }
}

fn arith_lt(a: &ArithVal, b: &ArithVal) -> bool {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a < b,
        (ArithVal::Float(a), ArithVal::Float(b)) => a < b,
        (ArithVal::Int(a), ArithVal::Float(b)) => (*a as f64) < *b,
        (ArithVal::Float(a), ArithVal::Int(b)) => *a < (*b as f64),
    }
}

fn arith_gt(a: &ArithVal, b: &ArithVal) -> bool {
    arith_lt(b, a)
}

fn arith_eq(a: &ArithVal, b: &ArithVal) -> bool {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a == b,
        (ArithVal::Float(a), ArithVal::Float(b)) => a == b,
        (ArithVal::Int(a), ArithVal::Float(b)) => (*a as f64) == *b,
        (ArithVal::Float(a), ArithVal::Int(b)) => *a == (*b as f64),
    }
}

/// Evaluate an arithmetic expression.
fn eval_arith(
    term: &Term,
    subst: &Substitution,
    interner: &StringInterner,
) -> Result<ArithVal, String> {
    let term = subst.walk(term);
    match &term {
        Term::Integer(n) => Ok(ArithVal::Int(*n)),
        Term::Float(f) => Ok(ArithVal::Float(*f)),
        Term::Var(id) => Err(format!(
            "Arithmetic error: unbound variable _{}", id
        )),
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            match (name, args.len()) {
                ("+", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_add(&l, &r)
                }
                ("-", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_sub(&l, &r)
                }
                ("*", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_mul(&l, &r)
                }
                ("/", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_div(&l, &r)
                }
                ("mod", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_mod(&l, &r)
                }
                ("-", 1) => {
                    let v = eval_arith(&args[0], subst, interner)?;
                    arith_neg(&v)
                }
                _ => Err(format!(
                    "Unknown arithmetic operator: {}/{}",
                    name,
                    args.len()
                )),
            }
        }
        _ => Err(format!("Cannot evaluate as arithmetic: {:?}", term)),
    }
}

/// Check a float result for NaN or Infinity, returning an error if detected.
fn check_float(f: f64) -> Result<ArithVal, String> {
    if f.is_nan() {
        Err("Arithmetic error: NaN result".to_string())
    } else if f.is_infinite() {
        Err("Arithmetic error: Infinity result".to_string())
    } else {
        Ok(ArithVal::Float(f))
    }
}

fn arith_add(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a.checked_add(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in addition".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a + b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 + b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a + *b as f64),
    }
}

fn arith_sub(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a.checked_sub(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in subtraction".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a - b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 - b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a - *b as f64),
    }
}

fn arith_mul(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a.checked_mul(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in multiplication".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a * b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 * b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a * *b as f64),
    }
}

fn arith_div(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(_), ArithVal::Int(0)) => Err("Division by zero".to_string()),
        (ArithVal::Int(a), ArithVal::Int(b)) => a.checked_div(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in division".to_string()),
        (_, ArithVal::Float(b)) if *b == 0.0 => Err("Division by zero".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a / b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 / b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a / *b as f64),
    }
}

fn arith_mod(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(_), ArithVal::Int(0)) => Err("Modulo by zero".to_string()),
        (ArithVal::Int(a), ArithVal::Int(b)) => Ok(ArithVal::Int(a % b)),
        _ => Err("mod requires integer arguments".to_string()),
    }
}

fn arith_neg(a: &ArithVal) -> Result<ArithVal, String> {
    match a {
        ArithVal::Int(n) => n.checked_neg()
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in negation".to_string()),
        ArithVal::Float(f) => check_float(-f),
    }
}

/// Check if a term is a proper list (ends with []).
fn is_proper_list(term: &Term, interner: &StringInterner) -> bool {
    match term {
        Term::Atom(id) => interner.resolve(*id) == "[]",
        Term::List { tail, .. } => is_proper_list(tail, interner),
        _ => false,
    }
}

/// Helper: check if a goal atom name matches a known builtin name.
pub fn builtin_atom_names() -> &'static [&'static str] {
    &["true", "fail", "false", "!"]
}

pub fn builtin_functor_names() -> &'static [(&'static str, usize)] {
    &[
        ("=", 2),
        ("\\=", 2),
        ("is", 2),
        ("<", 2),
        (">", 2),
        ("=<", 2),
        (">=", 2),
        ("=:=", 2),
        ("=\\=", 2),
        ("\\+", 1),
        ("var", 1),
        ("nonvar", 1),
        ("atom", 1),
        ("number", 1),
        ("integer", 1),
        ("float", 1),
        ("compound", 1),
        ("is_list", 1),
        (";", 2),
        ("->", 2),
        (",", 2),
        ("findall", 3),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn setup() -> StringInterner {
        let mut i = StringInterner::new();
        // Pre-intern common atoms
        i.intern("true");
        i.intern("fail");
        i.intern("!");
        i.intern("=");
        i.intern("\\=");
        i.intern("is");
        i.intern("<");
        i.intern(">");
        i.intern("=<");
        i.intern(">=");
        i.intern("=:=");
        i.intern("=\\=");
        i.intern("\\+");
        i.intern("+");
        i.intern("-");
        i.intern("*");
        i.intern("/");
        i.intern("mod");
        i
    }

    #[test]
    fn test_is_builtin() {
        let interner = setup();
        let true_id = interner.lookup("true").unwrap();
        assert!(is_builtin(&Term::Atom(true_id), &interner));

        let eq_id = interner.lookup("=").unwrap();
        let goal = Term::Compound {
            functor: eq_id,
            args: vec![Term::Var(0), Term::Atom(0)],
        };
        assert!(is_builtin(&goal, &interner));
    }

    #[test]
    fn test_exec_true() {
        let interner = setup();
        let true_id = interner.lookup("true").unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&Term::Atom(true_id), &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
    }

    #[test]
    fn test_exec_fail() {
        let interner = setup();
        let fail_id = interner.lookup("fail").unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&Term::Atom(fail_id), &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Failure));
    }

    #[test]
    fn test_exec_unify() {
        let interner = setup();
        let eq_id = interner.lookup("=").unwrap();
        let mut subst = Substitution::new();
        let goal = Term::Compound {
            functor: eq_id,
            args: vec![Term::Var(0), Term::Integer(42)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(42));
    }

    #[test]
    fn test_exec_not_unify() {
        let interner = setup();
        let neq_id = interner.lookup("\\=").unwrap();
        let mut subst = Substitution::new();
        // 1 \= 2 should succeed
        let goal = Term::Compound {
            functor: neq_id,
            args: vec![Term::Integer(1), Term::Integer(2)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));

        // 1 \= 1 should fail
        let goal = Term::Compound {
            functor: neq_id,
            args: vec![Term::Integer(1), Term::Integer(1)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Failure));
    }

    #[test]
    fn test_exec_is_arithmetic() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is 2 + 3 * 4", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(14));
    }

    #[test]
    fn test_exec_comparison() {
        let interner = setup();
        let lt_id = interner.lookup("<").unwrap();
        let mut subst = Substitution::new();

        // 1 < 2 should succeed
        let goal = Term::Compound {
            functor: lt_id,
            args: vec![Term::Integer(1), Term::Integer(2)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Success
        ));

        // 2 < 1 should fail
        let goal = Term::Compound {
            functor: lt_id,
            args: vec![Term::Integer(2), Term::Integer(1)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Failure
        ));
    }

    #[test]
    fn test_exec_cut() {
        let interner = setup();
        let cut_id = interner.lookup("!").unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&Term::Atom(cut_id), &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Cut));
    }

    #[test]
    fn test_type_checking_var() {
        let mut interner = setup();
        interner.intern("var");
        let var_id = interner.lookup("var").unwrap();
        let mut subst = Substitution::new();
        // var(X) where X is unbound should succeed
        let goal = Term::Compound {
            functor: var_id,
            args: vec![Term::Var(0)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));

        // var(42) should fail
        let goal = Term::Compound {
            functor: var_id,
            args: vec![Term::Integer(42)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Failure));
    }

    #[test]
    fn test_type_checking_atom() {
        let mut interner = setup();
        interner.intern("atom");
        let atom_id = interner.lookup("atom").unwrap();
        let mut subst = Substitution::new();
        let hello = interner.intern("hello");
        // atom(hello) should succeed
        let goal = Term::Compound {
            functor: atom_id,
            args: vec![Term::Atom(hello)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));

        // atom(42) should fail
        let goal = Term::Compound {
            functor: atom_id,
            args: vec![Term::Integer(42)],
        };
        let result = exec_builtin(&goal, &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Failure));
    }

    #[test]
    fn test_type_checking_integer() {
        let mut interner = setup();
        interner.intern("integer");
        let int_id = interner.lookup("integer").unwrap();
        let mut subst = Substitution::new();
        let goal = Term::Compound {
            functor: int_id,
            args: vec![Term::Integer(42)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Success
        ));

        let goal = Term::Compound {
            functor: int_id,
            args: vec![Term::Float(3.14)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Failure
        ));
    }

    #[test]
    fn test_type_checking_number() {
        let mut interner = setup();
        interner.intern("number");
        let num_id = interner.lookup("number").unwrap();
        let mut subst = Substitution::new();
        // number(42) should succeed
        let goal = Term::Compound {
            functor: num_id,
            args: vec![Term::Integer(42)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Success
        ));
        // number(3.14) should succeed
        let goal = Term::Compound {
            functor: num_id,
            args: vec![Term::Float(3.14)],
        };
        assert!(matches!(
            exec_builtin(&goal, &mut subst, &interner).unwrap(),
            BuiltinResult::Success
        ));
    }

    #[test]
    fn test_exec_mod() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is 10 mod 3", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(1));
    }

    #[test]
    fn test_integer_overflow_add() {
        let mut interner = setup();
        let query_str = format!("X is {} + 1", i64::MAX);
        let goals = Parser::parse_query(&query_str, &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }

    #[test]
    fn test_integer_overflow_mul() {
        let mut interner = setup();
        let query_str = format!("X is {} * 2", i64::MAX);
        let goals = Parser::parse_query(&query_str, &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }
}
