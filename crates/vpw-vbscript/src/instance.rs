//! Variable storage, and instances of script classes.
//!
//! # Why a variable is a shared cell
//!
//! VBScript passes arguments **`ByRef` by default**. A sub that writes to its
//! parameter writes to the caller's variable, and tables rely on it — that is
//! how the standard scripts return several values at once. Modelling a variable
//! as a value in a map cannot express it; modelling it as a shared cell can,
//! and a `ByRef` binding is then just the same cell under a second name.
//!
//! `ByVal` copies into a fresh cell, which is exactly what the keyword means.
//!
//! # Why names are stored folded
//!
//! VBScript is case-insensitive everywhere. A table declaring `Sub SolFlipper`
//! and calling `solflipper` is correct, and real tables do it constantly.
//! Folding once when a name is stored is cheaper and less error-prone than
//! remembering to compare case-insensitively at every lookup.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::ClassDef;
use crate::value::Value;

/// One variable. Shared so `ByRef` can alias it.
pub type Slot = Rc<RefCell<Value>>;

pub fn slot(v: Value) -> Slot {
    Rc::new(RefCell::new(v))
}

/// A set of variables, looked up without regard to case.
#[derive(Debug, Default)]
pub struct Vars {
    map: HashMap<Box<str>, Slot>,
}

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&Slot> {
        self.map.get(fold(name).as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(fold(name).as_str())
    }

    /// Declares a variable, or leaves it alone if it is already there.
    ///
    /// Re-declaring is not an error in VBScript when it happens across separate
    /// `Dim`s in different procedures, and a table that includes the same
    /// helper script twice would otherwise fail to load.
    pub fn declare(&mut self, name: &str, value: Value) -> Slot {
        self.map
            .entry(fold(name).into_boxed_str())
            .or_insert_with(|| slot(value))
            .clone()
    }

    /// Declares a variable, replacing whatever was there.
    pub fn set_slot(&mut self, name: &str, s: Slot) {
        self.map.insert(fold(name).into_boxed_str(), s);
    }

    /// Assigns, declaring the variable if it does not exist.
    pub fn assign(&mut self, name: &str, value: Value) {
        match self.map.get(fold(name).as_str()) {
            Some(s) => *s.borrow_mut() = value,
            None => {
                self.map.insert(fold(name).into_boxed_str(), slot(value));
            }
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(|k| &**k)
    }
}

/// The canonical form of a name.
pub fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// A live instance of a `Class ... End Class`.
///
/// It is not a [`crate::object::Object`]: dispatching one of its methods needs
/// the interpreter, and the `Object` trait deliberately does not have one so
/// that a host can implement it without knowing anything about scripts. The
/// interpreter handles instances directly instead, which also keeps `Is`
/// honest — two instances are the same object when they are the same `Rc`.
pub struct Instance {
    pub def: Rc<ClassDef>,
    /// The member variables. `RefCell` because a method reaches them through a
    /// shared reference, and because a table will happily have an object's
    /// method reach back into the same object.
    pub fields: RefCell<Vars>,
}

impl Instance {
    pub fn new(def: Rc<ClassDef>) -> Self {
        Self {
            def,
            fields: RefCell::new(Vars::new()),
        }
    }

    /// The field's cell, or `None` if the class has no such field.
    pub fn field(&self, name: &str) -> Option<Slot> {
        self.fields.borrow().get(name).cloned()
    }
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}>", self.def.name)
    }
}

/// Whether two instances are literally the same object, for `Is`.
pub fn same_instance(a: &Rc<Instance>, b: &Rc<Instance>) -> bool {
    Rc::ptr_eq(a, b)
}

/// Convenience for reading a slot without holding the borrow.
pub fn read(s: &Slot) -> Value {
    s.borrow().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_case_insensitive() {
        let mut v = Vars::new();
        v.assign("SolFlipper", Value::Long(1));
        assert!(v.contains("solflipper"));
        assert!(v.contains("SOLFLIPPER"));
        assert!(matches!(read(v.get("SolFLIPPER").unwrap()), Value::Long(1)));
    }

    #[test]
    fn a_slot_can_be_shared_which_is_what_byref_is() {
        let mut caller = Vars::new();
        let s = caller.declare("x", Value::Long(1));

        let mut callee = Vars::new();
        callee.set_slot("param", s.clone());
        callee.assign("param", Value::Long(42));

        assert!(matches!(read(caller.get("x").unwrap()), Value::Long(42)));
    }

    #[test]
    fn byval_would_be_a_fresh_cell() {
        let mut caller = Vars::new();
        caller.declare("x", Value::Long(1));

        let mut callee = Vars::new();
        callee.set_slot("param", slot(read(caller.get("x").unwrap())));
        callee.assign("param", Value::Long(42));

        assert!(matches!(read(caller.get("x").unwrap()), Value::Long(1)));
    }

    #[test]
    fn declaring_twice_keeps_the_first_value() {
        // Including the same helper script twice must not wipe its state.
        let mut v = Vars::new();
        v.declare("x", Value::Long(1));
        v.declare("x", Value::Long(2));
        assert!(matches!(read(v.get("x").unwrap()), Value::Long(1)));
    }
}
