//! VBScript's standard library, minus everything that needs interpreter state.
//!
//! Every function here is pure: name in, arguments in, value out. The ones that
//! are missing are missing on purpose — `Rnd`, `Timer`, `Now`, `Eval`,
//! `CreateObject`, `MsgBox`, `Err`, `Erase` all need something the interpreter
//! owns (the RNG seed, the clock, the scopes, the host), so the interpreter
//! implements them and consults this module first.
//!
//! # What a table actually calls
//!
//! The set below is not "all of VBScript"; it is what the seventy `.vbs` files
//! that ship with Visual Pinball actually use. `Split`/`UBound` for parsing
//! configuration, `Int`/`Fix`/`Round` for lamp and score arithmetic, the `Cxxx`
//! conversions to force a subtype before handing a value to a COM property, and
//! a great deal of string mangling for ROM names and option keys.
//!
//! # Null
//!
//! `Null` is the part people get wrong, and it is worth stating the rule up
//! front because it is not one rule:
//!
//! * The single-argument numeric functions (`Abs`, `Sgn`, `Int`, `Fix`, `Sqr`,
//!   `Round`, `Hex`, `Oct`, and the trigonometric family) answer `Null`.
//! * Most string functions answer `Null` when their *string* argument is
//!   `Null`: `Len`, `Left`, `Right`, `Mid`, `InStr`, `InStrRev`, `LCase`,
//!   `UCase`, `Trim`, `LTrim`, `RTrim`, `StrComp`.
//! * `Replace`, `StrReverse`, `Split`, `Join` and `Filter` raise error 94
//!   instead. That is not an inconsistency we invented; MSDN documents "an
//!   error occurs" for exactly those.
//! * The explicit conversions (`CDbl`, `CStr`, `CBool`, …) raise error 94 too,
//!   which is also documented — a conversion is a demand, not a maybe.
//! * A *numeric* argument that is `Null` always raises, even on a function
//!   whose string argument would have propagated: `Left(Null, 2)` is `Null`,
//!   `Left("abc", Null)` is error 94.
//!
//! Getting this wrong is quiet. A table that does
//! `If Left(GetOption(...), 3) = "ROM" Then` wants the comparison to be false
//! when the option is missing, not to abort the script inside a `_Init` handler
//! and leave half the playfield unbuilt.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::value::{Array, Value, round_half_to_even};

/// Calls a builtin by name, case-insensitively.
///
/// Returns `None` if there is no builtin with that name, so the caller can
/// carry on looking for a user procedure or a host global.
pub fn call(name: &str, args: &[Value]) -> Option<Result<Value>> {
    let mut buf = [0u8; MAX_NAME];
    let key = fold(name, &mut buf)?;
    dispatch(key, name, args)
}

/// Whether a name is a builtin. Used to keep a script from shadowing one.
///
/// This deliberately covers the constants as well as the functions: a table
/// that manages to `Dim vbTab` has broken every later use of it just as
/// thoroughly as one that redefines `Left`. It is *not* a "can I call this"
/// predicate — [`call`] still answers `None` for a constant's name.
pub fn exists(name: &str) -> bool {
    let mut buf = [0u8; MAX_NAME];
    let Some(key) = fold(name, &mut buf) else {
        return false;
    };
    // Probing with an empty argument list is enough to answer the question:
    // `dispatch` decides whether the name exists before it looks at the
    // arguments, so a builtin that needs some answers `Some(Err(450))`. Doing
    // it this way keeps exactly one list of names in the file.
    dispatch(key, name, &[]).is_some() || constant_folded(key).is_some()
}

/// The named constants (`vbNewLine`, `vbCrLf`, `vbTab`, …).
///
/// Returns `None` if the name is not a known constant.
pub fn constant(name: &str) -> Option<Value> {
    let mut buf = [0u8; MAX_NAME];
    constant_folded(fold(name, &mut buf)?)
}

/// Longer than the longest name we answer to (`vbApplicationModal`, 18).
const MAX_NAME: usize = 24;

/// Lowercases a name into a stack buffer.
///
/// Builtin lookup happens on every call expression a script evaluates, and
/// allocating a `String` only to throw it away is a poor way to spend a frame's
/// budget. Anything too long or not ASCII cannot be one of our names, so it is
/// rejected here rather than compared.
fn fold<'a>(name: &str, buf: &'a mut [u8; MAX_NAME]) -> Option<&'a str> {
    let n = name.len();
    if n > MAX_NAME || !name.is_ascii() {
        return None;
    }
    buf[..n].copy_from_slice(name.as_bytes());
    buf[..n].make_ascii_lowercase();
    std::str::from_utf8(&buf[..n]).ok()
}

// ----------------------------------------------------------------- dispatch ---

/// The table. `key` is already lowercased; `name` is the spelling the script
/// used, and exists only so an arity error names it the way the author wrote it.
fn dispatch(key: &str, name: &str, args: &[Value]) -> Option<Result<Value>> {
    Some(match key {
        // -- maths --
        "abs" => unary_number(name, args, f64::abs),
        "sgn" => unary(name, args, |v| Ok(Value::Long(sgn(v.to_number()?)))),

        // `RGB(r, g, b)` packs a colour into a Long, and it packs it **blue
        // last**: the result is `r + g*256 + b*65536`. Tables use it for every
        // light and every flasher, and getting the byte order backwards turns
        // a red light blue without anything failing.
        "rgb" => {
            if args.len() != 3 {
                return Some(Err(Error::wrong_argument_count(name)));
            }
            // The real one clamps rather than raising: `RGB(300, 0, 0)` is
            // full red, not an error.
            let channel = |i: usize| args[i].to_int().map(|v| v.clamp(0, 255));
            channel(0).and_then(|r| {
                channel(1).and_then(|g| channel(2).map(|b| Value::Long(r + g * 256 + b * 65536)))
            })
        }
        "int" => unary_number(name, args, f64::floor),
        "fix" => unary_number(name, args, f64::trunc),
        "sqr" => unary(name, args, sqr),
        "sin" => unary_number(name, args, f64::sin),
        "cos" => unary_number(name, args, f64::cos),
        "tan" => unary_number(name, args, f64::tan),
        "atn" => unary_number(name, args, f64::atan),
        "exp" => unary(name, args, exp),
        "log" => unary(name, args, log),
        "round" => arity(name, args, 1, 2).and_then(|()| round(args)),
        "hex" => unary(name, args, |v| radix_string(v, 16)),
        "oct" => unary(name, args, |v| radix_string(v, 8)),

        // -- conversion --
        "cbool" => strict(name, args, |v| Ok(Value::Bool(v.to_bool()?))),
        "cbyte" => strict(name, args, |v| narrow(v, 0.0, 255.0)),
        // 16-bit, which is the whole reason a table writes `CInt` and not
        // `CLng`: it wants the overflow.
        "cint" => strict(name, args, |v| narrow(v, -32768.0, 32767.0)),
        "clng" => strict(name, args, |v| Ok(Value::Long(v.to_int()?))),
        "csng" => strict(name, args, csng),
        "cdbl" => strict(name, args, |v| Ok(Value::Double(v.to_number()?))),
        "cstr" => strict(name, args, |v| Ok(Value::Str(v.to_str()?))),

        // -- type tests --
        "isnumeric" => strict(name, args, |v| Ok(Value::Bool(is_numeric(v)))),
        "isempty" => strict(name, args, |v| Ok(Value::Bool(matches!(v, Value::Empty)))),
        "isnull" => strict(name, args, |v| Ok(Value::Bool(v.is_null()))),
        "isarray" => strict(name, args, |v| Ok(Value::Bool(v.is_array()))),
        "isobject" => strict(name, args, |v| Ok(Value::Bool(v.is_object()))),
        // There is no Date subtype here at all (see `value.rs`), so nothing is
        // a date. Tables call `IsDate` on entered text and fall back to a
        // default when it says no, which is the branch we want anyway.
        "isdate" => strict(name, args, |_| Ok(Value::Bool(false))),
        "typename" => strict(name, args, |v| Ok(Value::str(v.type_name()))),
        "vartype" => strict(name, args, |v| Ok(Value::Long(v.var_type()))),

        // -- which engine is this --
        //
        // `controller.vbs` opens `LoadVBSFiles` with
        // `If ScriptEngineMajorVersion < 5 Then MsgBox ...`, a guard against
        // engines that predate this millennium. It sits under
        // `On Error Resume Next`, so leaving it undefined does not stop the
        // script — it does something quieter and worse: it raises, `Err` stays
        // set, and the next line's `If Err Then MsgBox "Unable to open " &
        // VBSfile` reports a file that opened perfectly well.
        //
        // The numbers are VBScript 5.8, the last version there was.
        "scriptengine" => arity(name, args, 0, 0).map(|()| Value::str("VBScript")),
        "scriptenginemajorversion" => arity(name, args, 0, 0).map(|()| Value::Long(5)),
        "scriptengineminorversion" => arity(name, args, 0, 0).map(|()| Value::Long(8)),
        "scriptenginebuildversion" => arity(name, args, 0, 0).map(|()| Value::Long(16996)),

        // -- strings --
        "len" => unary(name, args, |v| Ok(Value::Long(chars(v)?.len() as i32))),
        "left" => arity(name, args, 2, 2).and_then(|()| left_right(args, true)),
        "right" => arity(name, args, 2, 2).and_then(|()| left_right(args, false)),
        "mid" => arity(name, args, 2, 3).and_then(|()| mid(args)),
        "instr" => arity(name, args, 2, 4).and_then(|()| instr(args)),
        "instrrev" => arity(name, args, 2, 4).and_then(|()| instr_rev(args)),
        "replace" => arity(name, args, 3, 6).and_then(|()| replace(args)),
        "split" => arity(name, args, 1, 4).and_then(|()| split(args)),
        "join" => arity(name, args, 1, 2).and_then(|()| join(args)),
        "trim" => trim(name, args, true, true),
        "ltrim" => trim(name, args, true, false),
        "rtrim" => trim(name, args, false, true),
        "lcase" => unary(name, args, |v| Ok(Value::str(v.to_str()?.to_lowercase()))),
        "ucase" => unary(name, args, |v| Ok(Value::str(v.to_str()?.to_uppercase()))),
        "chr" => strict(name, args, |v| chr(v, false)),
        "chrw" => strict(name, args, |v| chr(v, true)),
        "asc" | "ascw" => strict(name, args, asc),
        "space" => strict(name, args, |v| repeat(v, ' ')),
        "string" => arity(name, args, 2, 2).and_then(|()| string_fn(args)),
        "strcomp" => arity(name, args, 2, 3).and_then(|()| str_comp(args)),
        "strreverse" => strict(name, args, str_reverse),
        "filter" => arity(name, args, 2, 4).and_then(|()| filter(args)),

        // -- arrays --
        "array" => Ok(array_value(args.to_vec())),
        "ubound" => arity(name, args, 1, 2).and_then(|()| bound(args, false)),
        "lbound" => arity(name, args, 1, 2).and_then(|()| bound(args, true)),

        _ => return None,
    })
}

// ------------------------------------------------------------------ helpers ---

/// Checks the argument count, naming the function the way the script spelled it.
///
/// VBScript raises 450 for this and not 5, and the distinction shows up in
/// tables: 450 is also what a script sees when it calls a *host* method with
/// the wrong arity, so a diagnostic handler that special-cases it has to see
/// 450 from us as well.
fn arity(name: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min || args.len() > max {
        return Err(Error::wrong_argument_count(name));
    }
    Ok(())
}

/// A one-argument builtin that must see its argument, `Null` and all.
fn strict(name: &str, args: &[Value], f: impl FnOnce(&Value) -> Result<Value>) -> Result<Value> {
    arity(name, args, 1, 1)?;
    f(&args[0])
}

/// A one-argument builtin that answers `Null` when handed `Null`.
fn unary(name: &str, args: &[Value], f: impl FnOnce(&Value) -> Result<Value>) -> Result<Value> {
    strict(
        name,
        args,
        |v| {
            if v.is_null() { Ok(Value::Null) } else { f(v) }
        },
    )
}

/// A one-argument builtin that is a plain function of a number.
fn unary_number(name: &str, args: &[Value], f: impl FnOnce(f64) -> f64) -> Result<Value> {
    unary(name, args, |v| Ok(Value::from_number(f(v.to_number()?))))
}

fn sgn(n: f64) -> i32 {
    if n > 0.0 {
        1
    } else if n < 0.0 {
        -1
    } else {
        0
    }
}

/// The value as a vector of characters.
///
/// Everything string-shaped here indexes by character, because VBScript does.
/// Working on bytes would put `Mid` half way through an `é` the moment a table
/// has an accented ROM description, and `Len` would answer 2 for it.
///
/// Real VBScript counts UTF-16 code units, so a character outside the basic
/// multilingual plane counts as two there and one here. Nothing in a table has
/// ever contained one.
fn chars(v: &Value) -> Result<Vec<char>> {
    Ok(v.to_str()?.chars().collect())
}

fn from_chars(cs: &[char]) -> Value {
    Value::str(cs.iter().collect::<String>())
}

fn array_value(items: Vec<Value>) -> Value {
    Value::Array(Rc::new(RefCell::new(Array::from_values(items))))
}

/// `vbBinaryCompare` (0) or `vbTextCompare` (1); `true` means case-insensitive.
///
/// Shared by `InStr`, `InStrRev`, `Replace`, `Split`, `StrComp` and `Filter`,
/// which all take it in the same spelling and all default to binary. VBA's
/// `vbDatabaseCompare` (2) means nothing outside a Jet database, so it is an
/// invalid argument here exactly as it is in VBScript.
fn compare_mode(v: Option<&Value>) -> Result<bool> {
    match v {
        None => Ok(false),
        Some(v) => match v.to_int()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::invalid_call()),
        },
    }
}

fn char_eq(a: char, b: char, fold_case: bool) -> bool {
    a == b || (fold_case && a.to_lowercase().eq(b.to_lowercase()))
}

fn slice_eq(a: &[char], b: &[char], fold_case: bool) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| char_eq(x, y, fold_case))
}

/// First 0-based index at or after `from` where `needle` occurs.
fn find_from(hay: &[char], needle: &[char], from: usize, fold_case: bool) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len())
        .find(|&i| slice_eq(&hay[i..i + needle.len()], needle, fold_case))
}

/// Last 0-based index where a non-empty `needle` occurs entirely within the
/// first `limit` characters of `hay`.
fn find_last(hay: &[char], needle: &[char], limit: usize, fold_case: bool) -> Option<usize> {
    let hay = &hay[..limit.min(hay.len())];
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| slice_eq(&hay[i..i + needle.len()], needle, fold_case))
}

/// Caps the two functions that can be asked to build an arbitrarily long
/// string. VBScript would happily try; in a browser tab `Space(2000000000)`
/// takes the whole page with it, so it becomes error 7 instead. A table that
/// legitimately wants a 64MB string does not exist.
const MAX_REPEAT: i32 = 64 * 1024 * 1024;

// -------------------------------------------------------------------- maths ---

/// `Sqr`. Error 5 on a negative rather than NaN, so that the
/// `On Error Resume Next` a table wraps its geometry helpers in has something
/// to look at.
fn sqr(v: &Value) -> Result<Value> {
    let n = v.to_number()?;
    if n < 0.0 {
        return Err(Error::invalid_call());
    }
    Ok(Value::from_number(n.sqrt()))
}

/// `Exp`. `Exp(1000)` is error 6 in VBScript, not `+Inf` — there is no way to
/// spell infinity in a Variant, so overflow is the only honest answer.
fn exp(v: &Value) -> Result<Value> {
    let r = v.to_number()?.exp();
    if r.is_infinite() {
        return Err(Error::overflow());
    }
    Ok(Value::from_number(r))
}

/// `Log`, which is the **natural** logarithm and not the base-10 one. A table
/// that ports a curve from somewhere else and gets a response 2.3 times too
/// flat has found this. Zero and negatives are refused outright rather than
/// answering -Inf or NaN.
fn log(v: &Value) -> Result<Value> {
    let n = v.to_number()?;
    if n <= 0.0 {
        return Err(Error::invalid_call());
    }
    Ok(Value::from_number(n.ln()))
}

/// `Round(n)` and `Round(n, digits)`.
///
/// Two surprises live here. The first is that it rounds half to *even*, like
/// `CInt` — `Round(0.5)` is `0` and `Round(1.5)` is `2`. The second is subtler:
/// the number a table wrote as `2.675` is not 2.675, it is 2.67499999999999982,
/// and rounding that at face value gives `2.67` where every real VBScript says
/// `2.68`. So the scaled value is snapped back to fifteen significant digits —
/// the precision `format_double` prints at — before the rounding decision is
/// made. Without it, a table's displayed score and its stored score disagree in
/// the last digit, and only sometimes, which is the worst kind of bug to be
/// handed.
fn round(args: &[Value]) -> Result<Value> {
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let n = args[0].to_number()?;
    let digits = match args.get(1) {
        None => 0,
        Some(v) => v.to_int()?,
    };
    // VBScript has no "round to the nearest hundred": negative digits are
    // error 5, not a second and cleverer mode.
    if digits < 0 {
        return Err(Error::invalid_call());
    }
    if !n.is_finite() || digits > 15 {
        return Ok(Value::from_number(n));
    }
    let scale = 10f64.powi(digits);
    let scaled = n * scale;
    if !scaled.is_finite() {
        return Ok(Value::from_number(n));
    }
    Ok(Value::from_number(round_half_to_even(snap(scaled)) / scale))
}

/// Re-reads a float at fifteen significant digits, discarding the binary
/// representation error that scaling by a power of ten introduces.
fn snap(x: f64) -> f64 {
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// `Hex` and `Oct`, which answer **strings** and not numbers — `Hex(255) & "h"`
/// has to be `"FFh"`, and a table building a color string depends on it.
///
/// The width is the surprise. VBScript picks it from the value's subtype, so
/// `Hex(-1)` is `"FFFF"` and only `Hex(-1&)` is `"FFFFFFFF"`. We have no 16-bit
/// subtype (see `value.rs`), so the width comes from the value's magnitude
/// instead, which lands on the same answer for every literal a script can
/// write. It also keeps `Hex` and `&H` round-tripping: `value::parse_number`
/// reads four hex digits or fewer as signed 16-bit for exactly this reason.
fn radix_string(v: &Value, radix: u32) -> Result<Value> {
    // `Hex(Empty)` is "0" and not "", even though `CStr(Empty)` is "". Empty
    // is a zero on the way in, and the zero is then formatted.
    let n = v.to_int()?;
    let is_narrow = (i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&n);
    let bits: u32 = if is_narrow {
        u32::from(n as i16 as u16)
    } else {
        n as u32
    };
    Ok(Value::str(if radix == 16 {
        format!("{bits:X}")
    } else {
        format!("{bits:o}")
    }))
}

// --------------------------------------------------------------- conversion ---

/// `CInt` and `CByte`: round to even, then refuse to fit.
fn narrow(v: &Value, lo: f64, hi: f64) -> Result<Value> {
    let n = round_half_to_even(v.to_number()?);
    if !(lo..=hi).contains(&n) {
        return Err(Error::overflow());
    }
    Ok(Value::Long(n as i32))
}

/// `CSng`.
///
/// A deliberate departure. Real `CSng` produces a Single, and the visible
/// consequence is precision: `CStr(CSng(1/3))` is `"0.3333333"` in VBScript.
/// There is no Single subtype here, and actually rounding through `f32` would
/// give `"0.333333343267441"` — further from the truth *and* uglier than
/// leaving it alone. So `CSng` is `CDbl` with Single's range check, which is
/// the part a table can branch on: `CSng(1e39)` still overflows.
fn csng(v: &Value) -> Result<Value> {
    let n = v.to_number()?;
    if n.is_finite() && n.abs() > f64::from(f32::MAX) {
        return Err(Error::overflow());
    }
    Ok(Value::Double(n))
}

/// `IsNumeric`.
///
/// `IsNumeric(Empty)` is **False**, even though `CDbl(Empty)` is `0`. That is
/// not a slip in the docs, it is the point of the function: a table uses it to
/// tell "the player typed a number" from "the player typed nothing".
///
/// It never raises, however hopeless the argument. A predicate that can throw
/// is no use inside the `If` that was supposed to guard the conversion.
fn is_numeric(v: &Value) -> bool {
    match v {
        Value::Empty | Value::Null | Value::Array(_) => false,
        // An object with a numeric default value counts; one without does not,
        // and must not blow up on the way to saying so.
        _ => v.to_number().is_ok(),
    }
}

// ------------------------------------------------------------------ strings ---

/// `Left` and `Right`, which clamp rather than fail when asked for too much.
fn left_right(args: &[Value], from_left: bool) -> Result<Value> {
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let s = chars(&args[0])?;
    let n = args[1].to_int()?;
    if n < 0 {
        return Err(Error::invalid_call());
    }
    let n = (n as usize).min(s.len());
    Ok(from_chars(if from_left {
        &s[..n]
    } else {
        &s[s.len() - n..]
    }))
}

/// `Mid(s, start)` and `Mid(s, start, length)`.
///
/// One-based, like every string position in VBScript. `Mid(s, 1, 3)` is the
/// first three characters and `Mid(s, 0)` is error 5, not the whole string.
/// Porting this to a zero-based language is the classic way to shift every ROM
/// name a table parses by one character.
///
/// A `start` past the end is *not* an error, it is `""`. Tables lean on that:
/// walking a string with `Mid(s, i, 1)` and stopping when the result is empty
/// is a normal loop idiom, and making it raise would break every one of them.
fn mid(args: &[Value]) -> Result<Value> {
    if args[0].is_null() {
        return Ok(Value::Null);
    }
    let s = chars(&args[0])?;
    let start = args[1].to_int()?;
    if start < 1 {
        return Err(Error::invalid_call());
    }
    let begin = (start as usize - 1).min(s.len());
    let len = match args.get(2) {
        None => s.len() - begin,
        Some(v) => {
            let n = v.to_int()?;
            if n < 0 {
                return Err(Error::invalid_call());
            }
            (n as usize).min(s.len() - begin)
        }
    };
    Ok(from_chars(&s[begin..begin + len]))
}

/// `InStr(haystack, needle)` and `InStr(start, haystack, needle [, compare])`.
///
/// The two forms take their arguments in a *different order*, which is
/// VBScript's doing and not ours: `start` is prepended, not appended. Reading
/// `InStr(1, s, ",")` as "find 1 in s" is a real way to lose an afternoon.
///
/// One-based, and `0` means not found — so the idiomatic test is
/// `If InStr(s, x) > 0`, and an implementation that answered `-1` for "absent"
/// would make every such test true.
fn instr(args: &[Value]) -> Result<Value> {
    let (start, hay, needle, cmp) = if args.len() >= 3 {
        (
            args[0].to_int()?,
            &args[1],
            &args[2],
            compare_mode(args.get(3))?,
        )
    } else {
        (1, &args[0], &args[1], false)
    };
    // The 2-argument form has no `start` to get wrong, so this can only fire
    // on the longer one.
    if start < 1 {
        return Err(Error::invalid_call());
    }
    if hay.is_null() || needle.is_null() {
        return Ok(Value::Null);
    }
    let hay = chars(hay)?;
    let needle = chars(needle)?;
    let from = start as usize - 1;
    // An empty haystack answers 0 even when the needle is empty too, while an
    // empty needle otherwise answers `start`. Both are documented, and the
    // second is what stops a "find the next comma" loop from spinning.
    if hay.is_empty() || from >= hay.len() {
        return Ok(Value::Long(0));
    }
    Ok(Value::Long(match find_from(&hay, &needle, from, cmp) {
        Some(i) => i as i32 + 1,
        None => 0,
    }))
}

/// `InStrRev(haystack, needle [, start [, compare]])`.
///
/// This one takes `start` *after* the strings while `InStr` takes it before.
/// There is no reason for it; it is simply what shipped, and a table that gets
/// it backwards searches for the wrong thing without complaining.
///
/// `start` defaults to -1, meaning "the end". The position it names is the last
/// character a match may occupy, so the search is effectively over
/// `Left(haystack, start)`: `InStrRev("abcdef", "de", 4)` is 0, because "de"
/// does not fit inside "abcd". MSDN never says which end of the match `start`
/// bounds — this is the reading that makes its own worked examples come out
/// right, and it is what VB6 implements.
fn instr_rev(args: &[Value]) -> Result<Value> {
    let hay = &args[0];
    let needle = &args[1];
    let start = match args.get(2) {
        None => -1,
        Some(v) => v.to_int()?,
    };
    let cmp = compare_mode(args.get(3))?;
    // -1 is the only negative that means anything, and 0 is not a position.
    if start == 0 || start < -1 {
        return Err(Error::invalid_call());
    }
    if hay.is_null() || needle.is_null() {
        return Ok(Value::Null);
    }
    let hay = chars(hay)?;
    let needle = chars(needle)?;
    if hay.is_empty() {
        return Ok(Value::Long(0));
    }
    let limit = if start == -1 {
        hay.len()
    } else {
        start as usize
    };
    if limit > hay.len() {
        return Ok(Value::Long(0));
    }
    // An empty needle answers the starting position, mirroring `InStr`.
    if needle.is_empty() {
        return Ok(Value::Long(limit as i32));
    }
    Ok(Value::Long(match find_last(&hay, &needle, limit, cmp) {
        Some(i) => i as i32 + 1,
        None => 0,
    }))
}

/// `Replace(expression, find, replacewith [, start [, count [, compare]]])`.
///
/// The trap is `start`. It does not mean "leave the first `start - 1`
/// characters alone", it means the result *begins* there:
/// `Replace("abcabc", "a", "x", 4)` is `"xbc"` and not `"abcxbc"`. Tables that
/// pass `start` at all almost always meant the other thing, so when one mangles
/// a ROM name, look here first.
///
/// `count` defaults to -1, meaning all of them. `count = 0` is a copy, not an
/// empty string.
fn replace(args: &[Value]) -> Result<Value> {
    // Documented as an error rather than as `Null`, unlike `Left` and friends.
    if args[0].is_null() || args[1].is_null() || args[2].is_null() {
        return Err(Error::invalid_null());
    }
    let s = chars(&args[0])?;
    let find = chars(&args[1])?;
    let with = chars(&args[2])?;
    let start = match args.get(3) {
        None => 1,
        Some(v) => v.to_int()?,
    };
    let count = match args.get(4) {
        None => -1,
        Some(v) => v.to_int()?,
    };
    let cmp = compare_mode(args.get(5))?;
    if start < 1 || count < -1 {
        return Err(Error::invalid_call());
    }
    let begin = start as usize - 1;
    if begin >= s.len() {
        return Ok(Value::str(""));
    }
    let s = &s[begin..];
    // An empty `find` matches everywhere; VBScript hands back the tail
    // unchanged rather than interleaving the replacement forever.
    if find.is_empty() || count == 0 {
        return Ok(from_chars(s));
    }
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    let mut i = 0;
    let mut done = 0i32;
    while i < s.len() {
        if count >= 0 && done >= count {
            break;
        }
        let Some(j) = find_from(s, &find, i, cmp) else {
            break;
        };
        out.extend_from_slice(&s[i..j]);
        out.extend_from_slice(&with);
        i = j + find.len();
        done += 1;
    }
    out.extend_from_slice(&s[i..]);
    Ok(from_chars(&out))
}

/// `Split(expression [, delimiter [, count [, compare]]])`.
///
/// Three things worth knowing, all of which tables hit:
///
/// * The default delimiter is a single **space**, not a comma and not
///   whitespace. It does not collapse runs, so `Split("a  b")` has three
///   elements and the middle one is empty.
/// * `Split("")` returns an **empty array** — `UBound` of it is -1, not 0. This
///   is the documented behavior ("an array with no elements and no data") and
///   it is why the safe idiom is `If UBound(parts) >= 0 Then`. An array holding
///   one empty string would arguably be the better design, and several ports
///   assume it, but a table written against the real engine has already worked
///   around this and would break if we "fixed" it.
/// * `count` caps the number of elements, and the *last* one keeps the rest of
///   the string, delimiters and all: `Split("a b c", " ", 2)` is
///   `["a", "b c"]`.
fn split(args: &[Value]) -> Result<Value> {
    if args[0].is_null() {
        return Err(Error::invalid_null());
    }
    let s = chars(&args[0])?;
    let delim = match args.get(1) {
        None => vec![' '],
        Some(v) => chars(v)?,
    };
    let count = match args.get(2) {
        None => -1,
        Some(v) => v.to_int()?,
    };
    let cmp = compare_mode(args.get(3))?;
    if count < -1 {
        return Err(Error::invalid_call());
    }
    if s.is_empty() || count == 0 {
        return Ok(array_value(Vec::new()));
    }
    // A zero-length delimiter cannot separate anything, so the whole string
    // comes back as the single element.
    if delim.is_empty() {
        return Ok(array_value(vec![from_chars(&s)]));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while count < 0 || (out.len() as i32) < count - 1 {
        let Some(j) = find_from(&s, &delim, i, cmp) else {
            break;
        };
        out.push(from_chars(&s[i..j]));
        i = j + delim.len();
    }
    out.push(from_chars(&s[i.min(s.len())..]));
    Ok(array_value(out))
}

/// `Join(list [, delimiter])`.
///
/// The default delimiter is a space, matching `Split`, so `Join(Split(s))` is
/// `s` again — runs of spaces and all.
fn join(args: &[Value]) -> Result<Value> {
    let arr = args[0].to_array()?;
    let arr = arr.borrow();
    // A rectangular array has no reading order a script could have meant, so
    // VBScript refuses instead of picking one.
    if arr.dimensions() != 1 {
        return Err(Error::invalid_call());
    }
    let sep: Rc<str> = match args.get(1) {
        None => Rc::from(" "),
        Some(v) => v.to_str()?,
    };
    let mut out = String::new();
    for (i, item) in arr.items.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        // A `Null` element raises 94 here rather than becoming "", which is
        // what `&` would have done with it. `Join` is documented as an error,
        // and silently dropping a missing value out of a joined key is worse
        // than stopping.
        out.push_str(&item.to_str()?);
    }
    Ok(Value::str(out))
}

/// `Trim`, `LTrim` and `RTrim`.
///
/// They strip **spaces only**. Not tabs, not newlines. A config line read with
/// `Trim(line)` still has its trailing tab, exactly as under the real engine,
/// and the comparison against a ROM name then fails for no visible reason.
/// Resist the urge to reach for `str::trim` here: it would be more useful and
/// less correct.
fn trim(name: &str, args: &[Value], left: bool, right: bool) -> Result<Value> {
    unary(name, args, |v| {
        let s = v.to_str()?;
        let mut t: &str = &s;
        if left {
            t = t.trim_start_matches(' ');
        }
        if right {
            t = t.trim_end_matches(' ');
        }
        Ok(Value::str(t))
    })
}

/// `Chr` and `ChrW`.
///
/// `Chr` is the ANSI one: 0-255, interpreted through the system code page. We
/// have no code page, so those 256 values are read as Latin-1, which agrees
/// with Windows-1252 everywhere except 128-159. Tables write `Chr(10)`,
/// `Chr(13)`, `Chr(34)` and `Chr(65 + n)`, all of which are in the part that
/// cannot disagree.
///
/// Both accept a negative argument as its unsigned 16-bit twin, because
/// `&H8000` arrives as -32768 and a script that wrote `ChrW(&H8000)` still
/// expects a character back.
fn chr(v: &Value, wide: bool) -> Result<Value> {
    let n = v.to_int()?;
    let code = match n {
        -32768..=-1 => (n + 65536) as u32,
        0..=65535 => n as u32,
        _ => return Err(Error::invalid_call()),
    };
    if !wide && code > 255 {
        return Err(Error::invalid_call());
    }
    // A lone surrogate is a perfectly good VBScript string and not a Rust
    // `char`. Refusing is the only honest answer available to us.
    let c = char::from_u32(code).ok_or_else(Error::invalid_call)?;
    Ok(Value::str(c.to_string()))
}

/// `Asc` and `AscW`, which are the same function here.
///
/// Real `Asc` answers an ANSI byte and `AscW` a UTF-16 code unit; with no code
/// page to convert through, both answer the code point. They agree for
/// everything below 128, which is everything a table asks about.
///
/// An empty string is error 5 — deliberately not 0, so that `Asc(Mid(s, i, 1))`
/// past the end of `s` is caught rather than quietly read as a NUL.
fn asc(v: &Value) -> Result<Value> {
    let s = v.to_str()?;
    let c = s.chars().next().ok_or_else(Error::invalid_call)?;
    Ok(Value::Long(c as i32))
}

/// `StrReverse`. Reverses characters, not bytes.
fn str_reverse(v: &Value) -> Result<Value> {
    Ok(Value::str(v.to_str()?.chars().rev().collect::<String>()))
}

fn repeat(count: &Value, c: char) -> Result<Value> {
    let n = count.to_int()?;
    if n < 0 {
        return Err(Error::invalid_call());
    }
    if n > MAX_REPEAT {
        return Err(Error::out_of_memory());
    }
    Ok(Value::str(
        std::iter::repeat_n(c, n as usize).collect::<String>(),
    ))
}

/// `String(number, character)`.
///
/// The second argument is either a string, of which only the **first
/// character** is used, or a character code. `String(5, "ab")` is `"aaaaa"`,
/// which surprises anyone expecting `"ababababab"` — that is a loop, not
/// `String`.
fn string_fn(args: &[Value]) -> Result<Value> {
    let c = match &args[1] {
        Value::Str(s) => s.chars().next().ok_or_else(Error::invalid_call)?,
        Value::Null => return Err(Error::invalid_null()),
        v => match chr(v, false)? {
            Value::Str(s) => s.chars().next().ok_or_else(Error::invalid_call)?,
            _ => unreachable!("chr always answers a string"),
        },
    };
    repeat(&args[0], c)
}

/// `StrComp(string1, string2 [, compare])`, answering -1, 0 or 1.
///
/// Not a difference of code points. A table writing `If StrComp(a, b) = 0` is
/// safe either way, but one writing `If StrComp(a, b) = 1` — a normal thing to
/// write against a documented three-value function — is not.
fn str_comp(args: &[Value]) -> Result<Value> {
    if args[0].is_null() || args[1].is_null() {
        return Ok(Value::Null);
    }
    let cmp = compare_mode(args.get(2))?;
    let a = args[0].to_str()?;
    let b = args[1].to_str()?;
    let ord = if cmp {
        a.to_lowercase().cmp(&b.to_lowercase())
    } else {
        a.as_ref().cmp(b.as_ref())
    };
    Ok(Value::Long(match ord {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

/// `Filter(inputStrings, value [, include [, compare]])`.
///
/// Matches on **substring**, not equality — `Filter(a, "Ball")` keeps
/// `"BallSave"`. `include` defaults to `True`; passing `False` keeps everything
/// that does *not* match, which is how a script strips one prefix group out of
/// an options list.
///
/// The result is always a fresh zero-based array, and it is empty (`UBound` of
/// -1) when nothing matched — not `Empty`, and not an error.
fn filter(args: &[Value]) -> Result<Value> {
    // Documented as error 5 for a non-array, `Null` included, rather than the
    // type mismatch `to_array` would otherwise give.
    let arr = args[0].to_array().map_err(|_| Error::invalid_call())?;
    let arr = arr.borrow();
    if arr.dimensions() != 1 {
        return Err(Error::invalid_call());
    }
    if args[1].is_null() {
        return Err(Error::invalid_null());
    }
    let needle = chars(&args[1])?;
    let include = match args.get(2) {
        None => true,
        Some(v) => v.to_bool()?,
    };
    let cmp = compare_mode(args.get(3))?;
    let mut out = Vec::new();
    for item in &arr.items {
        let hay = chars(item)?;
        if find_from(&hay, &needle, 0, cmp).is_some() == include {
            out.push(from_chars(&hay));
        }
    }
    Ok(array_value(out))
}

// ------------------------------------------------------------------- arrays ---

/// `UBound(array [, dimension])` and `LBound`.
///
/// `LBound` is always 0 — VBScript has no `Option Base` — so the length of an
/// array is `UBound(a) + 1` and an empty array answers -1. That -1 is not a
/// failure code: `Split("")` and a `Filter` that matched nothing both produce
/// it, and `For i = 0 To UBound(a)` then correctly runs zero times.
///
/// The dimension argument is **1-based** even though the subscripts are not.
fn bound(args: &[Value], lower: bool) -> Result<Value> {
    let arr = args[0].to_array()?;
    let arr = arr.borrow();
    let dim = match args.get(1) {
        None => 1,
        Some(v) => v.to_int()?,
    };
    if dim < 1 || dim as usize > arr.dimensions() {
        return Err(Error::subscript_out_of_range());
    }
    Ok(Value::Long(if lower {
        0
    } else {
        arr.bounds[dim as usize - 1]
    }))
}

// ---------------------------------------------------------------- constants ---

/// The named constants, by folded name.
///
/// These are values and not functions, so they are looked up separately and the
/// interpreter substitutes them at name resolution. The numbers have to be the
/// real ones: a table passes `vbYesNo + vbQuestion` to `MsgBox` as a single
/// integer, and `vbObjectError` is the base it adds to when raising its own
/// errors with `Err.Raise`.
fn constant_folded(key: &str) -> Option<Value> {
    let s = |t: &str| Some(Value::str(t));
    let n = |v: i32| Some(Value::Long(v));
    match key {
        // -- characters --
        // On Windows `vbNewLine` *is* CrLf. Tables build strings with one and
        // split them on the other; a single character here would break that.
        "vbnewline" | "vbcrlf" => s("\r\n"),
        "vbcr" => s("\r"),
        "vblf" => s("\n"),
        "vbtab" => s("\t"),
        "vbback" => s("\u{8}"),
        "vbformfeed" => s("\u{c}"),
        "vbverticaltab" => s("\u{b}"),
        "vbnullchar" => s("\0"),
        // Not the same as `Empty`: a genuine zero-length string, so
        // `TypeName(vbNullString)` is "String".
        "vbnullstring" => s(""),

        // -- VarType --
        "vbempty" => n(0),
        "vbnull" => n(1),
        "vbinteger" => n(2),
        "vblong" => n(3),
        "vbsingle" => n(4),
        "vbdouble" => n(5),
        "vbcurrency" => n(6),
        "vbdate" => n(7),
        "vbstring" => n(8),
        "vbobject" => n(9),
        "vberror" => n(10),
        "vbboolean" => n(11),
        "vbvariant" => n(12),
        "vbdataobject" => n(13),
        "vbdecimal" => n(14),
        "vbbyte" => n(17),
        "vbarray" => n(8192),

        // -- tristate --
        // -1 and not 1, for the same reason `True` is -1.
        "vbtrue" => n(-1),
        "vbfalse" => n(0),
        "vbusedefault" => n(-2),

        // -- string comparison --
        "vbbinarycompare" => n(0),
        "vbtextcompare" => n(1),

        // -- MsgBox buttons, icons and modality --
        "vbokonly" | "vbdefaultbutton1" | "vbapplicationmodal" => n(0),
        "vbokcancel" => n(1),
        "vbabortretryignore" => n(2),
        "vbyesnocancel" => n(3),
        "vbyesno" => n(4),
        "vbretrycancel" => n(5),
        "vbcritical" => n(16),
        "vbquestion" => n(32),
        "vbexclamation" => n(48),
        "vbinformation" => n(64),
        "vbdefaultbutton2" => n(256),
        "vbdefaultbutton3" => n(512),
        "vbdefaultbutton4" => n(768),
        "vbsystemmodal" => n(4096),

        // -- MsgBox results --
        "vbok" => n(1),
        "vbcancel" => n(2),
        "vbabort" => n(3),
        "vbretry" => n(4),
        "vbignore" => n(5),
        "vbyes" => n(6),
        "vbno" => n(7),

        // &H80040000, the base for `Err.Raise vbObjectError + 1` — which is
        // how the standard scripts report a missing ROM.
        "vbobjecterror" => n(-2147221504),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls a builtin and unwraps everything, which is what most tests want.
    fn c(name: &str, args: &[Value]) -> Value {
        call(name, args)
            .unwrap_or_else(|| panic!("no builtin named {name}"))
            .unwrap_or_else(|e| panic!("{name} failed: {e}"))
    }

    /// The error number a builtin raises.
    fn err(name: &str, args: &[Value]) -> i32 {
        call(name, args)
            .unwrap_or_else(|| panic!("no builtin named {name}"))
            .expect_err("expected an error")
            .number
    }

    fn s(v: Value) -> String {
        v.to_str().unwrap().to_string()
    }

    fn i(v: Value) -> i32 {
        match v {
            Value::Long(n) => n,
            other => panic!("expected a Long, got {other:?}"),
        }
    }

    fn d(v: Value) -> f64 {
        v.to_number().unwrap()
    }

    fn b(v: Value) -> bool {
        v.to_bool().unwrap()
    }

    fn n(x: f64) -> Value {
        Value::Double(x)
    }

    fn items(v: &Value) -> Vec<String> {
        v.to_array()
            .unwrap()
            .borrow()
            .items
            .iter()
            .map(|x| x.to_str().unwrap().to_string())
            .collect()
    }

    fn ubound(v: &Value) -> i32 {
        i(c("UBound", std::slice::from_ref(v)))
    }

    // -- dispatch ---------------------------------------------------------

    #[test]
    fn names_are_matched_without_regard_to_case() {
        assert_eq!(s(c("ucase", &[Value::str("ab")])), "AB");
        assert_eq!(s(c("UCASE", &[Value::str("ab")])), "AB");
        assert_eq!(s(c("UcAsE", &[Value::str("ab")])), "AB");
    }

    #[test]
    fn an_unknown_name_is_not_a_builtin() {
        // The interpreter has to tell "no such builtin" from "that builtin
        // failed", or a table's own `Sub SolFlipper` would never be reached.
        assert!(call("RotateToEnd", &[]).is_none());
        assert!(call("Rnd", &[]).is_none(), "Rnd belongs to the interpreter");
        assert!(!exists("Timer"));
        assert!(exists("Left"));
        assert!(exists("lEfT"));
        assert!(exists("vbNewLine"), "constants must not be shadowable");
    }

    #[test]
    fn a_name_too_long_to_be_ours_is_rejected_cheaply() {
        assert!(!exists("ThisIsAVeryLongTableProcedureName"));
        assert!(call("ThisIsAVeryLongTableProcedureName", &[]).is_none());
    }

    #[test]
    fn the_wrong_number_of_arguments_is_error_450() {
        assert_eq!(err("Left", &[Value::str("abc")]), 450);
        assert_eq!(err("Abs", &[]), 450);
        assert_eq!(
            err(
                "Mid",
                &[
                    Value::str("a"),
                    Value::Long(1),
                    Value::Long(1),
                    Value::Long(1)
                ]
            ),
            450
        );
    }

    // -- maths ------------------------------------------------------------

    #[test]
    fn int_and_fix_disagree_on_negatives() {
        // The reason both exist: `Int` floors, `Fix` truncates toward zero.
        assert_eq!(i(c("Int", &[n(-2.5)])), -3);
        assert_eq!(i(c("Fix", &[n(-2.5)])), -2);
        assert_eq!(i(c("Int", &[n(2.5)])), 2);
        assert_eq!(i(c("Fix", &[n(2.5)])), 2);
        // The disagreement survives all the way down to a half-step below
        // zero, which is where a table computing a nudge offset lives.
        assert_eq!(i(c("Int", &[n(-0.5)])), -1);
        assert_eq!(i(c("Fix", &[n(-0.5)])), 0);
        // Neither of them rounds. `Int(2.9)` is 2, not 3.
        assert_eq!(i(c("Int", &[n(2.9)])), 2);
        assert_eq!(i(c("Fix", &[n(-2.9)])), -2);
    }

    #[test]
    fn int_of_a_whole_number_is_a_long_and_not_a_float() {
        assert!(matches!(c("Int", &[n(7.0)]), Value::Long(7)));
        assert_eq!(s(c("Int", &[n(7.0)])), "7");
    }

    #[test]
    fn abs_and_sgn_answer_the_obvious_things() {
        assert_eq!(i(c("Abs", &[Value::Long(-5)])), 5);
        assert_eq!(d(c("Abs", &[n(-2.5)])), 2.5);
        assert_eq!(i(c("Sgn", &[n(-0.001)])), -1);
        assert_eq!(i(c("Sgn", &[Value::Long(0)])), 0);
        assert_eq!(i(c("Sgn", &[Value::Long(9)])), 1);
        // A numeric string is a number, here as everywhere else.
        assert_eq!(i(c("Abs", &[Value::str("-3")])), 3);
        assert_eq!(err("Abs", &[Value::str("wizard")]), 13);
    }

    #[test]
    fn the_trig_family_works_in_radians() {
        assert!((d(c("Sin", &[n(std::f64::consts::FRAC_PI_2)])) - 1.0).abs() < 1e-12);
        assert_eq!(d(c("Cos", &[Value::Long(0)])), 1.0);
        assert!((d(c("Atn", &[Value::Long(1)])) - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        assert!(d(c("Tan", &[Value::Long(0)])).abs() < 1e-12);
    }

    #[test]
    fn sqr_and_log_refuse_their_undefined_arguments() {
        assert_eq!(d(c("Sqr", &[Value::Long(9)])), 3.0);
        assert_eq!(d(c("Sqr", &[Value::Long(0)])), 0.0);
        // Error 5 and not a NaN, so `On Error Resume Next` can see something.
        assert_eq!(err("Sqr", &[Value::Long(-1)]), 5);
        // Natural log, not base 10.
        assert_eq!(d(c("Log", &[Value::Long(1)])), 0.0);
        assert!((d(c("Log", &[n(std::f64::consts::E)])) - 1.0).abs() < 1e-12);
        assert_eq!(err("Log", &[Value::Long(0)]), 5);
        assert_eq!(err("Log", &[Value::Long(-1)]), 5);
    }

    #[test]
    fn exp_overflows_rather_than_answering_infinity() {
        assert!((d(c("Exp", &[Value::Long(0)])) - 1.0).abs() < 1e-15);
        assert_eq!(err("Exp", &[Value::Long(1000)]), 6);
    }

    #[test]
    fn round_goes_to_even_like_cint_does() {
        assert_eq!(i(c("Round", &[n(0.5)])), 0);
        assert_eq!(i(c("Round", &[n(1.5)])), 2);
        assert_eq!(i(c("Round", &[n(2.5)])), 2);
        assert_eq!(i(c("Round", &[n(-2.5)])), -2);
        assert_eq!(i(c("Round", &[n(3.5)])), 4);
        // Anything that is not an exact half rounds the schoolbook way.
        assert_eq!(i(c("Round", &[n(2.4)])), 2);
        assert_eq!(i(c("Round", &[n(2.6)])), 3);
    }

    #[test]
    fn round_with_digits_undoes_the_binary_representation_error() {
        // 2.675 is really 2.67499999999999982. Taken at face value this would
        // answer 2.67, and every real VBScript says 2.68.
        assert_eq!(s(c("Round", &[n(2.675), Value::Long(2)])), "2.68");
        assert_eq!(s(c("Round", &[n(1.23456), Value::Long(3)])), "1.235");
        // A genuine half still goes to even.
        assert_eq!(s(c("Round", &[n(0.125), Value::Long(2)])), "0.12");
        assert_eq!(s(c("Round", &[n(0.135), Value::Long(2)])), "0.14");
        assert_eq!(i(c("Round", &[n(2.0), Value::Long(4)])), 2);
    }

    #[test]
    fn round_refuses_negative_digits() {
        // VBScript has no "round to the nearest hundred".
        assert_eq!(err("Round", &[n(1234.0), Value::Long(-2)]), 5);
    }

    #[test]
    fn hex_and_oct_are_strings() {
        // Because `Hex(255) & "h"` has to work.
        assert!(matches!(c("Hex", &[Value::Long(255)]), Value::Str(_)));
        assert_eq!(s(c("Hex", &[Value::Long(255)])), "FF");
        assert_eq!(s(c("Hex", &[Value::Long(0)])), "0");
        assert_eq!(s(c("Hex", &[Value::Empty])), "0");
        assert_eq!(s(c("Oct", &[Value::Long(8)])), "10");
        assert_eq!(s(c("Oct", &[Value::Long(64)])), "100");
    }

    #[test]
    fn hex_of_a_negative_is_twos_complement_at_the_narrowest_width() {
        // Matches VBScript, where `Hex(-1)` is "FFFF" and only a Long -1 gives
        // eight digits. It also round-trips: `&HFFFF` reads back as -1.
        assert_eq!(s(c("Hex", &[Value::Long(-1)])), "FFFF");
        assert_eq!(s(c("Hex", &[Value::Long(-32768)])), "8000");
        assert_eq!(s(c("Hex", &[Value::Long(-32769)])), "FFFF7FFF");
        assert_eq!(s(c("Hex", &[Value::Long(65535)])), "FFFF");
    }

    #[test]
    fn hex_rounds_its_argument_first() {
        assert_eq!(s(c("Hex", &[n(15.6)])), "10");
        assert_eq!(s(c("Hex", &[n(16.5)])), "10", "and rounds to even");
    }

    // -- conversion -------------------------------------------------------

    #[test]
    fn cint_rounds_to_even_and_then_refuses_to_fit() {
        assert_eq!(i(c("CInt", &[n(0.5)])), 0);
        assert_eq!(i(c("CInt", &[n(1.5)])), 2);
        assert_eq!(i(c("CInt", &[n(2.5)])), 2);
        assert_eq!(i(c("CInt", &[n(-1.5)])), -2);
        assert_eq!(i(c("CInt", &[Value::str(" 42 ")])), 42);
        // 16-bit, which is why a table writes `CInt` when it wants the
        // overflow and `CLng` when it does not.
        assert_eq!(i(c("CInt", &[Value::Long(32767)])), 32767);
        assert_eq!(err("CInt", &[Value::Long(32768)]), 6);
        assert_eq!(err("CInt", &[Value::Long(-32769)]), 6);
        assert_eq!(i(c("CLng", &[Value::Long(32768)])), 32768);
    }

    #[test]
    fn cbyte_is_a_byte() {
        assert_eq!(i(c("CByte", &[n(255.0)])), 255);
        assert_eq!(i(c("CByte", &[n(0.5)])), 0);
        assert_eq!(i(c("CByte", &[n(1.5)])), 2);
        assert_eq!(err("CByte", &[Value::Long(256)]), 6);
        assert_eq!(err("CByte", &[Value::Long(-1)]), 6);
    }

    #[test]
    fn cbool_follows_the_rule_that_anything_nonzero_is_true() {
        assert!(b(c("CBool", &[Value::Long(5)])));
        assert!(!b(c("CBool", &[Value::Long(0)])));
        assert!(!b(c("CBool", &[Value::str("0")])));
        assert!(!b(c("CBool", &[Value::Empty])));
        assert_eq!(err("CBool", &[Value::str("yes")]), 13);
    }

    #[test]
    fn cstr_and_cdbl_do_what_the_operators_do() {
        assert_eq!(s(c("CStr", &[Value::Long(1000)])), "1000");
        assert_eq!(
            s(c("CStr", &[n(1000.0)])),
            "1000",
            "no trailing .0 on a score"
        );
        assert_eq!(s(c("CStr", &[Value::Bool(true)])), "True");
        assert_eq!(s(c("CStr", &[Value::Empty])), "");
        assert_eq!(d(c("CDbl", &[Value::str("2.5")])), 2.5);
        assert!(matches!(c("CDbl", &[Value::Long(2)]), Value::Double(_)));
    }

    #[test]
    fn csng_keeps_the_range_check_and_drops_the_precision_loss() {
        // A deliberate departure: there is no Single subtype here, so `CSng`
        // is `CDbl` that still overflows outside Single's range.
        assert_eq!(d(c("CSng", &[n(2.5)])), 2.5);
        assert_eq!(err("CSng", &[n(1e39)]), 6);
    }

    #[test]
    fn the_conversions_refuse_null_rather_than_propagating_it() {
        // Documented: a conversion is a demand, not a maybe.
        for f in ["CBool", "CByte", "CInt", "CLng", "CSng", "CDbl", "CStr"] {
            assert_eq!(err(f, &[Value::Null]), 94, "{f}(Null)");
        }
    }

    // -- type tests -------------------------------------------------------

    #[test]
    fn isnumeric_says_no_to_empty_even_though_cdbl_says_zero() {
        // The point of the function: "typed nothing" is not "typed 0".
        assert!(!b(c("IsNumeric", &[Value::Empty])));
        assert_eq!(d(c("CDbl", &[Value::Empty])), 0.0);

        assert!(b(c("IsNumeric", &[Value::Long(1)])));
        assert!(b(c("IsNumeric", &[Value::str(" 12 ")])));
        assert!(b(c("IsNumeric", &[Value::str("-2.5e3")])));
        assert!(b(c("IsNumeric", &[Value::str("&HFF")])));
        assert!(b(c("IsNumeric", &[Value::Bool(true)])));
        assert!(!b(c("IsNumeric", &[Value::str("1,000")])));
        assert!(!b(c("IsNumeric", &[Value::str("")])));
        // Never an error, however hopeless the argument.
        assert!(!b(c("IsNumeric", &[Value::Null])));
        assert!(!b(c("IsNumeric", &[array_value(vec![])])));
    }

    #[test]
    fn the_is_family_tells_empty_null_and_nothing_apart() {
        assert!(b(c("IsEmpty", &[Value::Empty])));
        assert!(!b(c("IsEmpty", &[Value::Null])));
        assert!(b(c("IsNull", &[Value::Null])));
        assert!(!b(c("IsNull", &[Value::Empty])));
        // `Nothing` is an object, which is what makes `IsObject(x)` true for
        // an object variable that was declared and never `Set`.
        assert!(b(c("IsObject", &[Value::Nothing])));
        assert!(!b(c("IsObject", &[Value::Empty])));
        assert!(b(c("IsArray", &[array_value(vec![])])));
        assert!(!b(c("IsArray", &[Value::Null])));
    }

    #[test]
    fn isdate_is_always_false_because_there_is_no_date_subtype() {
        assert!(!b(c("IsDate", &[Value::str("1/1/2000")])));
        assert!(!b(c("IsDate", &[Value::Long(1)])));
        assert!(!b(c("IsDate", &[Value::Null])));
    }

    #[test]
    fn typename_and_vartype_agree_with_each_other() {
        assert_eq!(s(c("TypeName", &[Value::Empty])), "Empty");
        assert_eq!(s(c("TypeName", &[Value::Null])), "Null");
        assert_eq!(s(c("TypeName", &[Value::str("x")])), "String");
        assert_eq!(s(c("TypeName", &[Value::Bool(false)])), "Boolean");
        assert_eq!(s(c("TypeName", &[array_value(vec![])])), "Variant()");

        assert_eq!(i(c("VarType", &[Value::Empty])), 0);
        assert_eq!(i(c("VarType", &[Value::Null])), 1);
        assert_eq!(i(c("VarType", &[n(1.5)])), 5);
        assert_eq!(i(c("VarType", &[Value::str("x")])), 8);
        assert_eq!(i(c("VarType", &[Value::Bool(true)])), 11);
        // vbArray + vbVariant, which is what a table tests with
        // `VarType(x) And vbArray`.
        assert_eq!(i(c("VarType", &[array_value(vec![])])), 8192 + 12);
    }

    // -- strings ----------------------------------------------------------

    #[test]
    fn len_counts_characters_and_survives_empty() {
        assert_eq!(i(c("Len", &[Value::str("abc")])), 3);
        assert_eq!(i(c("Len", &[Value::str("")])), 0);
        assert_eq!(i(c("Len", &[Value::Empty])), 0);
        assert_eq!(i(c("Len", &[Value::Long(1000)])), 4, "converts first");
        // A character and not a byte: this would be 2 if we counted UTF-8.
        assert_eq!(i(c("Len", &[Value::str("é")])), 1);
    }

    #[test]
    fn left_and_right_clamp_instead_of_failing() {
        assert_eq!(s(c("Left", &[Value::str("abcdef"), Value::Long(3)])), "abc");
        assert_eq!(
            s(c("Right", &[Value::str("abcdef"), Value::Long(3)])),
            "def"
        );
        assert_eq!(s(c("Left", &[Value::str("ab"), Value::Long(99)])), "ab");
        assert_eq!(s(c("Right", &[Value::str("ab"), Value::Long(99)])), "ab");
        assert_eq!(s(c("Left", &[Value::str("ab"), Value::Long(0)])), "");
        assert_eq!(s(c("Left", &[Value::str(""), Value::Long(3)])), "");
        // A negative length is the one thing they will not take.
        assert_eq!(err("Left", &[Value::str("ab"), Value::Long(-1)]), 5);
        assert_eq!(err("Right", &[Value::str("ab"), Value::Long(-1)]), 5);
    }

    #[test]
    fn mid_is_one_based() {
        // The single most common bug when porting VBScript to anything.
        assert_eq!(
            s(c(
                "Mid",
                &[Value::str("abcdef"), Value::Long(1), Value::Long(3)]
            )),
            "abc"
        );
        assert_eq!(s(c("Mid", &[Value::str("abcdef"), Value::Long(3)])), "cdef");
        assert_eq!(
            s(c(
                "Mid",
                &[Value::str("abcdef"), Value::Long(2), Value::Long(2)]
            )),
            "bc"
        );
        // Zero is not "from the start", it is an error.
        assert_eq!(err("Mid", &[Value::str("abc"), Value::Long(0)]), 5);
        assert_eq!(err("Mid", &[Value::str("abc"), Value::Long(-1)]), 5);
    }

    #[test]
    fn mid_past_the_end_is_empty_and_not_an_error() {
        // The `Mid(s, i, 1)` walking loop depends on this to terminate.
        assert_eq!(s(c("Mid", &[Value::str("abc"), Value::Long(4)])), "");
        assert_eq!(s(c("Mid", &[Value::str("abc"), Value::Long(99)])), "");
        assert_eq!(
            s(c(
                "Mid",
                &[Value::str("abc"), Value::Long(2), Value::Long(99)]
            )),
            "bc"
        );
        assert_eq!(
            s(c(
                "Mid",
                &[Value::str("abc"), Value::Long(2), Value::Long(0)]
            )),
            ""
        );
        assert_eq!(s(c("Mid", &[Value::str(""), Value::Long(1)])), "");
        assert_eq!(
            err("Mid", &[Value::str("abc"), Value::Long(1), Value::Long(-1)]),
            5
        );
    }

    #[test]
    fn instr_is_one_based_and_zero_means_not_found() {
        assert_eq!(i(c("InStr", &[Value::str("abcabc"), Value::str("b")])), 2);
        assert_eq!(i(c("InStr", &[Value::str("abc"), Value::str("z")])), 0);
        // The 3-argument form puts `start` first, which is the trap.
        assert_eq!(
            i(c(
                "InStr",
                &[Value::Long(3), Value::str("abcabc"), Value::str("b")]
            )),
            5
        );
        assert_eq!(
            i(c(
                "InStr",
                &[Value::Long(6), Value::str("abcabc"), Value::str("b")]
            )),
            0
        );
        assert_eq!(
            i(c(
                "InStr",
                &[Value::Long(7), Value::str("abcabc"), Value::str("b")]
            )),
            0
        );
    }

    #[test]
    fn instr_handles_the_empty_cases_the_documented_way() {
        // An empty needle answers `start`; an empty haystack answers 0 even
        // then.
        assert_eq!(i(c("InStr", &[Value::str("abc"), Value::str("")])), 1);
        assert_eq!(
            i(c(
                "InStr",
                &[Value::Long(2), Value::str("abc"), Value::str("")]
            )),
            2
        );
        assert_eq!(i(c("InStr", &[Value::str(""), Value::str("")])), 0);
        assert_eq!(i(c("InStr", &[Value::str(""), Value::str("a")])), 0);
    }

    #[test]
    fn instr_start_below_one_is_an_error() {
        assert_eq!(
            err(
                "InStr",
                &[Value::Long(0), Value::str("abc"), Value::str("b")]
            ),
            5
        );
        assert_eq!(
            err(
                "InStr",
                &[Value::Long(-1), Value::str("abc"), Value::str("b")]
            ),
            5
        );
    }

    #[test]
    fn compare_mode_one_means_case_insensitive() {
        let binary = [Value::Long(1), Value::str("ABC"), Value::str("b")];
        assert_eq!(i(c("InStr", &binary)), 0);
        let mut text = binary.to_vec();
        text.push(Value::Long(1));
        assert_eq!(i(c("InStr", &text)), 2);
        // vbDatabaseCompare means nothing outside Jet.
        assert_eq!(
            err(
                "InStr",
                &[
                    Value::Long(1),
                    Value::str("a"),
                    Value::str("a"),
                    Value::Long(2)
                ]
            ),
            5
        );
    }

    #[test]
    fn instrrev_takes_start_after_the_strings() {
        // Note the argument order: the opposite way round from `InStr`.
        let hay = Value::str("XXpXXpXXPXXP");
        assert_eq!(
            i(c(
                "InStrRev",
                &[
                    hay.clone(),
                    Value::str("P"),
                    Value::Long(10),
                    Value::Long(1)
                ]
            )),
            9
        );
        assert_eq!(
            i(c(
                "InStrRev",
                &[
                    hay.clone(),
                    Value::str("P"),
                    Value::Long(-1),
                    Value::Long(0)
                ]
            )),
            12
        );
        assert_eq!(i(c("InStrRev", &[hay, Value::str("P"), Value::Long(8)])), 0);
        assert_eq!(i(c("InStrRev", &[Value::str("a.b.c"), Value::str(".")])), 4);
        assert_eq!(i(c("InStrRev", &[Value::str("abc"), Value::str("z")])), 0);
        assert_eq!(i(c("InStrRev", &[Value::str(""), Value::str("a")])), 0);
    }

    #[test]
    fn instrrev_bounds_the_end_of_the_match_and_not_its_start() {
        // "de" starts at 4 but ends at 5, so it does not fit in Left(s, 4).
        assert_eq!(
            i(c(
                "InStrRev",
                &[Value::str("abcdef"), Value::str("de"), Value::Long(4)]
            )),
            0
        );
        assert_eq!(
            i(c(
                "InStrRev",
                &[Value::str("abcdef"), Value::str("de"), Value::Long(5)]
            )),
            4
        );
        // 0 is not a position, and -1 is the only negative with a meaning.
        assert_eq!(
            err(
                "InStrRev",
                &[Value::str("abc"), Value::str("a"), Value::Long(0)]
            ),
            5
        );
        assert_eq!(
            err(
                "InStrRev",
                &[Value::str("abc"), Value::str("a"), Value::Long(-2)]
            ),
            5
        );
        // A start past the end is 0, not the last occurrence.
        assert_eq!(
            i(c(
                "InStrRev",
                &[Value::str("abc"), Value::str("a"), Value::Long(9)]
            )),
            0
        );
    }

    #[test]
    fn replace_replaces_every_occurrence_by_default() {
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str("a-b-c"), Value::str("-"), Value::str("+")]
            )),
            "a+b+c"
        );
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str("aaa"), Value::str("aa"), Value::str("b")]
            )),
            "ba"
        );
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str("abc"), Value::str("z"), Value::str("q")]
            )),
            "abc"
        );
        // Removing is replacing with nothing.
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str("a b c"), Value::str(" "), Value::str("")]
            )),
            "abc"
        );
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str(""), Value::str("a"), Value::str("b")]
            )),
            ""
        );
        // An empty `find` matches everywhere, so VBScript declines to try.
        assert_eq!(
            s(c(
                "Replace",
                &[Value::str("abc"), Value::str(""), Value::str("x")]
            )),
            "abc"
        );
    }

    #[test]
    fn replace_start_truncates_the_result_rather_than_skipping() {
        // The trap. Not "abcxbc".
        assert_eq!(
            s(c(
                "Replace",
                &[
                    Value::str("abcabc"),
                    Value::str("a"),
                    Value::str("x"),
                    Value::Long(4)
                ]
            )),
            "xbc"
        );
        // Past the end is "", not the original.
        assert_eq!(
            s(c(
                "Replace",
                &[
                    Value::str("abc"),
                    Value::str("a"),
                    Value::str("x"),
                    Value::Long(9)
                ]
            )),
            ""
        );
        assert_eq!(
            err(
                "Replace",
                &[
                    Value::str("abc"),
                    Value::str("a"),
                    Value::str("x"),
                    Value::Long(0)
                ]
            ),
            5
        );
    }

    #[test]
    fn replace_count_caps_the_substitutions() {
        let with = |count: i32| {
            s(c(
                "Replace",
                &[
                    Value::str("aaaa"),
                    Value::str("a"),
                    Value::str("b"),
                    Value::Long(1),
                    Value::Long(count),
                ],
            ))
        };
        assert_eq!(with(-1), "bbbb");
        assert_eq!(with(2), "bbaa");
        // Zero is a copy, not an empty string.
        assert_eq!(with(0), "aaaa");
        assert_eq!(
            err(
                "Replace",
                &[
                    Value::str("aaaa"),
                    Value::str("a"),
                    Value::str("b"),
                    Value::Long(1),
                    Value::Long(-2)
                ]
            ),
            5
        );
    }

    #[test]
    fn replace_honors_the_compare_mode() {
        let with = |mode: i32| {
            s(c(
                "Replace",
                &[
                    Value::str("aAa"),
                    Value::str("a"),
                    Value::str("-"),
                    Value::Long(1),
                    Value::Long(-1),
                    Value::Long(mode),
                ],
            ))
        };
        assert_eq!(with(0), "-A-");
        assert_eq!(with(1), "---");
    }

    #[test]
    fn split_defaults_to_a_single_space_and_does_not_collapse_runs() {
        assert_eq!(items(&c("Split", &[Value::str("a b c")])), ["a", "b", "c"]);
        // Three elements, the middle one empty. Anyone expecting whitespace
        // splitting finds out here rather than three tables later.
        assert_eq!(items(&c("Split", &[Value::str("a  b")])), ["a", "", "b"]);
        assert_eq!(
            items(&c("Split", &[Value::str("a,b"), Value::str(",")])),
            ["a", "b"]
        );
        assert_eq!(
            items(&c("Split", &[Value::str("a,,b"), Value::str(",")])),
            ["a", "", "b"]
        );
        assert_eq!(
            items(&c("Split", &[Value::str(",a"), Value::str(",")])),
            ["", "a"]
        );
        assert_eq!(
            items(&c("Split", &[Value::str("a,"), Value::str(",")])),
            ["a", ""]
        );
        assert_eq!(
            items(&c("Split", &[Value::str("abc"), Value::str(",")])),
            ["abc"]
        );
    }

    #[test]
    fn splitting_an_empty_string_gives_an_empty_array() {
        // The documented behavior, and the reason the safe idiom is
        // `If UBound(parts) >= 0 Then`. Not an array holding one "".
        let v = c("Split", &[Value::str("")]);
        assert!(v.is_array());
        assert_eq!(ubound(&v), -1);
        assert!(items(&v).is_empty());
        // Same for `Empty`, which becomes "" on the way in.
        assert_eq!(ubound(&c("Split", &[Value::Empty])), -1);
    }

    #[test]
    fn split_with_an_empty_delimiter_returns_the_whole_string() {
        let v = c("Split", &[Value::str("abc"), Value::str("")]);
        assert_eq!(items(&v), ["abc"]);
        assert_eq!(ubound(&v), 0);
    }

    #[test]
    fn split_count_leaves_the_remainder_in_the_last_element() {
        let with = |count: i32| {
            items(&c(
                "Split",
                &[Value::str("a b c d"), Value::str(" "), Value::Long(count)],
            ))
        };
        assert_eq!(with(-1), ["a", "b", "c", "d"]);
        assert_eq!(with(2), ["a", "b c d"]);
        assert_eq!(with(1), ["a b c d"]);
        assert!(with(0).is_empty());
        assert_eq!(
            err(
                "Split",
                &[Value::str("a"), Value::str(" "), Value::Long(-2)]
            ),
            5
        );
    }

    #[test]
    fn split_and_join_are_inverses_at_the_default_delimiter() {
        let v = c("Split", &[Value::str("a b  c")]);
        assert_eq!(s(c("Join", &[v])), "a b  c");
        let arr = array_value(vec![Value::Long(1), Value::str("x"), Value::Empty]);
        assert_eq!(s(c("Join", &[arr, Value::str(",")])), "1,x,");
        assert_eq!(s(c("Join", &[array_value(vec![])])), "");
        assert_eq!(err("Join", &[Value::str("abc")]), 13);
    }

    #[test]
    fn trim_strips_spaces_and_leaves_tabs_alone() {
        // Not `str::trim`. A config line read with `Trim` keeps its trailing
        // tab exactly as it does under the real engine, and a table comparing
        // the result against a ROM name has to fail the same way ours does.
        assert_eq!(s(c("Trim", &[Value::str("  ab  ")])), "ab");
        assert_eq!(s(c("LTrim", &[Value::str("  ab  ")])), "ab  ");
        assert_eq!(s(c("RTrim", &[Value::str("  ab  ")])), "  ab");
        assert_eq!(s(c("Trim", &[Value::str("\tab\t")])), "\tab\t");
        assert_eq!(s(c("Trim", &[Value::str("\r\nab\r\n")])), "\r\nab\r\n");
        assert_eq!(s(c("Trim", &[Value::str("   ")])), "");
        assert_eq!(s(c("Trim", &[Value::Empty])), "");
    }

    #[test]
    fn case_conversion_leaves_non_letters_alone() {
        assert_eq!(s(c("UCase", &[Value::str("aB3-x")])), "AB3-X");
        assert_eq!(s(c("LCase", &[Value::str("aB3-X")])), "ab3-x");
        assert_eq!(s(c("UCase", &[Value::str("")])), "");
    }

    #[test]
    fn chr_and_asc_round_trip() {
        assert_eq!(s(c("Chr", &[Value::Long(65)])), "A");
        assert_eq!(s(c("Chr", &[Value::Long(13)])), "\r");
        assert_eq!(i(c("Asc", &[Value::str("A")])), 65);
        assert_eq!(i(c("Asc", &[Value::str("Abc")])), 65, "only the first one");
        assert_eq!(i(c("AscW", &[Value::str("€")])), 8364);
        assert_eq!(s(c("ChrW", &[Value::Long(8364)])), "€");
        // `Chr` is the 8-bit one; anything above 255 needs `ChrW`.
        assert_eq!(err("Chr", &[Value::Long(8364)]), 5);
        assert_eq!(err("Chr", &[Value::Long(70000)]), 5);
        // An empty string has no first character, and answering 0 would hide
        // a walk that ran off the end of the string.
        assert_eq!(err("Asc", &[Value::str("")]), 5);
    }

    #[test]
    fn space_and_string_build_padding() {
        assert_eq!(s(c("Space", &[Value::Long(3)])), "   ");
        assert_eq!(s(c("Space", &[Value::Long(0)])), "");
        assert_eq!(err("Space", &[Value::Long(-1)]), 5);
        assert_eq!(s(c("String", &[Value::Long(4), Value::str("x")])), "xxxx");
        // Only the first character of the pattern is used.
        assert_eq!(s(c("String", &[Value::Long(3), Value::str("ab")])), "aaa");
        assert_eq!(s(c("String", &[Value::Long(3), Value::Long(65)])), "AAA");
        assert_eq!(s(c("String", &[Value::Long(0), Value::str("x")])), "");
        assert_eq!(err("String", &[Value::Long(2), Value::str("")]), 5);
        // A browser tab would not survive the honest answer here.
        assert_eq!(err("Space", &[Value::Long(MAX_REPEAT + 1)]), 7);
    }

    #[test]
    fn strcomp_answers_minus_one_zero_or_one() {
        assert_eq!(i(c("StrComp", &[Value::str("a"), Value::str("b")])), -1);
        assert_eq!(i(c("StrComp", &[Value::str("b"), Value::str("a")])), 1);
        assert_eq!(i(c("StrComp", &[Value::str("a"), Value::str("a")])), 0);
        // Never a code-point difference, however tempting: only these three.
        assert_eq!(i(c("StrComp", &[Value::str("a"), Value::str("z")])), -1);
        assert_eq!(i(c("StrComp", &[Value::str("A"), Value::str("a")])), -1);
        assert_eq!(
            i(c(
                "StrComp",
                &[Value::str("A"), Value::str("a"), Value::Long(1)]
            )),
            0
        );
        assert_eq!(i(c("StrComp", &[Value::str("ab"), Value::str("a")])), 1);
        assert_eq!(
            err(
                "StrComp",
                &[Value::str("a"), Value::str("a"), Value::Long(9)]
            ),
            5
        );
    }

    #[test]
    fn strreverse_reverses_characters() {
        assert_eq!(s(c("StrReverse", &[Value::str("abc")])), "cba");
        assert_eq!(s(c("StrReverse", &[Value::str("")])), "");
        assert_eq!(s(c("StrReverse", &[Value::str("aé")])), "éa");
    }

    #[test]
    fn filter_matches_on_substring_and_is_case_sensitive_by_default() {
        let a = array_value(vec![
            Value::str("BallSave"),
            Value::str("Multiball"),
            Value::str("Tilt"),
        ]);
        // "Multiball" contains "ball" but not "Ball", and binary is the
        // default — exactly the sort of thing an options list gets wrong
        // without saying so.
        assert_eq!(
            items(&c("Filter", &[a.clone(), Value::str("Ball")])),
            ["BallSave"]
        );
        assert_eq!(
            items(&c(
                "Filter",
                &[
                    a.clone(),
                    Value::str("ball"),
                    Value::Bool(true),
                    Value::Long(1)
                ]
            )),
            ["BallSave", "Multiball"]
        );
        // include = False keeps everything that does not match.
        assert_eq!(
            items(&c(
                "Filter",
                &[a.clone(), Value::str("Ball"), Value::Bool(false)]
            )),
            ["Multiball", "Tilt"]
        );
        // Nothing matched is an empty array, not an error and not Empty.
        assert_eq!(ubound(&c("Filter", &[a, Value::str("zzz")])), -1);
        assert_eq!(
            err("Filter", &[Value::str("not an array"), Value::str("a")]),
            5
        );
    }

    // -- arrays -----------------------------------------------------------

    #[test]
    fn array_builds_a_zero_based_array() {
        let a = c(
            "Array",
            &[Value::Long(10), Value::Long(20), Value::Long(30)],
        );
        assert!(a.is_array());
        assert_eq!(i(c("LBound", std::slice::from_ref(&a))), 0);
        // Inclusive upper bound: three elements means UBound 2.
        assert_eq!(ubound(&a), 2);
        assert_eq!(items(&a), ["10", "20", "30"]);
    }

    #[test]
    fn ubound_of_an_empty_array_is_minus_one() {
        // Not an error and not 0. `For i = 0 To UBound(a)` then runs zero
        // times, which is the whole point of answering -1.
        let a = c("Array", &[]);
        assert_eq!(ubound(&a), -1);
        assert_eq!(i(c("LBound", &[a])), 0);
    }

    #[test]
    fn bounds_take_a_one_based_dimension() {
        let a = Value::Array(Rc::new(RefCell::new(Array::new(vec![2, 4]).unwrap())));
        assert_eq!(i(c("UBound", &[a.clone(), Value::Long(1)])), 2);
        assert_eq!(i(c("UBound", &[a.clone(), Value::Long(2)])), 4);
        assert_eq!(
            i(c("UBound", std::slice::from_ref(&a))),
            2,
            "the default is the first"
        );
        assert_eq!(i(c("LBound", &[a.clone(), Value::Long(2)])), 0);
        // Dimension 0 does not exist even though subscript 0 does.
        assert_eq!(err("UBound", &[a.clone(), Value::Long(0)]), 9);
        assert_eq!(err("UBound", &[a, Value::Long(3)]), 9);
        assert_eq!(err("UBound", &[Value::Long(5)]), 13);
    }

    // -- Null -------------------------------------------------------------

    #[test]
    fn the_numeric_functions_propagate_null() {
        for f in [
            "Abs", "Sgn", "Int", "Fix", "Sqr", "Sin", "Cos", "Tan", "Atn", "Exp", "Log", "Round",
            "Hex", "Oct",
        ] {
            assert!(
                c(f, &[Value::Null]).is_null(),
                "{f}(Null) should be Null, not an error"
            );
        }
        assert!(c("Round", &[Value::Null, Value::Long(2)]).is_null());
    }

    #[test]
    fn the_string_functions_propagate_null_where_vbscript_does() {
        // `If Left(GetOption(...), 3) = "ROM"` has to be false when the option
        // is missing, not abort the script inside a table's `_Init`.
        assert!(c("Len", &[Value::Null]).is_null());
        assert!(c("Left", &[Value::Null, Value::Long(2)]).is_null());
        assert!(c("Right", &[Value::Null, Value::Long(2)]).is_null());
        assert!(c("Mid", &[Value::Null, Value::Long(1)]).is_null());
        assert!(c("LCase", &[Value::Null]).is_null());
        assert!(c("UCase", &[Value::Null]).is_null());
        assert!(c("Trim", &[Value::Null]).is_null());
        assert!(c("LTrim", &[Value::Null]).is_null());
        assert!(c("RTrim", &[Value::Null]).is_null());
        assert!(c("InStr", &[Value::Null, Value::str("a")]).is_null());
        assert!(c("InStr", &[Value::str("a"), Value::Null]).is_null());
        assert!(c("InStr", &[Value::Long(1), Value::Null, Value::str("a")]).is_null());
        assert!(c("InStrRev", &[Value::Null, Value::str("a")]).is_null());
        assert!(c("StrComp", &[Value::Null, Value::str("a")]).is_null());
        assert!(c("StrComp", &[Value::str("a"), Value::Null]).is_null());
    }

    #[test]
    fn a_null_numeric_argument_raises_even_where_the_string_one_would_not() {
        // `Left(Null, 2)` is Null; `Left("abc", Null)` is error 94. The
        // asymmetry is VBScript's, not ours.
        assert_eq!(err("Left", &[Value::str("abc"), Value::Null]), 94);
        assert_eq!(err("Mid", &[Value::str("abc"), Value::Null]), 94);
        assert_eq!(err("Space", &[Value::Null]), 94);
        assert_eq!(err("Chr", &[Value::Null]), 94);
        assert_eq!(err("String", &[Value::Long(2), Value::Null]), 94);
    }

    #[test]
    fn the_functions_documented_as_erroring_on_null_do_error() {
        assert_eq!(
            err("Replace", &[Value::Null, Value::str("a"), Value::str("b")]),
            94
        );
        assert_eq!(
            err("Replace", &[Value::str("a"), Value::Null, Value::str("b")]),
            94
        );
        assert_eq!(err("StrReverse", &[Value::Null]), 94);
        assert_eq!(err("Split", &[Value::Null]), 94);
        assert_eq!(err("Filter", &[array_value(vec![]), Value::Null]), 94);
        assert_eq!(err("Join", &[array_value(vec![Value::Null])]), 94);
        // The type tests never propagate: they answer *about* the Null.
        assert!(b(c("IsNull", &[Value::Null])));
        assert_eq!(s(c("TypeName", &[Value::Null])), "Null");
        assert_eq!(i(c("VarType", &[Value::Null])), 1);
    }

    // -- constants --------------------------------------------------------

    #[test]
    fn the_character_constants_are_the_windows_ones() {
        // `vbNewLine` is two characters. A table that splits on `vbCrLf` text
        // it built with `vbNewLine` depends on them being identical.
        assert_eq!(s(constant("vbNewLine").unwrap()), "\r\n");
        assert_eq!(s(constant("vbCrLf").unwrap()), "\r\n");
        assert_eq!(s(constant("vbCr").unwrap()), "\r");
        assert_eq!(s(constant("vbLf").unwrap()), "\n");
        assert_eq!(s(constant("vbTab").unwrap()), "\t");
        assert_eq!(s(constant("vbBack").unwrap()), "\u{8}");
        assert_eq!(s(constant("vbFormFeed").unwrap()), "\u{c}");
        assert_eq!(s(constant("vbVerticalTab").unwrap()), "\u{b}");
        // A real zero-length string, not Empty.
        assert_eq!(s(constant("vbNullString").unwrap()), "");
        assert_eq!(constant("vbNullString").unwrap().type_name(), "String");
    }

    #[test]
    fn the_vartype_constants_match_what_vartype_answers() {
        for (name, v) in [
            ("vbEmpty", Value::Empty),
            ("vbNull", Value::Null),
            ("vbLong", Value::Long(1)),
            ("vbDouble", Value::Double(1.5)),
            ("vbString", Value::str("x")),
            ("vbBoolean", Value::Bool(true)),
        ] {
            assert_eq!(
                i(constant(name).unwrap()),
                v.var_type(),
                "{name} must agree with VarType"
            );
        }
        assert_eq!(i(constant("vbObject").unwrap()), 9);
        assert_eq!(i(constant("vbVariant").unwrap()), 12);
        assert_eq!(i(constant("vbArray").unwrap()), 8192);
    }

    #[test]
    fn the_msgbox_constants_add_up() {
        // A table writes `MsgBox s, vbYesNo + vbInformation` and the host sees
        // a single 68.
        assert_eq!(i(constant("vbOKOnly").unwrap()), 0);
        assert_eq!(i(constant("vbYesNo").unwrap()), 4);
        assert_eq!(i(constant("vbInformation").unwrap()), 64);
        assert_eq!(
            i(constant("vbYesNo").unwrap()) + i(constant("vbInformation").unwrap()),
            68
        );
        assert_eq!(i(constant("vbYes").unwrap()), 6);
        assert_eq!(i(constant("vbNo").unwrap()), 7);
        assert_eq!(i(constant("vbCritical").unwrap()), 16);
    }

    #[test]
    fn vbtrue_is_minus_one_and_vbobjecterror_is_the_com_base() {
        assert_eq!(i(constant("vbTrue").unwrap()), -1);
        assert_eq!(i(constant("vbFalse").unwrap()), 0);
        // &H80040000, the base a table adds to in `Err.Raise`.
        assert_eq!(i(constant("vbObjectError").unwrap()), -2147221504);
    }

    #[test]
    fn constants_are_case_insensitive_and_unknown_names_are_none() {
        assert!(constant("VBNEWLINE").is_some());
        assert!(constant("vbnewline").is_some());
        assert!(constant("vbNotAConstant").is_none());
        assert!(constant("Left").is_none(), "a function is not a constant");
        // A constant is not callable, which is how the interpreter tells the
        // two kinds of builtin apart.
        assert!(call("vbNewLine", &[]).is_none());
    }
}
