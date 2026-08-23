//! The Variant, and the conversions VBScript performs on it.
//!
//! This is the part of VBScript that surprises people, and it is the part a
//! table depends on without ever saying so. `"10" = 10` is true. `Empty` equals
//! both `0` and `""`. `Null` swallows nearly every operator it touches. `+`
//! adds two numbers, concatenates two strings and *adds* a number and a numeric
//! string. `\` and `Mod` round their operands to integers before working, and
//! they round to even. `True` is `-1`, which is why `And` can be both the
//! logical operator and the bitwise one.
//!
//! Getting any of that wrong does not produce an error, it produces a table
//! that scores differently, so the rules are written down here with the reason
//! next to them rather than spread through the interpreter.
//!
//! # What this Variant does not have
//!
//! Real VBScript distinguishes `Integer` (16-bit), `Long` (32-bit), `Single`,
//! `Double`, `Currency`, `Date`, `Byte` and `Decimal`. Here there are two
//! numeric types, [`Value::Long`] and [`Value::Double`], because that is what
//! arithmetic actually needs: everything narrower promotes on overflow anyway,
//! and no table branches on the difference. The one visible consequence is that
//! `TypeName(1)` answers `"Long"` where VBScript answers `"Integer"`; `VarType`
//! reports the same way. `Date` is missing entirely — `Now` and `Date()` are
//! not something a pinball table's rules can depend on.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::instance::Instance;
use crate::object::Object;

/// A VBScript value.
#[derive(Clone)]
pub enum Value {
    /// A variable that has been declared and never assigned.
    Empty,
    /// "No valid data". Not the same as [`Value::Empty`] and not the same as
    /// [`Value::Nothing`].
    Null,
    Bool(bool),
    Long(i32),
    Double(f64),
    Str(Rc<str>),
    /// A `Dim a(10)` array. Shared by reference, like a VBScript SafeArray
    /// handed around by `ByRef`.
    Array(Rc<RefCell<Array>>),
    /// Something the host exposes: a flipper, a timer, a controller.
    Object(Rc<dyn Object>),
    /// An instance of a `Class` the script itself declared.
    ///
    /// Kept apart from [`Value::Object`] rather than made to implement the
    /// same trait, because calling one of its methods needs the interpreter
    /// and the host trait deliberately has no access to one. See
    /// [`crate::instance::Instance`].
    Instance(Rc<Instance>),
    /// A procedure, as a value. What `GetRef("Foo")` returns.
    ///
    /// VBScript calls this a function pointer and reports it as an object;
    /// Visual Pinball's own scripts use it sixty-odd times to register
    /// callbacks, so it is not an exotic corner.
    Proc(Rc<crate::ast::Proc>),
    /// An object variable that points at nothing.
    Nothing,
}

impl Value {
    /// Convenience for the very common case of building a string value.
    pub fn str(s: impl AsRef<str>) -> Self {
        Value::Str(Rc::from(s.as_ref()))
    }

    /// The name `TypeName` answers with.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Empty => "Empty",
            Value::Null => "Null",
            Value::Bool(_) => "Boolean",
            Value::Long(_) => "Long",
            Value::Double(_) => "Double",
            Value::Str(_) => "String",
            Value::Array(_) => "Variant()",
            Value::Nothing => "Nothing",
            Value::Object(o) => o.type_name(),
            // VBScript answers with the class's own name, which is what a
            // table checks against.
            Value::Instance(_) => "Object",
            Value::Proc(_) => "Object",
        }
    }

    /// The number `VarType` answers with (`vbEmpty`, `vbNull`, …).
    pub fn var_type(&self) -> i32 {
        match self {
            Value::Empty => 0,
            Value::Null => 1,
            Value::Double(_) => 5,
            Value::Str(_) => 8,
            // `vbObject`. `Nothing` reports as an object too, which is what
            // lets `IsObject(Nothing)` be true.
            Value::Object(_) | Value::Instance(_) | Value::Proc(_) | Value::Nothing => 9,
            Value::Bool(_) => 11,
            Value::Long(_) => 3,
            // `vbArray + vbVariant`.
            Value::Array(_) => 8192 + 12,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(
            self,
            Value::Object(_) | Value::Instance(_) | Value::Proc(_) | Value::Nothing
        )
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// Whether this is a value operators should propagate rather than compute
    /// with. Only `Null` is; `Empty` takes part in arithmetic as a zero.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    // -- conversions ------------------------------------------------------

    /// `CDbl`, and what every arithmetic operator does to its operands.
    ///
    /// `Empty` is zero, which is why `Empty + 1` is `1` and not an error.
    /// A string has to look like a number: this is where "Type mismatch" comes
    /// from in a table that fed a label into an arithmetic expression.
    pub fn to_number(&self) -> Result<f64> {
        match self {
            Value::Empty => Ok(0.0),
            Value::Null => Err(Error::invalid_null()),
            // The whole point of `True = -1`.
            Value::Bool(b) => Ok(if *b { -1.0 } else { 0.0 }),
            Value::Long(n) => Ok(f64::from(*n)),
            Value::Double(d) => Ok(*d),
            Value::Str(s) => parse_number(s).ok_or_else(Error::type_mismatch),
            Value::Array(_) => Err(Error::type_mismatch()),
            Value::Nothing => Err(Error::object_required()),
            Value::Object(o) => o.default_value()?.to_number(),
            Value::Instance(_) | Value::Proc(_) => Err(Error::object_required()),
        }
    }

    /// `CLng`: the number rounded to an integer, **to even**.
    ///
    /// Banker's rounding is not a curiosity here. `CInt(0.5)` is `0` and
    /// `CInt(1.5)` is `2`, and a table that computes a lamp index by rounding
    /// lights a different lamp if this is done the schoolbook way.
    pub fn to_int(&self) -> Result<i32> {
        let n = self.to_number()?;
        let r = round_half_to_even(n);
        if !r.is_finite() || r > f64::from(i32::MAX) || r < f64::from(i32::MIN) {
            return Err(Error::overflow());
        }
        Ok(r as i32)
    }

    /// `CStr`, and what `&` does to its operands.
    pub fn to_str(&self) -> Result<Rc<str>> {
        Ok(match self {
            // Not "Empty": an unassigned variable concatenates as nothing.
            Value::Empty => Rc::from(""),
            Value::Null => return Err(Error::invalid_null()),
            Value::Bool(b) => Rc::from(if *b { "True" } else { "False" }),
            Value::Long(n) => Rc::from(n.to_string().as_str()),
            Value::Double(d) => Rc::from(format_double(*d).as_str()),
            Value::Str(s) => s.clone(),
            Value::Array(_) => return Err(Error::type_mismatch()),
            Value::Nothing => return Err(Error::object_required()),
            Value::Object(o) => o.default_value()?.to_str()?,
            Value::Instance(_) | Value::Proc(_) => return Err(Error::object_required()),
        })
    }

    /// `CBool`, and what `If` does to its condition.
    ///
    /// Anything non-zero is true, so `If 5 Then` runs. An empty string is
    /// false, but so is `"0"`; `"abc"` is a type mismatch, not `False`.
    pub fn to_bool(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Empty => Ok(false),
            Value::Null => Err(Error::invalid_null()),
            _ => Ok(self.to_number()? != 0.0),
        }
    }

    /// The object behind an object variable.
    pub fn to_object(&self) -> Result<Rc<dyn Object>> {
        match self {
            Value::Object(o) => Ok(o.clone()),
            _ => Err(Error::object_required()),
        }
    }

    /// The array behind an array variable.
    pub fn to_array(&self) -> Result<Rc<RefCell<Array>>> {
        match self {
            Value::Array(a) => Ok(a.clone()),
            _ => Err(Error::type_mismatch()),
        }
    }

    /// Narrows a float back to `Long` when it is exactly an integer.
    ///
    /// VBScript has no separate float type in an expression's result: `2 * 3`
    /// is `6`, not `6.0`, and `CStr` of it is `"6"`. Every arithmetic operator
    /// runs its result through here so that a table printing a score does not
    /// get `1000.0` on the display.
    pub fn from_number(n: f64) -> Value {
        if n.is_finite() && n.fract() == 0.0 && n.abs() <= f64::from(i32::MAX) {
            Value::Long(n as i32)
        } else {
            Value::Double(n)
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Empty => write!(f, "Empty"),
            Value::Null => write!(f, "Null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Long(n) => write!(f, "{n}"),
            Value::Double(d) => write!(f, "{d}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Array(a) => write!(f, "Array({:?})", a.borrow().bounds),
            Value::Nothing => write!(f, "Nothing"),
            Value::Object(o) => write!(f, "<{}>", o.type_name()),
            Value::Instance(i) => write!(f, "<{}>", i.def.name),
            Value::Proc(p) => write!(f, "<proc {}>", p.name),
        }
    }
}

/// A `Dim a(3, 4)` array.
///
/// VBScript arrays are **zero-based with an inclusive upper bound**: `Dim a(3)`
/// has four elements, 0 through 3. `bounds` stores the upper bounds, so it is
/// literally what `UBound` answers.
#[derive(Debug, Clone)]
pub struct Array {
    pub bounds: Vec<i32>,
    pub items: Vec<Value>,
}

impl Array {
    /// An array with the given upper bounds, every element `Empty`.
    pub fn new(bounds: Vec<i32>) -> Result<Self> {
        let len = Self::len_of(&bounds)?;
        Ok(Self {
            bounds,
            items: vec![Value::Empty; len],
        })
    }

    /// A one-dimensional array holding the given values, which is what the
    /// `Array(...)` builtin and `Split` produce.
    pub fn from_values(items: Vec<Value>) -> Self {
        Self {
            bounds: vec![items.len() as i32 - 1],
            items,
        }
    }

    pub fn dimensions(&self) -> usize {
        self.bounds.len()
    }

    fn len_of(bounds: &[i32]) -> Result<usize> {
        let mut len = 1usize;
        for &b in bounds {
            // `Dim a(-1)` is legal and gives an empty array; anything lower is
            // a subscript error.
            if b < -1 {
                return Err(Error::subscript_out_of_range());
            }
            len = len
                .checked_mul((b + 1) as usize)
                .ok_or_else(Error::out_of_memory)?;
        }
        Ok(len)
    }

    /// Turns subscripts into an index, row-major as VBScript stores them.
    pub fn offset(&self, subscripts: &[i32]) -> Result<usize> {
        if subscripts.len() != self.bounds.len() {
            return Err(Error::subscript_out_of_range());
        }
        let mut index = 0usize;
        for (i, &s) in subscripts.iter().enumerate() {
            if s < 0 || s > self.bounds[i] {
                return Err(Error::subscript_out_of_range());
            }
            index = index * (self.bounds[i] + 1) as usize + s as usize;
        }
        Ok(index)
    }

    pub fn get(&self, subscripts: &[i32]) -> Result<Value> {
        Ok(self.items[self.offset(subscripts)?].clone())
    }

    pub fn set(&mut self, subscripts: &[i32], v: Value) -> Result<()> {
        let i = self.offset(subscripts)?;
        self.items[i] = v;
        Ok(())
    }

    /// `ReDim Preserve`: resizes and keeps what fits.
    ///
    /// VBScript only lets you grow the **last** dimension when preserving, and
    /// for good reason: with any other dimension every element would have to
    /// move, and the elements that "stay" would silently change meaning. We
    /// enforce the same restriction instead of quietly doing something else.
    pub fn redim_preserve(&mut self, bounds: Vec<i32>) -> Result<()> {
        if bounds.len() != self.bounds.len() {
            return Err(Error::subscript_out_of_range());
        }
        if bounds[..bounds.len() - 1] != self.bounds[..self.bounds.len() - 1] {
            return Err(Error::subscript_out_of_range());
        }
        let mut fresh = Array::new(bounds)?;
        let keep = self.items.len().min(fresh.items.len());
        fresh.items[..keep].clone_from_slice(&self.items[..keep]);
        *self = fresh;
        Ok(())
    }
}

// ---------------------------------------------------------------- numbers ---

/// Rounds half to even, which is what `CInt`, `CLng` and `Round` all do.
pub fn round_half_to_even(n: f64) -> f64 {
    let rounded = n.round();
    // `f64::round` rounds half away from zero, so the only disagreement is on
    // exact halves.
    if (n - n.trunc()).abs() == 0.5 && rounded % 2.0 != 0.0 {
        rounded - n.signum()
    } else {
        rounded
    }
}

/// Parses the numeric literals a string can be coerced from.
///
/// VBScript accepts leading and trailing spaces, a sign, a decimal point,
/// exponents, and the `&H` / `&O` prefixes. It does **not** accept a trailing
/// `%`, thousands separators, or an empty string — `CDbl("")` is a type
/// mismatch, while `Empty` converts to zero.
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix("&") {
        let (radix, digits) = match rest.as_bytes().first()?.to_ascii_lowercase() {
            b'h' => (16, &rest[1..]),
            b'o' => (8, &rest[1..]),
            _ => return None,
        };
        // A trailing `&` marks it as a Long; it carries no value.
        let digits = digits.strip_suffix('&').unwrap_or(digits);
        // `&HFFFF` is -1: hex literals are read as signed 16- or 32-bit.
        let raw = u32::from_str_radix(digits, radix).ok()?;
        return Some(if digits.len() <= 4 {
            f64::from(raw as u16 as i16)
        } else {
            f64::from(raw as i32)
        });
    }
    t.parse::<f64>().ok()
}

/// Formats a number the way VBScript's `CStr` does.
///
/// The differences from Rust's default that matter: an integral float has no
/// `.0`, and a number between -1 and 1 keeps its leading `0`. VBScript actually
/// drops that zero (`CStr(0.5)` is `"0.5"` in VBScript but `".5"` in VB6's
/// `Str`); `CStr` keeps it, and `CStr` is what we are modelling.
pub fn format_double(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 {
            "1.#INF".into()
        } else {
            "-1.#INF".into()
        };
    }
    if d == 0.0 {
        // Avoids "-0".
        return "0".into();
    }
    if d.fract() == 0.0 && d.abs() < 1e15 {
        return format!("{d:.0}");
    }
    // VBScript prints up to fifteen significant digits and trims the rest,
    // which is what keeps 0.1 + 0.2 printing as "0.3".
    let mut s = format!("{d:.15e}");
    if let Some((mantissa, exponent)) = s.split_once('e') {
        let exp: i32 = exponent.parse().unwrap_or(0);
        if (-5..15).contains(&exp) {
            // Fifteen **significant** digits, so the count of decimals depends
            // on where the point is: one digit sits before it. Using `15 - exp`
            // gives sixteen and lets `1/3` print a digit VBScript does not.
            s = format!("{:.*}", (14 - exp).clamp(0, 17) as usize, d);
            let trimmed = s.trim_end_matches('0').trim_end_matches('.');
            return trimmed.to_string();
        }
        let m = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}E{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_is_minus_one() {
        // Not decoration: it is what makes `And` work as both the logical and
        // the bitwise operator.
        assert_eq!(Value::Bool(true).to_number().unwrap(), -1.0);
        assert_eq!(Value::Bool(false).to_number().unwrap(), 0.0);
    }

    #[test]
    fn empty_is_a_zero_and_an_empty_string() {
        assert_eq!(Value::Empty.to_number().unwrap(), 0.0);
        assert_eq!(&*Value::Empty.to_str().unwrap(), "");
        assert!(!Value::Empty.to_bool().unwrap());
    }

    #[test]
    fn null_refuses_every_conversion() {
        assert!(Value::Null.to_number().is_err());
        assert!(Value::Null.to_str().is_err());
        assert!(Value::Null.to_bool().is_err());
    }

    #[test]
    fn a_numeric_string_converts_and_a_word_does_not() {
        assert_eq!(Value::str(" 10 ").to_number().unwrap(), 10.0);
        assert_eq!(Value::str("-2.5").to_number().unwrap(), -2.5);
        assert_eq!(Value::str("1e3").to_number().unwrap(), 1000.0);
        assert!(Value::str("abc").to_number().is_err());
        assert!(Value::str("").to_number().is_err());
    }

    #[test]
    fn hex_and_octal_literals_are_signed() {
        // `&HFFFF` is -1 and not 65535: it is read as a signed 16-bit value.
        assert_eq!(parse_number("&HFFFF"), Some(-1.0));
        assert_eq!(parse_number("&H7FFF"), Some(32767.0));
        assert_eq!(parse_number("&HFFFFFFFF"), Some(-1.0));
        assert_eq!(parse_number("&H10"), Some(16.0));
        assert_eq!(parse_number("&O10"), Some(8.0));
        assert_eq!(parse_number("&H20&"), Some(32.0));
    }

    #[test]
    fn rounding_goes_to_even() {
        assert_eq!(round_half_to_even(0.5), 0.0);
        assert_eq!(round_half_to_even(1.5), 2.0);
        assert_eq!(round_half_to_even(2.5), 2.0);
        assert_eq!(round_half_to_even(-0.5), 0.0);
        assert_eq!(round_half_to_even(-1.5), -2.0);
        assert_eq!(round_half_to_even(-2.5), -2.0);
        // Anything that is not an exact half rounds normally.
        assert_eq!(round_half_to_even(2.6), 3.0);
        assert_eq!(round_half_to_even(-2.6), -3.0);
    }

    #[test]
    fn an_integral_result_is_not_a_float() {
        // A table that prints its score must not get "1000.0".
        assert_eq!(&*Value::from_number(1000.0).to_str().unwrap(), "1000");
        assert!(matches!(Value::from_number(2.5), Value::Double(_)));
    }

    #[test]
    fn numbers_print_the_way_vbscript_prints_them() {
        assert_eq!(format_double(6.0), "6");
        assert_eq!(format_double(0.5), "0.5");
        assert_eq!(format_double(-0.25), "-0.25");
        assert_eq!(format_double(0.0), "0");
        assert_eq!(format_double(0.1 + 0.2), "0.3");
        assert_eq!(format_double(1.0 / 3.0), "0.333333333333333");
    }

    #[test]
    fn arrays_have_an_inclusive_upper_bound() {
        // `Dim a(3)` has four elements, which is the single most common
        // off-by-one when porting VBScript to anything else.
        let a = Array::new(vec![3]).unwrap();
        assert_eq!(a.items.len(), 4);
        assert_eq!(a.bounds, vec![3]);
    }

    #[test]
    fn a_two_dimensional_array_is_row_major() {
        let a = Array::new(vec![1, 2]).unwrap();
        assert_eq!(a.items.len(), 2 * 3);
        assert_eq!(a.offset(&[0, 0]).unwrap(), 0);
        assert_eq!(a.offset(&[0, 2]).unwrap(), 2);
        assert_eq!(a.offset(&[1, 0]).unwrap(), 3);
        assert!(a.offset(&[2, 0]).is_err());
        assert!(a.offset(&[0]).is_err());
    }

    #[test]
    fn an_empty_array_is_legal() {
        let a = Array::new(vec![-1]).unwrap();
        assert!(a.items.is_empty());
        assert!(Array::new(vec![-2]).is_err());
    }

    #[test]
    fn redim_preserve_keeps_what_fits() {
        let mut a = Array::from_values(vec![Value::Long(1), Value::Long(2)]);
        a.redim_preserve(vec![3]).unwrap();
        assert_eq!(a.items.len(), 4);
        assert!(matches!(a.items[1], Value::Long(2)));
        assert!(matches!(a.items[3], Value::Empty));

        a.redim_preserve(vec![0]).unwrap();
        assert_eq!(a.items.len(), 1);
        assert!(matches!(a.items[0], Value::Long(1)));
    }

    #[test]
    fn redim_preserve_only_grows_the_last_dimension() {
        let mut a = Array::new(vec![1, 1]).unwrap();
        assert!(a.redim_preserve(vec![1, 4]).is_ok());
        assert!(
            a.redim_preserve(vec![4, 4]).is_err(),
            "growing an earlier dimension would move every element"
        );
    }
}
