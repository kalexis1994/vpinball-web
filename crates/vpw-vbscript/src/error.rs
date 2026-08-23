//! Runtime errors, and the `Err` object tables read them through.
//!
//! Error handling is not a corner of VBScript that pinball tables avoid; it is
//! load-bearing. `On Error Resume Next` around a block and then a check of
//! `Err.Number` is the idiom the standard scripts use to probe for optional
//! features — whether a controller is installed, whether a table declared a
//! given object. There are four hundred references to `Err` in Visual Pinball's
//! own scripts.
//!
//! So the numbers matter. A table that checks `If Err.Number = 5 Then` has to
//! see a 5, which means the runtime errors carry the codes the real engine
//! raises rather than codes of our own.

use std::fmt;
use std::rc::Rc;

pub type Result<T> = std::result::Result<T, Error>;

/// What went wrong, in the shape `Err` exposes it.
#[derive(Debug, Clone)]
pub struct Error {
    /// The value `Err.Number` answers with.
    pub number: i32,
    /// The value `Err.Description` answers with.
    pub description: Rc<str>,
    /// The value `Err.Source` answers with.
    pub source: Rc<str>,
    /// Where it happened, when we know. Not part of the `Err` object; it is
    /// there so the host can log something a human can act on.
    pub line: Option<u32>,
    /// Set for the control-flow signals that are not really errors. They never
    /// reach a script's `Err`.
    pub(crate) control: Option<Control>,
}

/// Not an error: the interpreter's way of unwinding for `Exit` and friends.
///
/// They travel on the error channel because they have to unwind the same way an
/// error does — out of nested `If`s, out of `With`, out of a `For` body — and
/// giving them their own channel would mean every statement returned a
/// three-way result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::enum_variant_names,
    reason = "these are the `Exit` statements; naming them anything else would obscure that"
)]
pub(crate) enum Control {
    ExitSub,
    ExitFunction,
    ExitProperty,
    ExitFor,
    ExitDo,
}

impl Error {
    pub fn new(number: i32, description: impl AsRef<str>) -> Self {
        Self {
            number,
            description: Rc::from(description.as_ref()),
            source: Rc::from("Microsoft VBScript runtime error"),
            line: None,
            control: None,
        }
    }

    /// Attaches the line the error came from, if it does not have one yet.
    pub fn at_line(mut self, line: u32) -> Self {
        self.line.get_or_insert(line);
        self
    }

    pub(crate) fn control(c: Control) -> Self {
        Self {
            number: 0,
            description: Rc::from(""),
            source: Rc::from(""),
            line: None,
            control: Some(c),
        }
    }

    /// Whether this is a control-flow signal rather than a real error. Those
    /// must never be swallowed by `On Error Resume Next`.
    pub(crate) fn as_control(&self) -> Option<Control> {
        self.control
    }

    pub fn is_error(&self) -> bool {
        self.control.is_none()
    }

    // The codes the real engine raises. Kept as constructors so a call site
    // reads as what went wrong rather than as a number.

    /// 5 — the workhorse: a builtin got an argument it cannot use.
    pub fn invalid_call() -> Self {
        Self::new(5, "Invalid procedure call or argument")
    }
    /// 6
    pub fn overflow() -> Self {
        Self::new(6, "Overflow")
    }
    /// 7
    pub fn out_of_memory() -> Self {
        Self::new(7, "Out of memory")
    }
    /// 9 — a subscript outside the array, or the wrong number of subscripts.
    pub fn subscript_out_of_range() -> Self {
        Self::new(9, "Subscript out of range")
    }
    /// 11
    pub fn division_by_zero() -> Self {
        Self::new(11, "Division by zero")
    }
    /// 13 — the one a table hits when a label reaches arithmetic.
    pub fn type_mismatch() -> Self {
        Self::new(13, "Type mismatch")
    }
    /// 91 — using an object variable that was never `Set`.
    pub fn object_required() -> Self {
        Self::new(424, "Object required")
    }
    /// 94 — `Null` reached something that cannot take it.
    pub fn invalid_null() -> Self {
        Self::new(94, "Invalid use of Null")
    }
    /// 424
    pub fn object_variable_not_set() -> Self {
        Self::new(91, "Object variable not set")
    }
    /// 438 — the object exists but has no such member.
    pub fn no_such_member(name: &str) -> Self {
        Self::new(
            438,
            format!("Object doesn't support this property or method: '{name}'"),
        )
    }
    /// 500 — `Option Explicit` and an undeclared name.
    pub fn undefined_variable(name: &str) -> Self {
        Self::new(500, format!("Variable is undefined: '{name}'"))
    }
    /// 424, raised when a name is called that is not a procedure.
    pub fn undefined_procedure(name: &str) -> Self {
        Self::new(424, format!("Sub or Function not defined: '{name}'"))
    }
    /// 450 — the arity does not match.
    pub fn wrong_argument_count(name: &str) -> Self {
        Self::new(450, format!("Wrong number of arguments: '{name}'"))
    }
    /// 1002 — a parse error. Raised before anything runs.
    pub fn syntax(message: impl AsRef<str>, line: u32) -> Self {
        Self {
            number: 1002,
            description: Rc::from(message.as_ref()),
            source: Rc::from("Microsoft VBScript compilation error"),
            line: Some(line),
            control: None,
        }
    }
    /// What `Err.Raise` produces: a script raising an error of its own.
    pub fn raised(number: i32, source: Rc<str>, description: Rc<str>) -> Self {
        Self {
            number,
            description,
            source,
            line: None,
            control: None,
        }
    }

    /// 28 — the recursion guard. Not a VBScript feature: see
    /// [`crate::interp::Interpreter`].
    pub fn out_of_stack() -> Self {
        Self::new(28, "Out of stack space")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.control.is_some() {
            return write!(f, "internal control flow escaped the interpreter");
        }
        match self.line {
            Some(l) => write!(f, "line {l}: {} ({})", self.description, self.number),
            None => write!(f, "{} ({})", self.description, self.number),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_are_the_ones_tables_check_for() {
        // A table that says `If Err.Number = 13 Then` has to see a 13.
        assert_eq!(Error::type_mismatch().number, 13);
        assert_eq!(Error::subscript_out_of_range().number, 9);
        assert_eq!(Error::division_by_zero().number, 11);
        assert_eq!(Error::invalid_call().number, 5);
        assert_eq!(Error::overflow().number, 6);
    }

    #[test]
    fn control_flow_is_not_an_error() {
        assert!(!Error::control(Control::ExitSub).is_error());
        assert!(Error::type_mismatch().is_error());
    }

    #[test]
    fn the_first_line_attributed_wins() {
        // The innermost frame knows best; outer frames must not overwrite it.
        let e = Error::type_mismatch().at_line(7).at_line(99);
        assert_eq!(e.line, Some(7));
    }
}
