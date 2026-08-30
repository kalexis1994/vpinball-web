//! How the script reaches things that are not the script.
//!
//! A pinball table's VBScript spends nearly all of its time talking to objects
//! it did not create: `LeftFlipper.RotateToEnd`, `Bumper1.TimerEnabled = True`,
//! `Table1.Nudge 90, 3`. Those objects live in the player, not in the
//! interpreter, and this trait is the whole of the boundary between them.
//!
//! It is deliberately small. Everything a script can do to an object is read a
//! property, write a property, or call a method, and VBScript does not really
//! distinguish the last two — `x.Foo` is a property read if `Foo` takes no
//! arguments and a call if it does, and the script cannot tell which the host
//! implemented.
//!
//! # Why `&self` and not `&mut self`
//!
//! Because a script routinely holds several references to the same object and
//! reaches it re-entrantly: a bumper's `_Hit` handler can set a property on the
//! very bumper that is dispatching the event. A `&mut self` interface would
//! deadlock a `RefCell` on the first table that does it, which is all of them.
//! Implementors keep their own interior mutability, which is also what the COM
//! objects being modelled do.

use std::rc::Rc;

use crate::error::{Error, Result};
use crate::value::Value;

/// Something a script can hold in a variable and send messages to.
pub trait Object {
    /// What `TypeName` answers for this object.
    fn type_name(&self) -> &'static str;

    /// Reads a property, or calls a method that takes no arguments.
    ///
    /// `args` is not empty when the script indexed the result — `x.Item(3)`, or
    /// a parameterised property.
    fn get(&self, name: &str, args: &[Value]) -> Result<Value> {
        let _ = args;
        Err(Error::no_such_member(name))
    }

    /// Writes a property. `Set x.Foo = y` arrives here too, with `by_ref` true.
    ///
    /// The distinction matters for exactly one reason: `x.Foo = obj` assigns
    /// `obj`'s *default value* while `Set x.Foo = obj` assigns the object
    /// itself, and a host that ignores the difference will store a string where
    /// the table meant a reference.
    fn set(&self, name: &str, args: &[Value], value: Value, by_ref: bool) -> Result<()> {
        let _ = (args, value, by_ref);
        Err(Error::no_such_member(name))
    }

    /// Calls a method.
    ///
    /// The default forwards to [`Object::get`], because for most hosts a method
    /// and a read-only property are the same function. Override it when a
    /// member has to behave differently depending on how it was reached.
    fn call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.get(name, args)
    }

    /// The value the object stands for when it lands in an expression.
    ///
    /// In COM this is the `DISPID_VALUE` member. Most objects do not have one,
    /// and using such an object as a number or a string is an error — which is
    /// the right answer, and is what the default does.
    fn default_value(&self) -> Result<Value> {
        Err(Error::object_required())
    }

    /// The members `For Each` should walk, if this object is a collection.
    fn enumerate(&self) -> Option<Vec<Value>> {
        None
    }

    /// Identity, for the `Is` operator.
    ///
    /// `Is` compares references and not contents, so this has to answer
    /// whether the two are literally the same object. The default uses the
    /// vtable-and-address pair of the `Rc`, which is right for every
    /// implementor that does not proxy something else.
    fn same_object(&self, other: &Rc<dyn Object>) -> bool {
        std::ptr::eq(
            self as *const Self as *const (),
            Rc::as_ptr(other) as *const (),
        )
    }
}

/// Where `CreateObject` and undeclared global names are resolved.
///
/// The interpreter knows nothing about flippers. When a script says
/// `LeftFlipper.RotateToEnd`, the name is not in any script scope, so the
/// interpreter asks the host — this is that question. The player answers it
/// with the table's game items; a test answers it with a stub.
pub trait Host {
    /// A global object by name, or `None` if the host does not know it.
    ///
    /// Called for names the script never declared, and the answer is cached for
    /// the run: a table's objects do not appear and disappear.
    fn global(&self, name: &str) -> Option<Value>;

    /// `CreateObject("VPinMAME.Controller")` and friends.
    fn create_object(&self, prog_id: &str) -> Result<Value> {
        Err(Error::new(
            429,
            format!("ActiveX component can't create object: '{prog_id}'"),
        ))
    }

    /// Where `MsgBox` and `debug.print` go.
    ///
    /// Tables use `MsgBox` for real diagnostics — a missing ROM, an option set
    /// wrong — so it has to go somewhere a person will see, not to a dialog
    /// nobody can dismiss in a browser and not to nowhere.
    fn message(&self, text: &str) {
        let _ = text;
    }

    /// Seconds since midnight, which is what `Timer` answers with.
    ///
    /// It is the host's job because there is no clock this crate can reach:
    /// in a browser the time comes from `performance.now()` through the
    /// player, and a test wants a clock it controls. Zero is a fine answer for
    /// a host that has none — a table uses `Timer` to measure intervals, and a
    /// clock that never moves reads as "no time has passed".
    fn seconds(&self) -> f64 {
        0.0
    }

    /// The wall clock, in milliseconds since the Unix epoch, if the host has
    /// one.
    ///
    /// Kept apart from [`Self::seconds`], which is the *table's* clock: that
    /// one starts at zero when the ball does and is what a script measures
    /// intervals against. This is the one on the wall, and the only thing that
    /// wants it is a table drawing a clock face.
    ///
    /// `None` from a host with no clock, and the date functions then answer
    /// from the day VBScript counts from. A frozen clock is a clock whose
    /// hands do not move; no clock at all used to be an error every tick.
    fn now_millis(&self) -> Option<f64> {
        None
    }
}

/// A host that knows nothing. Useful for testing the language on its own.
pub struct NoHost;

impl Host for NoHost {
    fn global(&self, _name: &str) -> Option<Value> {
        None
    }
}
