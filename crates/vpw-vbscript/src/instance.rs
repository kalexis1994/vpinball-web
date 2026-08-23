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
    map: NameMap<Slot>,
}

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&Slot> {
        let mut buf = [0u8; MAX_NAME];
        match fold_into(name, &mut buf) {
            Some(key) => self.map.get(key),
            None => self.map.get(fold(name).as_str()),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        let mut buf = [0u8; MAX_NAME];
        match fold_into(name, &mut buf) {
            Some(key) => self.map.contains_key(key),
            None => self.map.contains_key(fold(name).as_str()),
        }
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

/// The hasher the name tables use.
///
/// FNV-1a, which is eight lines and no dependency. The maps it serves are all
/// keyed by an identifier out of a table's script — `nr`, `FadingLevel`,
/// `NFadeL` — and they are looked up more often than anything else in the
/// interpreter: nearly nine hundred thousand expressions are evaluated for
/// every five seconds of F-14, and most of them reach a variable by name.
///
/// The standard library's default is SipHash-1-3, which is chosen to make hash
/// collisions unforgeable by an attacker who controls the keys. That is the
/// right default for a map holding user input off a network and the wrong one
/// here: the keys are identifiers the table's author wrote, the map lives for
/// as long as the table is open, and the guarantee is being paid for on every
/// variable a script mentions. FNV has no such guarantee and is several times
/// faster on keys this short.
#[derive(Default, Clone, Copy)]
pub struct NameHasher(u64);

impl std::hash::Hasher for NameHasher {
    fn write(&mut self, bytes: &[u8]) {
        // 1469598103934665603 and 1099511628211 are FNV-1a's 64-bit offset
        // basis and prime.
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        self.0 = hash;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// [`NameHasher`], as a `HashMap` needs it.
pub type Names = std::hash::BuildHasherDefault<NameHasher>;

/// A map keyed by a script identifier.
pub type NameMap<V> = HashMap<Box<str>, V, Names>;

/// The longest name that folds on the stack.
///
/// Sixty-four bytes. VBScript's own limit for an identifier is 255, so this is
/// not a rule about the language; it is the point past which a name is rare
/// enough that an allocation does not matter. Nothing in `core.vbs` or in any
/// table script tried comes near it.
pub const MAX_NAME: usize = 64;

/// [`fold`], into a caller's buffer instead of onto the heap.
///
/// Returns `None` for a name that will not fit or is not ASCII, so the caller
/// can fall back to [`fold`] — never a truncated name, which would silently
/// become a *different* variable.
///
/// This exists because [`Vars::get`] is the single hottest thing in the
/// interpreter. Every variable a script mentions is looked up by name, and
/// looking it up means folding its case first; doing that with `fold` puts a
/// `String` on the heap and takes it off again for every `nr` and every
/// `FadingLevel` in a table's script. F-14 calls one two-line subroutine
/// ninety-four times every five milliseconds, so those are hundreds of
/// thousands of allocations a second, all of them for a key that is dead as
/// soon as the hash is computed.
pub fn fold_into<'a>(name: &str, buf: &'a mut [u8; MAX_NAME]) -> Option<&'a str> {
    let n = name.len();
    if n > MAX_NAME || !name.is_ascii() {
        return None;
    }
    buf[..n].copy_from_slice(name.as_bytes());
    buf[..n].make_ascii_lowercase();
    std::str::from_utf8(&buf[..n]).ok()
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
