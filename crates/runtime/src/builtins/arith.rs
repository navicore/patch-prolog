//! Arithmetic expression evaluation (`is/2`, comparisons).
//!
//! Ported byte-for-byte from patch-prolog v1's `builtins.rs` (eval_arith
//! plus the arith_* helpers). Semantics — floored `mod`, truncating `//`,
//! float-yielding `/`, checked i64 overflow, NaN/Infinity rejection, and
//! the exact zero-divisor labels — match v1 so error message text is
//! identical (verified against the v1 oracle; see unit tests).
//!
//! NOTE on the immediate-integer range: cell `INT` is a 61-bit immediate
//! but `eval` computes in full i64. A result that fits i64 yet overflows
//! the i61 immediate cannot be boxed until M4; for M3 the `is/2` ABI
//! reports it as an error (see `pred::plg_rt_b_is`). Evaluation itself
//! never narrows — it always works in i64.

use crate::cell::*;
use crate::machine::Machine;

/// An evaluated arithmetic value: integer or float (mirrors v1 ArithVal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArithValue {
    Int(i64),
    Float(f64),
}

// ---- error raising (structured balls; rendered text is v1-identical) ------

fn overflow(m: &mut Machine, operation: &str) {
    let ctx = format!("Arithmetic error: integer overflow in {operation}");
    crate::errors::evaluation(m, "int_overflow", &ctx);
}

fn zero_divisor(m: &mut Machine, label: &str) {
    let ctx = format!("Division by zero ({label})");
    crate::errors::evaluation(m, "zero_divisor", &ctx);
}

/// v1's `int_args_required`: the culprit term is the placeholder atom id 0,
/// which in v1's interner resolves to "member". We reproduce v1's rendered
/// string verbatim (the culprit is meaningless — a v1 quirk we mirror so
/// output stays byte-identical). See report.
fn int_args_required(m: &mut Machine, op: &str) {
    let culprit = make_atom(m.atoms.intern("member"));
    let ctx = format!("{op} requires integer arguments");
    crate::errors::type_error(m, "integer", culprit, &ctx);
}

fn shift_undefined(m: &mut Machine, op: &str) {
    let ctx = format!("Shift {op} requires a non-negative count in [0, 64)");
    crate::errors::evaluation(m, "undefined", &ctx);
}

/// NaN/Infinity rejection after a float operation (v1 `check_float`).
fn check_float(m: &mut Machine, f: f64) -> Result<ArithValue, ()> {
    if f.is_nan() {
        crate::errors::evaluation(m, "undefined", "Arithmetic error: NaN result");
        Err(())
    } else if f.is_infinite() {
        crate::errors::evaluation(m, "float_overflow", "Arithmetic error: Infinity result");
        Err(())
    } else {
        Ok(ArithValue::Float(f))
    }
}

fn as_f64(v: ArithValue) -> f64 {
    match v {
        ArithValue::Int(n) => n as f64,
        ArithValue::Float(f) => f,
    }
}

// ---- value comparison (v1 arith_lt / arith_gt / arith_eq) ----------------

pub fn arith_lt(a: ArithValue, b: ArithValue) -> bool {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => a < b,
        (Float(a), Float(b)) => a < b,
        (Int(a), Float(b)) => (a as f64) < b,
        (Float(a), Int(b)) => a < (b as f64),
    }
}

pub fn arith_gt(a: ArithValue, b: ArithValue) -> bool {
    arith_lt(b, a)
}

pub fn arith_eq(a: ArithValue, b: ArithValue) -> bool {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => a == b,
        (Float(a), Float(b)) => a == b,
        (Int(a), Float(b)) => (a as f64) == b,
        (Float(a), Int(b)) => a == (b as f64),
    }
}

// ---- the evaluator -------------------------------------------------------

/// Build a predicate-indicator term `Name/Arity` on the heap and return its
/// word. This is the ISO culprit shape for `type_error(evaluable, _)`
/// (ISO 13211-1 §8.6): the non-evaluable functor named as `Name/Arity`,
/// including arity 0 for a bare atom.
fn predicate_indicator(m: &mut Machine, name: u32, arity: i64) -> Word {
    let slash = m.atoms.intern("/");
    let pi = m.heap.len();
    m.heap.push(pack_functor(slash, 2));
    m.heap.push(make_atom(name));
    m.heap.push(make_int(arity));
    make(TAG_STR, pi as u64)
}

/// Evaluate `expr` (a heap word) to an arithmetic value. On `Err(())`,
/// `m.error` is already populated with v1-identical message text. The unit
/// error type is intentional: the rich error lives in `m.error` (the M3 ABI
/// contract), so callers only need to know success vs failure.
#[allow(clippy::result_unit_err)]
pub fn eval(m: &mut Machine, expr: Word) -> Result<ArithValue, ()> {
    let w = m.deref(expr);
    match tag_of(w) {
        TAG_INT => Ok(ArithValue::Int(int_value(w))),
        TAG_BIG => Ok(ArithValue::Int(m.heap[payload(w) as usize] as i64)),
        TAG_FLT => Ok(ArithValue::Float(f64::from_bits(
            m.heap[payload(w) as usize],
        ))),
        TAG_REF => {
            let ctx = format!("Arithmetic error: unbound variable _{}", payload(w));
            crate::errors::instantiation(m, &ctx);
            Err(())
        }
        TAG_ATOM => {
            // ISO 8.6: a non-evaluable atom's culprit is the predicate
            // indicator `Atom/0`, not the bare atom — matching the compound
            // path below (`foo(1)` → `foo/1`).
            let culprit = predicate_indicator(m, atom_id(w), 0);
            crate::errors::type_error(m, "evaluable", culprit, "Cannot evaluate as arithmetic");
            Err(())
        }
        TAG_LST => {
            // A list literal is non-evaluable; keep the bare-term culprit
            // (out of scope of the atom-indicator fix).
            crate::errors::type_error(m, "evaluable", w, "Cannot evaluate as arithmetic");
            Err(())
        }
        TAG_STR => eval_struct(m, w),
        _ => unreachable!("bad tag in arith eval"),
    }
}

fn eval_struct(m: &mut Machine, w: Word) -> Result<ArithValue, ()> {
    let idx = payload(w) as usize;
    let (functor, arity) = unpack_functor(m.heap[idx]);
    let name = m.atoms.resolve(functor).to_string();
    // Evaluate arguments first (left-to-right), matching v1's recursion.
    let a0 = m.heap[idx + 1];
    match (name.as_str(), arity) {
        ("+", 2) => {
            let (a, b) = bin(m, idx)?;
            add(m, a, b)
        }
        ("-", 2) => {
            let (a, b) = bin(m, idx)?;
            sub(m, a, b)
        }
        ("*", 2) => {
            let (a, b) = bin(m, idx)?;
            mul(m, a, b)
        }
        ("/", 2) => {
            let (a, b) = bin(m, idx)?;
            div(m, a, b)
        }
        ("//", 2) => {
            let (a, b) = bin(m, idx)?;
            int_div(m, a, b)
        }
        ("mod", 2) => {
            let (a, b) = bin(m, idx)?;
            modulo(m, a, b)
        }
        ("rem", 2) => {
            let (a, b) = bin(m, idx)?;
            rem(m, a, b)
        }
        ("**", 2) => {
            let (a, b) = bin(m, idx)?;
            pow_float(m, a, b)
        }
        ("^", 2) => {
            let (a, b) = bin(m, idx)?;
            pow(m, a, b)
        }
        ("<<", 2) => {
            let (a, b) = bin(m, idx)?;
            shl(m, a, b)
        }
        (">>", 2) => {
            let (a, b) = bin(m, idx)?;
            shr(m, a, b)
        }
        ("/\\", 2) => {
            let (a, b) = bin(m, idx)?;
            bit_and(m, a, b)
        }
        ("\\/", 2) => {
            let (a, b) = bin(m, idx)?;
            bit_or(m, a, b)
        }
        ("xor", 2) => {
            let (a, b) = bin(m, idx)?;
            bit_xor(m, a, b)
        }
        ("div", 2) => {
            let (a, b) = bin(m, idx)?;
            div_floor(m, a, b)
        }
        ("min", 2) => {
            let (a, b) = bin(m, idx)?;
            Ok(if arith_lt(a, b) { a } else { b })
        }
        ("max", 2) => {
            let (a, b) = bin(m, idx)?;
            Ok(if arith_lt(a, b) { b } else { a })
        }
        ("-", 1) => {
            let a = eval(m, a0)?;
            neg(m, a)
        }
        ("abs", 1) => {
            let a = eval(m, a0)?;
            abs(m, a)
        }
        ("sign", 1) => {
            let a = eval(m, a0)?;
            Ok(sign(a))
        }
        ("\\", 1) => {
            let a = eval(m, a0)?;
            bit_not(m, a)
        }
        _ => {
            // Unknown operator → type_error(evaluable, name/arity).
            let name_id = m.atoms.intern(&name);
            let culprit = predicate_indicator(m, name_id, arity as i64);
            let ctx = format!("Unknown arithmetic operator: {name}/{arity}");
            crate::errors::type_error(m, "evaluable", culprit, &ctx);
            Err(())
        }
    }
}

/// Evaluate the two arguments of a binary STR at heap `idx`.
fn bin(m: &mut Machine, idx: usize) -> Result<(ArithValue, ArithValue), ()> {
    let a = eval(m, m.heap[idx + 1])?;
    let b = eval(m, m.heap[idx + 2])?;
    Ok((a, b))
}

// ---- binary operations (v1 arith_*) --------------------------------------

fn add(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => a
            .checked_add(b)
            .map(Int)
            .ok_or_else(|| overflow(m, "addition")),
        (Float(a), Float(b)) => check_float(m, a + b),
        (Int(a), Float(b)) => check_float(m, a as f64 + b),
        (Float(a), Int(b)) => check_float(m, a + b as f64),
    }
}

fn sub(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => a
            .checked_sub(b)
            .map(Int)
            .ok_or_else(|| overflow(m, "subtraction")),
        (Float(a), Float(b)) => check_float(m, a - b),
        (Int(a), Float(b)) => check_float(m, a as f64 - b),
        (Float(a), Int(b)) => check_float(m, a - b as f64),
    }
}

fn mul(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => a
            .checked_mul(b)
            .map(Int)
            .ok_or_else(|| overflow(m, "multiplication")),
        (Float(a), Float(b)) => check_float(m, a * b),
        (Int(a), Float(b)) => check_float(m, a as f64 * b),
        (Float(a), Int(b)) => check_float(m, a * b as f64),
    }
}

fn div(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    // ISO §9.1.4: (/)/2 always yields a float.
    match (a, b) {
        (_, Int(0)) => {
            zero_divisor(m, "float division");
            Err(())
        }
        (_, Float(0.0)) => {
            zero_divisor(m, "float division");
            Err(())
        }
        (Int(a), Int(b)) => check_float(m, a as f64 / b as f64),
        (Float(a), Float(b)) => check_float(m, a / b),
        (Int(a), Float(b)) => check_float(m, a as f64 / b),
        (Float(a), Int(b)) => check_float(m, a / b as f64),
    }
}

fn modulo(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(_), Int(0)) => {
            zero_divisor(m, "modulo");
            Err(())
        }
        (Int(_), Int(i64::MIN)) => {
            overflow(m, "mod");
            Err(())
        }
        (Int(a), Int(b)) => {
            // ISO mod: result has the sign of the divisor.
            let r = a.rem_euclid(b.abs());
            if b < 0 && r != 0 {
                Ok(Int(r - b.abs()))
            } else {
                Ok(Int(r))
            }
        }
        _ => {
            int_args_required(m, "mod");
            Err(())
        }
    }
}

fn rem(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(_), Int(0)) => {
            zero_divisor(m, "remainder");
            Err(())
        }
        (Int(a), Int(b)) => a.checked_rem(b).map(Int).ok_or_else(|| overflow(m, "rem")),
        _ => {
            int_args_required(m, "rem");
            Err(())
        }
    }
}

fn int_div(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(_), Int(0)) => {
            zero_divisor(m, "integer division");
            Err(())
        }
        (Int(a), Int(b)) => a
            .checked_div(b)
            .map(Int)
            .ok_or_else(|| overflow(m, "division")),
        _ => {
            int_args_required(m, "//");
            Err(())
        }
    }
}

fn div_floor(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(_), Int(0)) => {
            zero_divisor(m, "floor division");
            Err(())
        }
        (Int(a), Int(b)) => {
            let q = match a.checked_div(b) {
                Some(q) => q,
                None => {
                    overflow(m, "floor division");
                    return Err(());
                }
            };
            let r = match a.checked_rem(b) {
                Some(r) => r,
                None => {
                    overflow(m, "floor division");
                    return Err(());
                }
            };
            if r != 0 && (r < 0) != (b < 0) {
                q.checked_sub(1)
                    .map(Int)
                    .ok_or_else(|| overflow(m, "floor division"))
            } else {
                Ok(Int(q))
            }
        }
        _ => {
            int_args_required(m, "div");
            Err(())
        }
    }
}

fn pow_float(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    check_float(m, as_f64(a).powf(as_f64(b)))
}

fn pow(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(base), Int(exp)) if exp >= 0 => {
            let exp_u32 = match u32::try_from(exp) {
                Ok(e) => e,
                Err(_) => {
                    overflow(m, "integer power");
                    return Err(());
                }
            };
            base.checked_pow(exp_u32)
                .map(Int)
                .ok_or_else(|| overflow(m, "integer power"))
        }
        _ => check_float(m, as_f64(a).powf(as_f64(b))),
    }
}

fn shl(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(v), Int(n)) => {
            let bits = match u32::try_from(n) {
                Ok(b) => b,
                Err(_) => {
                    shift_undefined(m, "<<");
                    return Err(());
                }
            };
            if bits >= 64 {
                shift_undefined(m, "<<");
                return Err(());
            }
            v.checked_shl(bits)
                .map(Int)
                .ok_or_else(|| overflow(m, "shift_left"))
        }
        _ => {
            int_args_required(m, "<<");
            Err(())
        }
    }
}

fn shr(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(v), Int(n)) => {
            let bits = match u32::try_from(n) {
                Ok(b) => b,
                Err(_) => {
                    shift_undefined(m, ">>");
                    return Err(());
                }
            };
            if bits >= 64 {
                shift_undefined(m, ">>");
                return Err(());
            }
            v.checked_shr(bits)
                .map(Int)
                .ok_or_else(|| overflow(m, "shift_right"))
        }
        _ => {
            int_args_required(m, ">>");
            Err(())
        }
    }
}

fn bit_and(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => Ok(Int(a & b)),
        _ => {
            int_args_required(m, "/\\");
            Err(())
        }
    }
}

fn bit_or(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => Ok(Int(a | b)),
        _ => {
            int_args_required(m, "\\/");
            Err(())
        }
    }
}

fn bit_xor(m: &mut Machine, a: ArithValue, b: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match (a, b) {
        (Int(a), Int(b)) => Ok(Int(a ^ b)),
        _ => {
            int_args_required(m, "xor");
            Err(())
        }
    }
}

// ---- unary operations ----------------------------------------------------

fn neg(m: &mut Machine, a: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match a {
        Int(n) => n
            .checked_neg()
            .map(Int)
            .ok_or_else(|| overflow(m, "negation")),
        Float(f) => check_float(m, -f),
    }
}

fn abs(m: &mut Machine, a: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match a {
        Int(n) => n.checked_abs().map(Int).ok_or_else(|| overflow(m, "abs")),
        Float(f) => check_float(m, f.abs()),
    }
}

/// `\(I)` — bitwise complement (issue #33). Integer-only, like the binary
/// bitwise operators; `\ 0` evaluates to `-1`.
fn bit_not(m: &mut Machine, a: ArithValue) -> Result<ArithValue, ()> {
    use ArithValue::*;
    match a {
        Int(n) => Ok(Int(!n)),
        Float(_) => {
            int_args_required(m, "\\");
            Err(())
        }
    }
}

fn sign(a: ArithValue) -> ArithValue {
    match a {
        ArithValue::Int(n) => ArithValue::Int(n.signum()),
        ArithValue::Float(f) => ArithValue::Float(f.signum()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plg_shared::StringInterner;

    fn machine() -> Box<Machine> {
        Machine::new(StringInterner::new(), Vec::new())
    }

    /// Build a binary STR `op(a, b)` on the heap, returning its STR word.
    fn bin_str(m: &mut Machine, op: &str, a: Word, b: Word) -> Word {
        let f = m.atoms.intern(op);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, 2));
        m.heap.push(a);
        m.heap.push(b);
        make(TAG_STR, idx as u64)
    }

    fn un_str(m: &mut Machine, op: &str, a: Word) -> Word {
        let f = m.atoms.intern(op);
        let idx = m.heap.len();
        m.heap.push(pack_functor(f, 1));
        m.heap.push(a);
        make(TAG_STR, idx as u64)
    }

    fn flt(m: &mut Machine, f: f64) -> Word {
        let idx = m.heap.len();
        m.heap.push(f.to_bits());
        make(TAG_FLT, idx as u64)
    }

    fn msg(m: &Machine) -> &str {
        m.error.as_ref().unwrap().message.as_str()
    }

    #[test]
    fn happy_paths() {
        let mut m = machine();
        let e = bin_str(&mut m, "+", make_int(2), make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(5)));

        let e = bin_str(&mut m, "*", make_int(4), make_int(5));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(20)));

        // 2.0 + 3 = 5.0
        let two = flt(&mut m, 2.0);
        let e = bin_str(&mut m, "+", two, make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Float(5.0)));

        // 2 ** 3 = 8.0 (float power)
        let e = bin_str(&mut m, "**", make_int(2), make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Float(8.0)));

        // 2 ^ 3 = 8 (integer power)
        let e = bin_str(&mut m, "^", make_int(2), make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(8)));

        // floored mod sign-follows-divisor
        let e = bin_str(&mut m, "mod", make_int(10), make_int(-3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-2)));
        let e = bin_str(&mut m, "mod", make_int(-10), make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(2)));

        // div floors toward -inf
        let e = bin_str(&mut m, "div", make_int(10), make_int(-3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-4)));

        let e = un_str(&mut m, "abs", make_int(-5));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(5)));
        let e = un_str(&mut m, "sign", make_int(-5));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-1)));
        let e = un_str(&mut m, "-", make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-3)));

        // \(I): unary bitwise complement (issue #33). \ 0 = -1, \ 5 = -6.
        let e = un_str(&mut m, "\\", make_int(0));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-1)));
        let e = un_str(&mut m, "\\", make_int(5));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(-6)));

        let e = bin_str(&mut m, "/\\", make_int(5), make_int(3));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(1)));
        let e = bin_str(&mut m, "xor", make_int(3), make_int(5));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(6)));
        let e = bin_str(&mut m, "<<", make_int(5), make_int(1));
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(10)));

        // max/min with mixed types preserve operand type
        let two = flt(&mut m, 2.0);
        let e = bin_str(&mut m, "max", make_int(1), two);
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Float(2.0)));
        let two = flt(&mut m, 2.0);
        let e = bin_str(&mut m, "min", make_int(1), two);
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(1)));
    }

    #[test]
    fn err_zero_divisors() {
        let cases = [
            ("//", "integer division"),
            ("mod", "modulo"),
            ("rem", "remainder"),
            ("div", "floor division"),
        ];
        for (op, label) in cases {
            let mut m = machine();
            let e = bin_str(&mut m, op, make_int(1), make_int(0));
            assert!(eval(&mut m, e).is_err());
            assert_eq!(
                msg(&m),
                format!("error(evaluation_error(zero_divisor), Division by zero ({label}))")
            );
        }
        // (/)/2 zero divisor reports "float division"
        let mut m = machine();
        let e = bin_str(&mut m, "/", make_int(1), make_int(0));
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(zero_divisor), Division by zero (float division))"
        );
    }

    #[test]
    fn err_int_overflow() {
        // INT_MAX is the largest i61 immediate; INT_MAX * INT_MAX ~ 2^120
        // overflows i64, exercising the checked-mul overflow path. (A bare
        // i64::MAX cannot be an immediate INT, so we drive overflow through
        // genuine i61 operands.)
        let mut m = machine();
        let e = bin_str(&mut m, "*", make_int(INT_MAX), make_int(INT_MAX));
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(int_overflow), Arithmetic error: integer overflow in multiplication)"
        );

        // Negation overflow uses the "negation" label (i64::MIN-style edge):
        // nest so the inner value is computed in i64 then negated. INT_MIN is
        // a valid immediate, and -INT_MIN fits, so drive overflow via mul.
        let mut m = machine();
        let e = bin_str(&mut m, "+", make_int(INT_MAX), make_int(INT_MAX));
        // INT_MAX + INT_MAX = 2^61 - 2, fits i64 — succeeds at eval level.
        assert_eq!(eval(&mut m, e), Ok(ArithValue::Int(INT_MAX + INT_MAX)));
    }

    #[test]
    fn err_type_evaluable_atom_and_compound() {
        let mut m = machine();
        let foo = m.atoms.intern("foo");
        assert!(eval(&mut m, make_atom(foo)).is_err());
        assert_eq!(
            msg(&m),
            "error(type_error(evaluable, /(foo, 0)), Cannot evaluate as arithmetic)"
        );

        let mut m = machine();
        let e = un_str(&mut m, "foo", make_int(1));
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(type_error(evaluable, /(foo, 1)), Unknown arithmetic operator: foo/1)"
        );
    }

    #[test]
    fn err_instantiation() {
        let mut m = machine();
        let v = m.new_var();
        assert!(eval(&mut m, v).is_err());
        // payload(v) is the heap index of the var cell.
        let idx = payload(v);
        assert_eq!(
            msg(&m),
            format!("error(instantiation_error, Arithmetic error: unbound variable _{idx})")
        );
    }

    #[test]
    fn err_nan_and_infinity() {
        // 0.0 / 0.0 is caught as zero_divisor (divisor is zero) before NaN.
        let mut m = machine();
        let a = flt(&mut m, 0.0);
        let b = flt(&mut m, 0.0);
        let e = bin_str(&mut m, "/", a, b);
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(zero_divisor), Division by zero (float division))"
        );

        // sqrt is not available; force NaN via 0.0 ** ... no — use a NaN input.
        // Infinity result: 1e308 * 10 overflows to +inf.
        let mut m = machine();
        let big = flt(&mut m, 1.0e308);
        let ten = flt(&mut m, 10.0);
        let e = bin_str(&mut m, "*", big, ten);
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(float_overflow), Arithmetic error: Infinity result)"
        );

        // NaN propagation: NaN + 1.0 → NaN result.
        let mut m = machine();
        let nan = flt(&mut m, f64::NAN);
        let one = flt(&mut m, 1.0);
        let e = bin_str(&mut m, "+", nan, one);
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(undefined), Arithmetic error: NaN result)"
        );
    }

    #[test]
    fn err_int_args_required() {
        let mut m = machine();
        let two = flt(&mut m, 2.0);
        let e = bin_str(&mut m, "mod", make_int(5), two);
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(type_error(integer, member), mod requires integer arguments)"
        );
    }

    #[test]
    fn err_shift_undefined() {
        let mut m = machine();
        let e = bin_str(&mut m, "<<", make_int(1), make_int(64));
        assert!(eval(&mut m, e).is_err());
        assert_eq!(
            msg(&m),
            "error(evaluation_error(undefined), Shift << requires a non-negative count in [0, 64))"
        );
    }

    #[test]
    fn mixed_comparison_helpers() {
        // 1 =:= 1.0 true; 1.0 < 1 false
        assert!(arith_eq(ArithValue::Int(1), ArithValue::Float(1.0)));
        assert!(!arith_lt(ArithValue::Float(1.0), ArithValue::Int(1)));
        assert!(arith_lt(ArithValue::Int(1), ArithValue::Int(2)));
        assert!(arith_gt(ArithValue::Float(2.0), ArithValue::Int(1)));
    }
}
