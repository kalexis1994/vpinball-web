//! The objects a VBScript host is expected to have, beyond the language.
//!
//! `Scripting.Dictionary` is not part of VBScript — it comes from the Windows
//! Script Host — but every VBScript that has ever needed a map uses it, and
//! Visual Pinball's `core.vbs` will not load without one. So it is here, and a
//! host can hand it out from [`crate::object::Host::create_object`].

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::object::Object;
use crate::value::{Array, Value};

/// `CreateObject("Scripting.Dictionary")`.
///
/// # Why a list and not a hash map
///
/// Because the keys are Variants, and hashing a Variant correctly means
/// deciding whether `1`, `"1"` and `True` are the same key — questions with
/// fiddly answers that only matter if the map is big. Real dictionaries in a
/// pinball table hold tens of entries: `core.vbs` keeps one of the classes
/// whose timers are due, and a table keeps one per mode. A linear scan over
/// twenty entries is faster than hashing them and impossible to get subtly
/// wrong.
///
/// If a table ever turns up with a thousand-entry dictionary this is the place
/// to change, and the behaviour will not move when it does.
pub struct Dictionary {
    entries: RefCell<Vec<(Value, Value)>>,
    /// 0 binary, 1 text. Decides whether string keys are case-sensitive.
    compare_mode: RefCell<i32>,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
            compare_mode: RefCell::new(0),
        }
    }

    fn position(&self, key: &Value) -> Option<usize> {
        let text = *self.compare_mode.borrow() != 0;
        self.entries
            .borrow()
            .iter()
            .position(|(k, _)| same_key(k, key, text))
    }

    fn key_arg<'a>(&self, args: &'a [Value]) -> Result<&'a Value> {
        args.first().ok_or_else(Error::invalid_call)
    }
}

impl Object for Dictionary {
    fn type_name(&self) -> &'static str {
        "Dictionary"
    }

    fn get(&self, name: &str, args: &[Value]) -> Result<Value> {
        match &*name.to_ascii_lowercase() {
            "count" => Ok(Value::Long(self.entries.borrow().len() as i32)),

            // Reading a key that is not there **creates it**, empty. That is a
            // real quirk of the original and not a mistake: `If d(k) = 0 Then`
            // leaves `k` in the dictionary behind it, which is why a table's
            // `Count` can grow just from being looked at.
            "" | "item" => {
                let key = self.key_arg(args)?;
                match self.position(key) {
                    Some(i) => Ok(self.entries.borrow()[i].1.clone()),
                    None => {
                        self.entries.borrow_mut().push((key.clone(), Value::Empty));
                        Ok(Value::Empty)
                    }
                }
            }

            "exists" => Ok(Value::Bool(self.position(self.key_arg(args)?).is_some())),

            "add" => {
                let key = self.key_arg(args)?.clone();
                let value = args.get(1).cloned().unwrap_or(Value::Empty);
                if self.position(&key).is_some() {
                    // The real one raises rather than replacing, and tables
                    // rely on it: `On Error Resume Next : d.Add k, v` is how
                    // they do "insert if absent".
                    return Err(Error::new(
                        457,
                        "This key is already associated with an element of this collection",
                    ));
                }
                self.entries.borrow_mut().push((key, value));
                Ok(Value::Empty)
            }

            "remove" => {
                let key = self.key_arg(args)?;
                match self.position(key) {
                    Some(i) => {
                        self.entries.borrow_mut().remove(i);
                        Ok(Value::Empty)
                    }
                    None => Err(Error::new(32811, "Element not found")),
                }
            }

            "removeall" => {
                self.entries.borrow_mut().clear();
                Ok(Value::Empty)
            }

            "keys" => Ok(collect(self, |e| e.0.clone())),
            "items" => Ok(collect(self, |e| e.1.clone())),

            "comparemode" => Ok(Value::Long(*self.compare_mode.borrow())),

            _ => Err(Error::no_such_member(name)),
        }
    }

    fn set(&self, name: &str, args: &[Value], value: Value, _by_ref: bool) -> Result<()> {
        match &*name.to_ascii_lowercase() {
            "" | "item" => {
                let key = self.key_arg(args)?.clone();
                match self.position(&key) {
                    Some(i) => self.entries.borrow_mut()[i].1 = value,
                    None => self.entries.borrow_mut().push((key, value)),
                }
                Ok(())
            }
            // Assigning a key **renames** it, keeping the value in place.
            "key" => {
                let key = self.key_arg(args)?.clone();
                match self.position(&key) {
                    Some(i) => {
                        self.entries.borrow_mut()[i].0 = value;
                        Ok(())
                    }
                    None => Err(Error::new(32811, "Element not found")),
                }
            }
            "comparemode" => {
                *self.compare_mode.borrow_mut() = value.to_int()?;
                Ok(())
            }
            _ => Err(Error::no_such_member(name)),
        }
    }

    /// A dictionary used as a value is its `Item`, which is why `d(k)` works.
    fn default_value(&self) -> Result<Value> {
        Err(Error::object_required())
    }

    /// `For Each k In d` walks the **keys**, not the values.
    fn enumerate(&self) -> Option<Vec<Value>> {
        Some(self.entries.borrow().iter().map(|e| e.0.clone()).collect())
    }
}

fn collect(d: &Dictionary, f: impl Fn(&(Value, Value)) -> Value) -> Value {
    let items: Vec<Value> = d.entries.borrow().iter().map(f).collect();
    Value::Array(Rc::new(RefCell::new(Array::from_values(items))))
}

/// Whether two keys are the same key.
///
/// Objects compare by identity, everything else by value with VBScript's usual
/// coercion — so `1` and `"1"` are one key. Tables use both: `core.vbs` keys
/// its timer dictionaries on **class instances**, and a table keys its modes on
/// strings.
fn same_key(a: &Value, b: &Value, text_compare: bool) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => x.same_object(y),
        (Value::Instance(x), Value::Instance(y)) => Rc::ptr_eq(x, y),
        (Value::Str(x), Value::Str(y)) if text_compare => x.eq_ignore_ascii_case(y),
        _ => {
            // Anything else goes through the `=` operator, which already knows
            // that a numeric string equals a number.
            crate::ops::binary(crate::ast::BinOp::Eq, a, b)
                .and_then(|v| v.to_bool())
                .unwrap_or(false)
        }
    }
}

/// `CreateObject("Scripting.FileSystemObject")`, for a host with no files.
///
/// Tables use this for one thing: remembering a preference between sessions —
/// a colour lookup table, a chosen camera, a high score. Circus keeps its LUT
/// number in `Circus_LUT.txt`, and half the tables of the last decade do
/// something like it.
///
/// A browser has no such folder, and the answer is not to refuse: the scripts
/// that use it are written defensively, because on a real machine the file is
/// not there the first time either.
///
/// ```vbscript
/// Set FileObj = CreateObject("Scripting.FileSystemObject")
/// If Not FileObj.FolderExists(UserDirectory) Then
///     LUTset = 17
///     Exit Sub
/// End If
/// ```
///
/// So this answers **no** to every question about what exists, and swallows
/// every write. A table takes the path it takes on a machine it has never run
/// on before, which is the truth: it has not. Refusing to create the object at
/// all fails the whole script on the `CreateObject` line, and the table does
/// not load.
///
/// What it does *not* do is pretend to persist. A host that wants the
/// preference to survive a reload should put it somewhere a browser can keep
/// it, and this is the seam to do that behind.
pub struct FileSystem;

impl Object for FileSystem {
    fn type_name(&self) -> &'static str {
        "FileSystemObject"
    }

    fn get(&self, name: &str, _args: &[Value]) -> Result<Value> {
        match &*name.to_ascii_lowercase() {
            // Nothing is there, which is what a table checks before reading.
            "fileexists" | "folderexists" | "driveexists" => Ok(Value::Bool(false)),
            // And a file it opens to write is somewhere to write to.
            "createtextfile" | "opentextfile" | "getfile" | "getfolder" | "createfolder"
            | "getspecialfolder" => Ok(Value::Object(Rc::new(TextStream))),
            "buildpath" => Ok(Value::Empty),
            other => Err(Error::new(
                438,
                format!("Object doesn't support this property or method: '{other}'"),
            )),
        }
    }

    fn set(&self, _name: &str, _args: &[Value], _value: Value, _by_ref: bool) -> Result<()> {
        Ok(())
    }
}

/// What [`FileSystem`] hands back for a file: something that takes writes and
/// has nothing to read.
pub struct TextStream;

impl Object for TextStream {
    fn type_name(&self) -> &'static str {
        "TextStream"
    }

    fn get(&self, name: &str, _args: &[Value]) -> Result<Value> {
        match &*name.to_ascii_lowercase() {
            // A reader should stop before it starts rather than block.
            "atendofstream" | "atendofline" => Ok(Value::Bool(true)),
            "readline" | "readall" | "read" => Ok(Value::str("")),
            "line" | "column" => Ok(Value::Long(1)),
            // Writing, closing, deleting: all fine, all nowhere.
            _ => Ok(Value::Empty),
        }
    }

    fn set(&self, _name: &str, _args: &[Value], _value: Value, _by_ref: bool) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> Dictionary {
        Dictionary::new()
    }

    fn get(d: &Dictionary, name: &str, args: &[Value]) -> Value {
        d.get(name, args).expect("the member should exist")
    }

    #[test]
    fn add_and_read_back() {
        let d = d();
        d.get("add", &[Value::str("a"), Value::Long(1)]).unwrap();
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 1.0);
        assert_eq!(
            get(&d, "item", &[Value::str("a")]).to_number().unwrap(),
            1.0
        );
    }

    #[test]
    fn adding_the_same_key_twice_raises() {
        // Which is what makes `On Error Resume Next : d.Add k, v` an
        // insert-if-absent, and tables write it that way.
        let d = d();
        d.get("add", &[Value::str("a"), Value::Long(1)]).unwrap();
        let e = d
            .get("add", &[Value::str("a"), Value::Long(2)])
            .unwrap_err();
        assert_eq!(e.number, 457);
    }

    #[test]
    fn reading_a_missing_key_creates_it() {
        // A real quirk of the original: a dictionary can grow just from being
        // looked at, and a table's `Count` reflects that.
        let d = d();
        assert!(matches!(
            get(&d, "item", &[Value::str("nope")]),
            Value::Empty
        ));
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 1.0);
        assert!(get(&d, "exists", &[Value::str("nope")]).to_bool().unwrap());
    }

    #[test]
    fn exists_does_not_create() {
        let d = d();
        assert!(!get(&d, "exists", &[Value::str("nope")]).to_bool().unwrap());
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 0.0);
    }

    #[test]
    fn assigning_an_item_inserts_or_replaces() {
        let d = d();
        d.set("item", &[Value::str("a")], Value::Long(1), false)
            .unwrap();
        d.set("item", &[Value::str("a")], Value::Long(2), false)
            .unwrap();
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 1.0);
        assert_eq!(
            get(&d, "item", &[Value::str("a")]).to_number().unwrap(),
            2.0
        );
    }

    #[test]
    fn remove_and_remove_all() {
        let d = d();
        d.get("add", &[Value::str("a"), Value::Long(1)]).unwrap();
        d.get("add", &[Value::str("b"), Value::Long(2)]).unwrap();
        d.get("remove", &[Value::str("a")]).unwrap();
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 1.0);
        assert_eq!(
            d.get("remove", &[Value::str("zzz")]).unwrap_err().number,
            32811
        );
        d.get("removeall", &[]).unwrap();
        assert_eq!(get(&d, "count", &[]).to_number().unwrap(), 0.0);
    }

    #[test]
    fn keys_and_items_come_out_as_arrays() {
        let d = d();
        d.get("add", &[Value::str("a"), Value::Long(1)]).unwrap();
        d.get("add", &[Value::str("b"), Value::Long(2)]).unwrap();

        let keys = get(&d, "keys", &[]).to_array().unwrap();
        assert_eq!(keys.borrow().items.len(), 2);
        let items = get(&d, "items", &[]).to_array().unwrap();
        assert_eq!(items.borrow().items[1].to_number().unwrap(), 2.0);
    }

    #[test]
    fn a_numeric_string_is_the_same_key_as_the_number() {
        let d = d();
        d.get("add", &[Value::Long(1), Value::str("one")]).unwrap();
        assert!(get(&d, "exists", &[Value::str("1")]).to_bool().unwrap());
    }

    #[test]
    fn compare_mode_decides_whether_case_matters() {
        let d = d();
        d.get("add", &[Value::str("Ball"), Value::Long(1)]).unwrap();
        assert!(!get(&d, "exists", &[Value::str("ball")]).to_bool().unwrap());

        d.set("comparemode", &[], Value::Long(1), false).unwrap();
        assert!(get(&d, "exists", &[Value::str("ball")]).to_bool().unwrap());
    }

    #[test]
    fn for_each_walks_the_keys() {
        let d = d();
        d.get("add", &[Value::str("a"), Value::Long(1)]).unwrap();
        let walked = d.enumerate().unwrap();
        assert_eq!(&*walked[0].to_str().unwrap(), "a");
    }
}
