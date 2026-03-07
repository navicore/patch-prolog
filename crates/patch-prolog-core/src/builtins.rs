use crate::term::{StringInterner, Term};
use crate::unify::Substitution;

/// Check if a goal is a built-in predicate.
pub fn is_builtin(goal: &Term, interner: &StringInterner) -> bool {
    match goal {
        Term::Atom(id) => {
            let name = interner.resolve(*id);
            matches!(name, "true" | "fail" | "false" | "!" | "nl")
        }
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            match (name, args.len()) {
                ("=", 2) | ("\\=", 2) | ("unify_with_occurs_check", 2) | ("is", 2) => true,
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
                // Meta-call
                ("once", 1) | ("call", 1) => true,
                // Atom/string predicates
                ("atom_length", 2) | ("atom_concat", 3) | ("atom_chars", 2) => true,
                // I/O predicates
                ("write", 1) | ("writeln", 1) => true,
                // Term ordering
                ("compare", 3) => true,
                ("@<", 2) | ("@>", 2) | ("@=<", 2) | ("@>=", 2) => true,
                // Term introspection
                ("functor", 3) | ("arg", 3) | ("=..", 2) => true,
                // Integer enumeration
                ("between", 3) => true,
                // Term copying
                ("copy_term", 2) => true,
                // Peano arithmetic
                ("succ", 2) | ("plus", 3) => true,
                // List sorting
                ("msort", 2) | ("sort", 2) => true,
                // Number/string conversion
                ("number_chars", 2) | ("number_codes", 2) => true,
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
    /// once/1: solve goal, take first solution only.
    Once(Term),
    /// call/1: execute a term as a goal.
    Call(Term),
    /// atom_length/2: atom, length
    AtomLength(Term, Term),
    /// atom_concat/3: atom1, atom2, result
    AtomConcat(Term, Term, Term),
    /// atom_chars/2: atom, char list
    AtomChars(Term, Term),
    /// write/1: write term to stdout (no newline).
    Write(Term),
    /// writeln/1: write term to stdout with newline.
    Writeln(Term),
    /// nl/0: write newline to stdout.
    Nl,
    /// compare/3: Order, Term1, Term2 — standard term ordering.
    Compare(Term, Term, Term),
    /// functor/3: Term, Name, Arity.
    Functor(Term, Term, Term),
    /// arg/3: N, Term, Arg.
    Arg(Term, Term, Term),
    /// =../2: Term, List (univ).
    Univ(Term, Term),
    /// between/3: Low, High, X — integer enumeration.
    Between(Term, Term, Term),
    /// copy_term/2: Original, Copy.
    CopyTerm(Term, Term),
    /// succ/2: X, S — successor relation.
    Succ(Term, Term),
    /// plus/3: X, Y, Z — addition relation.
    Plus(Term, Term, Term),
    /// msort/2: List, Sorted.
    MSort(Term, Term),
    /// sort/2: List, Sorted.
    Sort(Term, Term),
    /// number_chars/2: Number, Chars.
    NumberChars(Term, Term),
    /// number_codes/2: Number, Codes.
    NumberCodes(Term, Term),
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
                "nl" => Ok(BuiltinResult::Nl),
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
                ("unify_with_occurs_check", 2) => {
                    if subst.unify_with_occurs_check(&args[0], &args[1]) {
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
                    let walked = subst.apply(&args[0]);
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
                    if let Term::Compound {
                        functor,
                        args: inner_args,
                    } = &left
                    {
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
                ("->", 2) => Ok(BuiltinResult::IfThen(args[0].clone(), args[1].clone())),
                (",", 2) => Ok(BuiltinResult::Conjunction(args[0].clone(), args[1].clone())),
                ("findall", 3) => Ok(BuiltinResult::FindAll(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                ("once", 1) => Ok(BuiltinResult::Once(args[0].clone())),
                ("call", 1) => Ok(BuiltinResult::Call(args[0].clone())),
                // Atom/string predicates
                ("atom_length", 2) => {
                    Ok(BuiltinResult::AtomLength(args[0].clone(), args[1].clone()))
                }
                ("atom_concat", 3) => Ok(BuiltinResult::AtomConcat(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                ("atom_chars", 2) => Ok(BuiltinResult::AtomChars(args[0].clone(), args[1].clone())),
                // I/O
                ("write", 1) => Ok(BuiltinResult::Write(args[0].clone())),
                ("writeln", 1) => Ok(BuiltinResult::Writeln(args[0].clone())),
                // Term ordering
                ("compare", 3) => Ok(BuiltinResult::Compare(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                ("@<", 2) => {
                    let cmp =
                        term_compare(&subst.apply(&args[0]), &subst.apply(&args[1]), interner);
                    if cmp == std::cmp::Ordering::Less {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("@>", 2) => {
                    let cmp =
                        term_compare(&subst.apply(&args[0]), &subst.apply(&args[1]), interner);
                    if cmp == std::cmp::Ordering::Greater {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("@=<", 2) => {
                    let cmp =
                        term_compare(&subst.apply(&args[0]), &subst.apply(&args[1]), interner);
                    if cmp != std::cmp::Ordering::Greater {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                ("@>=", 2) => {
                    let cmp =
                        term_compare(&subst.apply(&args[0]), &subst.apply(&args[1]), interner);
                    if cmp != std::cmp::Ordering::Less {
                        Ok(BuiltinResult::Success)
                    } else {
                        Ok(BuiltinResult::Failure)
                    }
                }
                // Term introspection
                ("functor", 3) => Ok(BuiltinResult::Functor(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                ("arg", 3) => Ok(BuiltinResult::Arg(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                ("=..", 2) => Ok(BuiltinResult::Univ(args[0].clone(), args[1].clone())),
                // Integer enumeration
                ("between", 3) => Ok(BuiltinResult::Between(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                // Term copying
                ("copy_term", 2) => Ok(BuiltinResult::CopyTerm(args[0].clone(), args[1].clone())),
                // Peano arithmetic
                ("succ", 2) => Ok(BuiltinResult::Succ(args[0].clone(), args[1].clone())),
                ("plus", 3) => Ok(BuiltinResult::Plus(
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                )),
                // List sorting
                ("msort", 2) => Ok(BuiltinResult::MSort(args[0].clone(), args[1].clone())),
                ("sort", 2) => Ok(BuiltinResult::Sort(args[0].clone(), args[1].clone())),
                // Number/string conversion
                ("number_chars", 2) => {
                    Ok(BuiltinResult::NumberChars(args[0].clone(), args[1].clone()))
                }
                ("number_codes", 2) => {
                    Ok(BuiltinResult::NumberCodes(args[0].clone(), args[1].clone()))
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
        Term::Var(id) => Err(format!("Arithmetic error: unbound variable _{}", id)),
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
                ("//", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_int_div(&l, &r)
                }
                ("mod", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_mod(&l, &r)
                }
                ("rem", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    arith_rem(&l, &r)
                }
                ("-", 1) => {
                    let v = eval_arith(&args[0], subst, interner)?;
                    arith_neg(&v)
                }
                ("abs", 1) => {
                    let v = eval_arith(&args[0], subst, interner)?;
                    arith_abs(&v)
                }
                ("sign", 1) => {
                    let v = eval_arith(&args[0], subst, interner)?;
                    Ok(arith_sign(&v))
                }
                ("max", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    Ok(arith_max(&l, &r))
                }
                ("min", 2) => {
                    let l = eval_arith(&args[0], subst, interner)?;
                    let r = eval_arith(&args[1], subst, interner)?;
                    Ok(arith_min(&l, &r))
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
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_add(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in addition".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a + b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 + b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a + *b as f64),
    }
}

fn arith_sub(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_sub(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in subtraction".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a - b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 - b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a - *b as f64),
    }
}

fn arith_mul(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_mul(*b)
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
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_div(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in division".to_string()),
        (_, ArithVal::Float(b)) if *b == 0.0 => Err("Division by zero".to_string()),
        (ArithVal::Float(_), ArithVal::Int(0)) => Err("Division by zero".to_string()),
        (ArithVal::Float(a), ArithVal::Float(b)) => check_float(a / b),
        (ArithVal::Int(a), ArithVal::Float(b)) => check_float(*a as f64 / b),
        (ArithVal::Float(a), ArithVal::Int(b)) => check_float(a / *b as f64),
    }
}

fn arith_mod(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(_), ArithVal::Int(0)) => Err("Modulo by zero".to_string()),
        (ArithVal::Int(_), ArithVal::Int(i64::MIN)) => {
            Err("Arithmetic error: integer overflow in mod".to_string())
        }
        (ArithVal::Int(a), ArithVal::Int(b)) => {
            // ISO Prolog mod: result has the sign of the divisor
            // X mod Y = X - floor(X/Y) * Y
            // b.abs() is safe here because we excluded i64::MIN above
            let r = a.rem_euclid(b.abs());
            if *b < 0 && r != 0 {
                Ok(ArithVal::Int(r - b.abs()))
            } else {
                Ok(ArithVal::Int(r))
            }
        }
        _ => Err("mod requires integer arguments".to_string()),
    }
}

/// ISO `//` — truncating integer division (integers only)
fn arith_int_div(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(_), ArithVal::Int(0)) => Err("Division by zero".to_string()),
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_div(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in division".to_string()),
        _ => Err("// requires integer arguments".to_string()),
    }
}

/// ISO `rem` — truncating remainder (sign follows dividend)
fn arith_rem(a: &ArithVal, b: &ArithVal) -> Result<ArithVal, String> {
    match (a, b) {
        (ArithVal::Int(_), ArithVal::Int(0)) => Err("Remainder by zero".to_string()),
        (ArithVal::Int(a), ArithVal::Int(b)) => a
            .checked_rem(*b)
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in rem".to_string()),
        _ => Err("rem requires integer arguments".to_string()),
    }
}

fn arith_neg(a: &ArithVal) -> Result<ArithVal, String> {
    match a {
        ArithVal::Int(n) => n
            .checked_neg()
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in negation".to_string()),
        ArithVal::Float(f) => check_float(-f),
    }
}

fn arith_abs(a: &ArithVal) -> Result<ArithVal, String> {
    match a {
        ArithVal::Int(n) => n
            .checked_abs()
            .map(ArithVal::Int)
            .ok_or_else(|| "Arithmetic error: integer overflow in abs".to_string()),
        ArithVal::Float(f) => check_float(f.abs()),
    }
}

fn arith_sign(a: &ArithVal) -> ArithVal {
    match a {
        ArithVal::Int(n) => ArithVal::Int(n.signum()),
        ArithVal::Float(f) => ArithVal::Float(f.signum()),
    }
}

fn arith_max(a: &ArithVal, b: &ArithVal) -> ArithVal {
    if arith_lt(a, b) {
        b.clone()
    } else {
        a.clone()
    }
}

fn arith_min(a: &ArithVal, b: &ArithVal) -> ArithVal {
    if arith_lt(a, b) {
        a.clone()
    } else {
        b.clone()
    }
}

/// Standard order of terms (ISO Prolog):
/// Variables < Numbers < Atoms < Compound terms
/// Within numbers: by value. Within atoms: alphabetical.
/// Within compounds: by arity, then functor name, then arguments left-to-right.
pub fn term_compare(a: &Term, b: &Term, interner: &StringInterner) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn type_rank(t: &Term) -> u8 {
        match t {
            Term::Var(_) => 0,
            Term::Float(_) => 1,
            Term::Integer(_) => 1,
            Term::Atom(_) => 2,
            Term::List { .. } => 3,
            Term::Compound { .. } => 3,
        }
    }

    let ra = type_rank(a);
    let rb = type_rank(b);
    if ra != rb {
        return ra.cmp(&rb);
    }

    match (a, b) {
        (Term::Var(a), Term::Var(b)) => a.cmp(b),
        (Term::Integer(a), Term::Integer(b)) => a.cmp(b),
        (Term::Float(a), Term::Float(b)) => {
            // NaN sorts after all other floats (deterministic total order)
            a.partial_cmp(b)
                .unwrap_or_else(|| match (a.is_nan(), b.is_nan()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => unreachable!(),
                })
        }
        (Term::Integer(a), Term::Float(b)) => {
            // NaN sorts after everything; ISO: float < integer when same value
            if b.is_nan() {
                return Ordering::Less;
            }
            let cmp = (*a as f64).partial_cmp(b).unwrap_or(Ordering::Less);
            if cmp == Ordering::Equal {
                Ordering::Greater // integer > float for same value (ISO 8.4.2.1)
            } else {
                cmp
            }
        }
        (Term::Float(a), Term::Integer(b)) => {
            // NaN sorts after everything; ISO: float < integer when same value
            if a.is_nan() {
                return Ordering::Greater;
            }
            let cmp = a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Greater);
            if cmp == Ordering::Equal {
                Ordering::Less // float < integer for same value (ISO 8.4.2.1)
            } else {
                cmp
            }
        }
        (Term::Atom(a), Term::Atom(b)) => interner.resolve(*a).cmp(interner.resolve(*b)),
        (
            Term::Compound {
                functor: f1,
                args: a1,
            },
            Term::Compound {
                functor: f2,
                args: a2,
            },
        ) => {
            // Compare by arity first, then functor name, then args
            a1.len()
                .cmp(&a2.len())
                .then_with(|| interner.resolve(*f1).cmp(interner.resolve(*f2)))
                .then_with(|| {
                    for (x, y) in a1.iter().zip(a2.iter()) {
                        let c = term_compare(x, y, interner);
                        if c != Ordering::Equal {
                            return c;
                        }
                    }
                    Ordering::Equal
                })
        }
        (Term::List { .. }, Term::List { .. }) => {
            // Iterative list comparison to avoid stack overflow on long lists
            let mut cur_a = a;
            let mut cur_b = b;
            loop {
                match (cur_a, cur_b) {
                    (Term::List { head: h1, tail: t1 }, Term::List { head: h2, tail: t2 }) => {
                        let c = term_compare(h1, h2, interner);
                        if c != Ordering::Equal {
                            return c;
                        }
                        cur_a = t1;
                        cur_b = t2;
                    }
                    _ => return term_compare(cur_a, cur_b, interner),
                }
            }
        }
        // List vs Compound: lists are .(H,T) which is arity 2
        (
            Term::List { head: h, tail: t },
            Term::Compound {
                functor: f2,
                args: a2,
            },
        ) => {
            // List is ./2; compare arity, then functor ".", then args
            2usize
                .cmp(&a2.len())
                .then_with(|| ".".cmp(interner.resolve(*f2)))
                .then_with(|| {
                    if a2.len() >= 1 {
                        let c = term_compare(h, &a2[0], interner);
                        if c != Ordering::Equal {
                            return c;
                        }
                    }
                    if a2.len() >= 2 {
                        return term_compare(t, &a2[1], interner);
                    }
                    Ordering::Equal
                })
        }
        (
            Term::Compound {
                functor: f1,
                args: a1,
            },
            Term::List { head: h, tail: t },
        ) => a1
            .len()
            .cmp(&2usize)
            .then_with(|| interner.resolve(*f1).cmp("."))
            .then_with(|| {
                if a1.len() >= 1 {
                    let c = term_compare(&a1[0], h, interner);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                if a1.len() >= 2 {
                    return term_compare(&a1[1], t, interner);
                }
                Ordering::Equal
            }),
        _ => unreachable!("term_compare: unhandled Term variant"),
    }
}

/// Collect list elements from a term. Returns None if not a proper list.
pub fn collect_list(term: &Term, interner: &StringInterner) -> Option<Vec<Term>> {
    let mut elements = Vec::new();
    let mut current = term;
    loop {
        match current {
            Term::Atom(id) if interner.resolve(*id) == "[]" => return Some(elements),
            Term::List { head, tail } => {
                elements.push(head.as_ref().clone());
                current = tail;
            }
            _ => return None,
        }
    }
}

/// Build a list term from elements.
pub fn build_list(elements: Vec<Term>, interner: &StringInterner) -> Term {
    let nil_id = interner.lookup("[]").expect("[] must be interned");
    let mut list = Term::Atom(nil_id);
    for elem in elements.into_iter().rev() {
        list = Term::List {
            head: Box::new(elem),
            tail: Box::new(list),
        };
    }
    list
}

/// Check if a term is a proper list (ends with []).
fn is_proper_list(term: &Term, interner: &StringInterner) -> bool {
    let mut current = term;
    loop {
        match current {
            Term::Atom(id) => return interner.resolve(*id) == "[]",
            Term::List { tail, .. } => current = tail,
            _ => return false,
        }
    }
}

/// Helper: check if a goal atom name matches a known builtin name.
pub fn builtin_atom_names() -> &'static [&'static str] {
    &["true", "fail", "false", "!", "nl"]
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
        ("once", 1),
        ("call", 1),
        ("atom_length", 2),
        ("atom_concat", 3),
        ("atom_chars", 2),
        ("write", 1),
        ("writeln", 1),
        ("compare", 3),
        ("@<", 2),
        ("@>", 2),
        ("@=<", 2),
        ("@>=", 2),
        ("functor", 3),
        ("arg", 3),
        ("=..", 2),
        ("between", 3),
        ("copy_term", 2),
        ("succ", 2),
        ("plus", 3),
        ("msort", 2),
        ("sort", 2),
        ("number_chars", 2),
        ("number_codes", 2),
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
        i.intern("//");
        i.intern("rem");
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
    fn test_mod_i64_min_divisor() {
        // arith_mod with i64::MIN divisor should error, not panic from .abs()
        let result = arith_mod(&ArithVal::Int(5), &ArithVal::Int(i64::MIN));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }

    #[test]
    fn test_mod_i64_min_dividend_neg1() {
        // i64::MIN mod -1 should be 0 (rem_euclid handles this correctly)
        let result = arith_mod(&ArithVal::Int(i64::MIN), &ArithVal::Int(-1));
        match result {
            Ok(ArithVal::Int(0)) => {}
            other => panic!("Expected Ok(Int(0)), got {:?}", other),
        }
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

    #[test]
    fn test_arith_abs() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is abs(-5)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(5));
    }

    #[test]
    fn test_arith_abs_positive() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is abs(3)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(3));
    }

    #[test]
    fn test_arith_sign() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is sign(-42)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(-1));
    }

    #[test]
    fn test_arith_sign_zero() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is sign(0)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(0));
    }

    #[test]
    fn test_arith_max() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is max(3, 7)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(7));
    }

    #[test]
    fn test_arith_min() {
        let mut interner = setup();
        let goals = Parser::parse_query("X is min(3, 7)", &mut interner).unwrap();
        let mut subst = Substitution::new();
        let result = exec_builtin(&goals[0], &mut subst, &interner).unwrap();
        assert!(matches!(result, BuiltinResult::Success));
        assert_eq!(subst.walk(&Term::Var(0)), Term::Integer(3));
    }
}
