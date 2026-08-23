//! What the operators do.
//!
//! Split out from the interpreter because these are the rules most likely to be
//! got subtly wrong, and because they can then be tested on their own without
//! standing a whole script up.
//!
//! The three that catch people:
//!
//! - **`+` is overloaded and `&` is not.** `"1" + "2"` is `"12"`; `1 + "2"` is
//!   `3`; `"a" + 1` is a type mismatch. `&` always concatenates.
//! - **`\` and `Mod` round their operands first**, to even, and then work on
//!   integers. `7.5 \ 2` is `4`, not `3`, because `7.5` rounds to `8`.
//! - **The logical operators are bitwise.** `And` on two Booleans behaves like
//!   the logical one only because `True` is `-1`, all bits set. `5 And 3` is
//!   `1`, and tables use that on purpose for switch masks.
//!
//! And one that catches people harder: **`And` and `Or` do not short-circuit.**
//! `If Not IsNull(x) And x.Foo Then` evaluates `x.Foo` whatever `x` is. Tables
//! are written knowing this and nest their `If`s instead; an implementation
//! that helpfully short-circuits would change which errors a table raises.

use std::rc::Rc;

use crate::ast::{BinOp, UnOp};
use crate::error::{Error, Result};
use crate::instance::same_instance;
use crate::value::{Value, round_half_to_even};

/// Applies a unary operator.
pub fn unary(op: UnOp, v: &Value) -> Result<Value> {
    match op {
        UnOp::Plus => {
            // Accepted and ignored, but it still coerces: `+"abc"` is an error.
            if v.is_null() {
                return Ok(Value::Null);
            }
            Ok(Value::from_number(v.to_number()?))
        }
        UnOp::Neg => {
            if v.is_null() {
                return Ok(Value::Null);
            }
            Ok(Value::from_number(-v.to_number()?))
        }
        UnOp::Not => {
            if v.is_null() {
                return Ok(Value::Null);
            }
            // `Not` on a Boolean gives a Boolean; on a number it flips the
            // bits. `Not 5` is `-6`, and that is not a bug.
            if let Value::Bool(b) = v {
                return Ok(Value::Bool(!b));
            }
            Ok(Value::Long(!to_i32(v)?))
        }
    }
}

/// Applies a binary operator.
pub fn binary(op: BinOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    use BinOp::*;
    match op {
        // `Is` is the one operator that does not look at values at all.
        Is => Ok(Value::Bool(is_same(lhs, rhs))),

        Concat => {
            // `&` is the only operator that keeps working on `Null`: it treats
            // it as an empty string, so `Null & "a"` is `"a"`. Every other one
            // propagates.
            let l = if lhs.is_null() {
                Rc::from("")
            } else {
                lhs.to_str()?
            };
            let r = if rhs.is_null() {
                Rc::from("")
            } else {
                rhs.to_str()?
            };
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(&l);
            s.push_str(&r);
            Ok(Value::str(s))
        }

        Add => {
            if lhs.is_null() || rhs.is_null() {
                return Ok(Value::Null);
            }
            // Two strings concatenate. Anything else adds — including a string
            // and a number, which is where a table's stray label turns into a
            // type mismatch.
            if matches!(lhs, Value::Str(_)) && matches!(rhs, Value::Str(_)) {
                return binary(Concat, lhs, rhs);
            }
            // `Empty + "a"` is `"a"`: an unassigned variable concatenates.
            if matches!(lhs, Value::Empty) && matches!(rhs, Value::Str(_))
                || matches!(rhs, Value::Empty) && matches!(lhs, Value::Str(_))
            {
                return binary(Concat, lhs, rhs);
            }
            arith(lhs, rhs, |a, b| Ok(a + b))
        }
        Sub => arith(lhs, rhs, |a, b| Ok(a - b)),
        Mul => arith(lhs, rhs, |a, b| Ok(a * b)),
        Div => arith(lhs, rhs, |a, b| {
            if b == 0.0 {
                // Not an infinity: VBScript raises.
                return Err(Error::division_by_zero());
            }
            Ok(a / b)
        }),
        Pow => arith(lhs, rhs, |a, b| Ok(a.powf(b))),

        IntDiv => integer_op(lhs, rhs, |a, b| {
            a.checked_div(b).ok_or_else(|| {
                if b == 0 {
                    Error::division_by_zero()
                } else {
                    // `i32::MIN \ -1`.
                    Error::overflow()
                }
            })
        }),
        Mod => integer_op(lhs, rhs, |a, b| {
            a.checked_rem(b).ok_or_else(|| {
                if b == 0 {
                    Error::division_by_zero()
                } else {
                    Error::overflow()
                }
            })
        }),

        Eq | Ne | Lt | Gt | Le | Ge => compare(op, lhs, rhs),

        And | Or | Xor | Eqv | Imp => logical(op, lhs, rhs),
    }
}

/// `Is`: reference identity.
///
/// `Nothing Is Nothing` is true. Two different host objects that happen to wrap
/// the same thing are the same object only if the host says so.
fn is_same(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Nothing, Value::Nothing) => true,
        (Value::Instance(a), Value::Instance(b)) => same_instance(a, b),
        (Value::Object(a), Value::Object(b)) => a.same_object(b),
        (Value::Proc(a), Value::Proc(b)) => Rc::ptr_eq(a, b),
        // Comparing a non-object with `Is` is a type mismatch in the real
        // engine, but tables write `If x Is Nothing` on variables that might
        // hold anything, and answering `False` is what they expect.
        _ => false,
    }
}

fn arith(lhs: &Value, rhs: &Value, f: impl Fn(f64, f64) -> Result<f64>) -> Result<Value> {
    if lhs.is_null() || rhs.is_null() {
        return Ok(Value::Null);
    }
    Ok(Value::from_number(f(lhs.to_number()?, rhs.to_number()?)?))
}

/// The operators that work on integers: `\` and `Mod`.
///
/// Both round their operands **before** dividing, which is why `7.5 \ 2` is
/// `4`. Doing the division first and truncating would give `3`.
fn integer_op(lhs: &Value, rhs: &Value, f: impl Fn(i32, i32) -> Result<i32>) -> Result<Value> {
    if lhs.is_null() || rhs.is_null() {
        return Ok(Value::Null);
    }
    Ok(Value::Long(f(to_i32(lhs)?, to_i32(rhs)?)?))
}

fn to_i32(v: &Value) -> Result<i32> {
    let n = round_half_to_even(v.to_number()?);
    if !n.is_finite() || n > f64::from(i32::MAX) || n < f64::from(i32::MIN) {
        return Err(Error::overflow());
    }
    Ok(n as i32)
}

/// Comparison, with the rule that decides everything else: **two strings
/// compare as strings, anything else compares as numbers**.
///
/// So `"10" = 10` is true — the string becomes a number — while `"10" = "9"` is
/// false, because as strings `"10"` sorts first. A table that stores a switch
/// number as a string and compares it against a number gets the answer it
/// wanted; one that compares two strings gets lexicographic order.
///
/// `Empty` is a special case in both directions: it equals `0` and it equals
/// `""`, which is how `If x = "" Then` works on a variable that was never
/// assigned.
fn compare(op: BinOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    if lhs.is_null() || rhs.is_null() {
        return Ok(Value::Null);
    }

    let ordering = match (lhs, rhs) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        // `Empty` compared with a string behaves as `""`.
        (Value::Empty, Value::Str(b)) => "".cmp(&**b),
        (Value::Str(a), Value::Empty) => (**a).cmp(""),
        _ => {
            let (a, b) = (lhs.to_number()?, rhs.to_number()?);
            // NaN cannot come from a script's arithmetic —division raises
            // rather than producing one— so this only guards host values.
            a.partial_cmp(&b).ok_or_else(Error::type_mismatch)?
        }
    };

    use std::cmp::Ordering::*;
    Ok(Value::Bool(match op {
        BinOp::Eq => ordering == Equal,
        BinOp::Ne => ordering != Equal,
        BinOp::Lt => ordering == Less,
        BinOp::Le => ordering != Greater,
        BinOp::Gt => ordering == Greater,
        BinOp::Ge => ordering != Less,
        _ => unreachable!("compare called with {op:?}"),
    }))
}

/// `And`, `Or`, `Xor`, `Eqv`, `Imp`.
///
/// Bitwise on integers, and on Booleans too — `True` being `-1` is what makes
/// the bitwise answer coincide with the logical one. Two Booleans give a
/// Boolean back so that `TypeName` stays honest.
///
/// `Null` does **not** simply propagate here. `False And Null` is `False`,
/// because whatever the unknown is, the answer is already decided; but
/// `True And Null` is `Null`. Same for `Or` the other way round. This is
/// three-valued logic, and tables that probe for optional features lean on it.
fn logical(op: BinOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    let both_bool = matches!(lhs, Value::Bool(_) | Value::Empty | Value::Null)
        && matches!(rhs, Value::Bool(_) | Value::Empty | Value::Null);

    if lhs.is_null() || rhs.is_null() {
        // The cases where the known operand settles it on its own.
        let known = if lhs.is_null() { rhs } else { lhs };
        if !known.is_null() {
            let k = known.to_bool().unwrap_or(false);
            match op {
                BinOp::And if !k => return Ok(Value::Bool(false)),
                BinOp::Or if k => return Ok(Value::Bool(true)),
                _ => {}
            }
        }
        return Ok(Value::Null);
    }

    let (a, b) = (to_i32(lhs)?, to_i32(rhs)?);
    let r = match op {
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        // Bitwise equivalence: the bits that agree.
        BinOp::Eqv => !(a ^ b),
        // Implication: `Not a Or b`.
        BinOp::Imp => !a | b,
        _ => unreachable!("logical called with {op:?}"),
    };

    if both_bool {
        return Ok(Value::Bool(r != 0));
    }
    Ok(Value::Long(r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinOp::*;

    fn num(v: Value) -> f64 {
        v.to_number().unwrap()
    }
    fn text(v: Value) -> String {
        v.to_str().unwrap().to_string()
    }
    fn truth(v: Value) -> bool {
        v.to_bool().unwrap()
    }
    fn b(op: BinOp, l: Value, r: Value) -> Value {
        binary(op, &l, &r).unwrap()
    }

    #[test]
    fn plus_adds_numbers_and_joins_strings() {
        assert_eq!(num(b(Add, Value::Long(1), Value::Long(2))), 3.0);
        assert_eq!(text(b(Add, Value::str("1"), Value::str("2"))), "12");
        // A number and a numeric string add.
        assert_eq!(num(b(Add, Value::Long(1), Value::str("2"))), 3.0);
        // A number and a word do not.
        assert!(binary(Add, &Value::Long(1), &Value::str("a")).is_err());
    }

    #[test]
    fn ampersand_always_joins() {
        assert_eq!(text(b(Concat, Value::Long(1), Value::Long(2))), "12");
        assert_eq!(text(b(Concat, Value::str("a"), Value::Long(1))), "a1");
        // And it is the one operator `Null` does not poison.
        assert_eq!(text(b(Concat, Value::Null, Value::str("a"))), "a");
    }

    #[test]
    fn empty_concatenates_with_a_string_and_adds_with_a_number() {
        assert_eq!(text(b(Add, Value::Empty, Value::str("a"))), "a");
        assert_eq!(num(b(Add, Value::Empty, Value::Long(5))), 5.0);
    }

    #[test]
    fn integer_division_rounds_its_operands_first() {
        // `7.5 \ 2` is 4 because 7.5 rounds to 8. Dividing first and
        // truncating would give 3.
        assert_eq!(num(b(IntDiv, Value::Double(7.5), Value::Long(2))), 4.0);
        assert_eq!(num(b(IntDiv, Value::Long(7), Value::Long(2))), 3.0);
        assert_eq!(num(b(IntDiv, Value::Long(-7), Value::Long(2))), -3.0);
    }

    #[test]
    fn mod_rounds_its_operands_too() {
        assert_eq!(num(b(Mod, Value::Long(7), Value::Long(3))), 1.0);
        // 7.6 rounds to 8, and 8 mod 3 is 2.
        assert_eq!(num(b(Mod, Value::Double(7.6), Value::Long(3))), 2.0);
        // The sign follows the left operand, as in C.
        assert_eq!(num(b(Mod, Value::Long(-7), Value::Long(3))), -1.0);
    }

    #[test]
    fn dividing_by_zero_raises_instead_of_giving_infinity() {
        assert_eq!(
            binary(Div, &Value::Long(1), &Value::Long(0))
                .unwrap_err()
                .number,
            11
        );
        assert_eq!(
            binary(IntDiv, &Value::Long(1), &Value::Long(0))
                .unwrap_err()
                .number,
            11
        );
        assert_eq!(
            binary(Mod, &Value::Long(1), &Value::Long(0))
                .unwrap_err()
                .number,
            11
        );
    }

    #[test]
    fn exponentiation_is_floating_point() {
        assert_eq!(num(b(Pow, Value::Long(2), Value::Long(10))), 1024.0);
        assert_eq!(num(b(Pow, Value::Long(2), Value::Double(0.5))), 2f64.sqrt());
    }

    #[test]
    fn a_numeric_string_compares_as_a_number() {
        assert!(truth(b(Eq, Value::str("10"), Value::Long(10))));
        assert!(truth(b(Gt, Value::str("10"), Value::Long(9))));
    }

    #[test]
    fn two_strings_compare_as_strings() {
        // Lexicographic, so "10" sorts before "9".
        assert!(truth(b(Lt, Value::str("10"), Value::str("9"))));
        assert!(truth(b(Eq, Value::str("abc"), Value::str("abc"))));
    }

    #[test]
    fn empty_equals_both_zero_and_the_empty_string() {
        assert!(truth(b(Eq, Value::Empty, Value::Long(0))));
        assert!(truth(b(Eq, Value::Empty, Value::str(""))));
        assert!(!truth(b(Eq, Value::Empty, Value::str("a"))));
    }

    #[test]
    fn true_is_minus_one_which_is_why_and_works() {
        assert_eq!(num(Value::Bool(true)), -1.0);
        assert!(truth(b(And, Value::Bool(true), Value::Bool(true))));
        assert!(!truth(b(And, Value::Bool(true), Value::Bool(false))));
        // And on numbers it really is bitwise.
        assert_eq!(num(b(And, Value::Long(5), Value::Long(3))), 1.0);
        assert_eq!(num(b(Or, Value::Long(5), Value::Long(2))), 7.0);
        assert_eq!(num(b(Xor, Value::Long(5), Value::Long(3))), 6.0);
    }

    #[test]
    fn two_booleans_stay_a_boolean() {
        // So `TypeName` does not start answering "Long" halfway through a
        // condition.
        assert_eq!(
            b(And, Value::Bool(true), Value::Bool(false)).type_name(),
            "Boolean"
        );
        assert_eq!(b(And, Value::Long(1), Value::Long(1)).type_name(), "Long");
    }

    #[test]
    fn not_flips_bits_on_a_number_and_the_value_on_a_boolean() {
        assert!(!truth(unary(UnOp::Not, &Value::Bool(true)).unwrap()));
        assert_eq!(num(unary(UnOp::Not, &Value::Long(5)).unwrap()), -6.0);
    }

    #[test]
    fn and_and_or_are_three_valued_around_null() {
        // The known operand can settle it even when the other is unknown.
        assert!(!truth(b(And, Value::Bool(false), Value::Null)));
        assert!(truth(b(Or, Value::Bool(true), Value::Null)));
        // And when it cannot, the answer is Null.
        assert!(b(And, Value::Bool(true), Value::Null).is_null());
        assert!(b(Or, Value::Bool(false), Value::Null).is_null());
    }

    #[test]
    fn null_poisons_arithmetic_and_comparison() {
        assert!(b(Add, Value::Long(1), Value::Null).is_null());
        assert!(b(Mul, Value::Long(2), Value::Null).is_null());
        assert!(b(Eq, Value::Long(1), Value::Null).is_null());
        assert!(unary(UnOp::Neg, &Value::Null).unwrap().is_null());
    }

    #[test]
    fn eqv_and_imp_are_bitwise() {
        // `Eqv` keeps the bits that agree; `Imp` is `Not a Or b`.
        assert_eq!(num(b(Eqv, Value::Long(5), Value::Long(5))), -1.0);
        assert_eq!(num(b(Imp, Value::Long(0), Value::Long(0))), -1.0);
        assert!(truth(b(Imp, Value::Bool(false), Value::Bool(false))));
        assert!(!truth(b(Imp, Value::Bool(true), Value::Bool(false))));
    }

    #[test]
    fn nothing_is_nothing() {
        assert!(truth(b(Is, Value::Nothing, Value::Nothing)));
        assert!(!truth(b(Is, Value::Nothing, Value::Long(0))));
    }

    #[test]
    fn a_result_that_is_a_whole_number_is_not_a_float() {
        assert_eq!(text(b(Mul, Value::Long(2), Value::Long(3))), "6");
        assert_eq!(text(b(Div, Value::Long(6), Value::Long(2))), "3");
        assert_eq!(text(b(Div, Value::Long(1), Value::Long(2))), "0.5");
    }
}
