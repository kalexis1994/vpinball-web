//! Running the tree.
//!
//! A tree-walking interpreter. Not a fast one, and deliberately so: a pinball
//! table's script runs on events — a switch closed, a timer fired — not in the
//! inner loop. The physics runs at 1000 Hz; the script runs a few dozen times a
//! second. Correctness and legibility buy more here than a bytecode compiler
//! would.
//!
//! # Scoping, which VBScript keeps simple
//!
//! There are two scopes and no more: the script's own, and the one inside a
//! procedure. No block scope — a variable `Dim`ed inside an `If` is visible for
//! the rest of the procedure. A class method sees its instance's fields plus
//! the script scope, but not the caller's locals.
//!
//! # The two things worth knowing before reading
//!
//! **`Foo(1)` is ambiguous until it runs.** It is a subscript if `Foo` is an
//! array, a call if `Foo` is a procedure, and a property read if `Foo` is a
//! host object. [`Interpreter::eval_index`] is where that is decided, and it
//! is the heart of the whole thing.
//!
//! **Errors and `Exit` travel the same channel.** `Exit Sub` unwinds exactly
//! the way an error does — out of nested `If`s, out of a `With`, out of a loop
//! body — so it rides on `Err` as a [`crate::error::Control`] rather than
//! making every statement return a three-way result. Nothing outside this
//! module can see them: `On Error Resume Next` checks and refuses to swallow
//! them, which is what keeps `Exit Sub` working inside a protected block.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::*;
use crate::builtins;
use crate::error::{Control, Error, Result};
use crate::instance::{Instance, MAX_NAME, NameMap, Slot, Vars, fold, fold_into, read, slot};
use crate::object::{Host, NoHost, Object};
use crate::ops;
use crate::parser;
use crate::value::{Array, Value};

/// How deep procedure calls may nest.
///
/// VBScript has no such limit; the native stack does. A table with a bug that
/// makes a handler call itself would otherwise take the process down instead of
/// reporting "Out of stack space", and in a browser that means the tab.
const MAX_CALL_DEPTH: u32 = 256;

/// One procedure's activation.
struct Frame {
    locals: Vars,
    /// The instance whose method is running, if this is one.
    me: Option<Rc<Instance>>,
    /// The cell a `Function` assigns its result to, under its own name.
    result: Option<Slot>,
    /// Whether `On Error Resume Next` is in force. It is **per procedure**:
    /// the real engine resets it on entry and restores it on return, so a sub
    /// cannot leave its caller unprotected or over-protected.
    resume_next: bool,
}

impl Frame {
    fn new() -> Self {
        Self {
            locals: Vars::new(),
            me: None,
            result: None,
            resume_next: false,
        }
    }
}

/// The `Err` object's state.
#[derive(Default)]
struct ErrState {
    number: i32,
    description: Rc<str>,
    source: Rc<str>,
}

/// The `Err` object itself.
///
/// It shares the interpreter's state rather than copying it, because a table's
/// idiom is `On Error Resume Next`, do something, `If Err.Number <> 0`, and the
/// read has to see what the failing statement recorded a moment earlier.
///
/// `Err` used bare — `If Err Then` — reads as its number, which is why it has a
/// default value. Visual Pinball's own scripts write it that way.
struct ErrObject(Rc<RefCell<ErrState>>);

impl Object for ErrObject {
    fn type_name(&self) -> &'static str {
        "ErrObject"
    }

    fn default_value(&self) -> Result<Value> {
        Ok(Value::Long(self.0.borrow().number))
    }

    fn get(&self, name: &str, args: &[Value]) -> Result<Value> {
        match &*name.to_ascii_lowercase() {
            "number" => Ok(Value::Long(self.0.borrow().number)),
            "description" => Ok(Value::Str(self.0.borrow().description.clone())),
            "source" => Ok(Value::Str(self.0.borrow().source.clone())),
            "clear" => {
                *self.0.borrow_mut() = ErrState::default();
                Ok(Value::Empty)
            }
            "raise" => {
                let number = args.first().map(Value::to_int).transpose()?.unwrap_or(0);
                let source = match args.get(1) {
                    Some(v) => v.to_str()?,
                    None => Rc::from("Microsoft VBScript runtime error"),
                };
                let description = match args.get(2) {
                    Some(v) => v.to_str()?,
                    None => Rc::from("Unknown runtime error"),
                };
                Err(Error::raised(number, source, description))
            }
            // `HelpFile` and `HelpContext` exist and are always empty.
            "helpfile" | "helpcontext" => Ok(Value::str("")),
            _ => Err(Error::no_such_member(name)),
        }
    }

    fn set(&self, name: &str, _args: &[Value], value: Value, _by_ref: bool) -> Result<()> {
        let mut e = self.0.borrow_mut();
        match &*name.to_ascii_lowercase() {
            "number" => e.number = value.to_int()?,
            "description" => e.description = value.to_str()?,
            "source" => e.source = value.to_str()?,
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }
}

/// A parsed and running script.
pub struct Interpreter {
    globals: RefCell<Vars>,
    procs: RefCell<NameMap<Rc<Proc>>>,
    classes: RefCell<NameMap<Rc<ClassDef>>>,
    /// Names declared `Const`, which may not be assigned to.
    consts: RefCell<Vars>,

    frames: RefCell<Vec<Frame>>,
    /// How much script has been run. Diagnostics only, and off the wasm build:
    /// the question "is the interpreter slow or is the table asking for a lot"
    /// cannot be answered without a denominator.
    #[cfg(not(target_arch = "wasm32"))]
    stmts: std::cell::Cell<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    exprs: std::cell::Cell<u64>,
    /// The `With` subjects currently open, innermost last.
    with_stack: RefCell<Vec<Value>>,
    err: Rc<RefCell<ErrState>>,
    depth: RefCell<u32>,

    option_explicit: RefCell<bool>,
    /// `On Error Resume Next` at script level, outside any procedure.
    global_resume_next: RefCell<bool>,
    rng: RefCell<u32>,
    /// The last number `Rnd` produced, for `Rnd` with a negative argument.
    last_rnd: RefCell<f64>,
    /// The locale identifier `SetLocale` was last given. See the call.
    locale: std::cell::Cell<i32>,

    host: Rc<dyn Host>,
}

/// The locale this interpreter behaves as, and the one tables ask for.
///
/// 1033 is `en-US`. Numbers here are parsed and printed one way whatever is
/// set, so this is a fact about the implementation rather than a setting.
const US_ENGLISH: i32 = 1033;

impl Default for Interpreter {
    fn default() -> Self {
        Self::new(Rc::new(NoHost))
    }
}

impl Interpreter {
    pub fn new(host: Rc<dyn Host>) -> Self {
        Self {
            globals: RefCell::new(Vars::new()),
            #[cfg(not(target_arch = "wasm32"))]
            stmts: std::cell::Cell::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            exprs: std::cell::Cell::new(0),
            procs: RefCell::new(NameMap::default()),
            classes: RefCell::new(NameMap::default()),
            consts: RefCell::new(Vars::new()),
            frames: RefCell::new(Vec::new()),
            with_stack: RefCell::new(Vec::new()),
            err: Rc::new(RefCell::new(ErrState::default())),
            depth: RefCell::new(0),
            option_explicit: RefCell::new(false),
            global_resume_next: RefCell::new(false),
            // Any seed but zero; xorshift is stuck at zero forever.
            rng: RefCell::new(0x2545_F491),
            last_rnd: RefCell::new(0.0),
            locale: std::cell::Cell::new(US_ENGLISH),
            host,
        }
    }

    /// Parses and runs a script, adding whatever it declares to this
    /// interpreter. Can be called more than once, which is what
    /// `ExecuteGlobal` and a table that includes several helper scripts need.
    pub fn load(&self, src: &str) -> Result<()> {
        let program = parser::parse(src)?;
        // Only while this unit's own top level runs. `Option Explicit` belongs
        // to the file it is written in, and leaving it on afterwards made it
        // everybody's: `core.vbs` opens with it and is loaded first, so every
        // table script that followed inherited a rule it never asked for and
        // died at its first undeclared name — reported against the table's own
        // line, which makes it look like the table's fault.
        //
        // Procedures do not need the guard: each one carries the rule of the
        // unit it was written in and puts it back for the length of the call
        // (see [`Interpreter::invoke`]), which is what VBScript does by
        // settling this when it compiles.
        let _unit = self.with_option_explicit(program.option_explicit);
        // Procedures and classes are hoisted: a script routinely calls
        // something defined further down the file, and the real engine compiles
        // the whole thing before running any of it.
        self.hoist(&program.body);
        self.run_block(&program.body)?;
        Ok(())
    }

    /// Calls a procedure the script declared. This is how a host delivers an
    /// event: `interp.call("Bumper1_Hit", &[])`.
    ///
    /// A name that does not exist is **not** an error. A table only defines
    /// handlers for the things it cares about, and asking whether it wants to
    /// know about every switch would be the caller's job otherwise.
    pub fn call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        let Some(p) = self.lookup_proc(name) else {
            return Ok(None);
        };
        let args: Vec<Slot> = args.iter().map(|v| slot(v.clone())).collect();
        self.invoke(&p, &args, None).map(Some)
    }

    /// Whether the script declared a procedure with this name.
    pub fn has_proc(&self, name: &str) -> bool {
        let mut buf = [0u8; MAX_NAME];
        match fold_into(name, &mut buf) {
            Some(key) => self.procs.borrow().contains_key(key),
            None => self.procs.borrow().contains_key(fold(name).as_str()),
        }
    }

    /// Reads a script-level variable.
    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.globals.borrow().get(name).map(read)
    }

    /// Writes a script-level variable, declaring it if needed.
    pub fn set_global(&self, name: &str, v: Value) {
        self.globals.borrow_mut().assign(name, v);
    }

    // -- declarations ------------------------------------------------------

    /// Registers everything a block declares, before any of it runs.
    ///
    /// Procedures and classes, because a script routinely calls something
    /// defined further down the file. **And declarations**, because VBScript
    /// hoists `Dim` and `Const` to the top of their scope: they are handled
    /// when the unit is compiled, not when the line is reached.
    ///
    /// That is not a nicety. Visual Pinball's `core.vbs` writes
    /// `FlipperSolNumber(0) = sLLFlipper` on line 2090 and
    /// `Const sLLFlipper = 48` on line 2850, and without hoisting the library
    /// does not load at all.
    fn hoist(&self, body: &[Stmt]) {
        for s in body {
            match &s.kind {
                StmtKind::Proc(p) => {
                    self.procs
                        .borrow_mut()
                        .insert(fold(&p.name).into_boxed_str(), p.clone());
                }
                StmtKind::Class(c) => {
                    self.classes
                        .borrow_mut()
                        .insert(fold(&c.name).into_boxed_str(), c.clone());
                }
                _ => {}
            }
        }
        // Constants first: an array's bounds are allowed to be one.
        self.hoist_declarations(body, true);
        self.hoist_declarations(body, false);
    }

    /// Walks a block for `Dim` and `Const` and declares what it finds.
    ///
    /// Recursive, because a `Dim` inside an `If` is hoisted just the same — the
    /// branch not being taken does not stop the name existing.
    fn hoist_declarations(&self, body: &[Stmt], constants: bool) {
        for s in body {
            match &s.kind {
                StmtKind::Const(items) if constants => {
                    for (name, e) in items {
                        // A constant expression can only use literals and other
                        // constants, so this cannot depend on anything that has
                        // not run yet. If it somehow fails, the statement will
                        // do it again in order.
                        if let Ok(v) = self.eval(e) {
                            self.consts.borrow_mut().assign(name, v.clone());
                            self.declare(name, v);
                        }
                    }
                }
                StmtKind::Dim(names) if !constants => {
                    for d in names {
                        // Bounds are evaluated if they can be. When they cannot
                        // — a table with a bound that is not really constant —
                        // the name is still declared, and the `Dim` statement
                        // gives it its shape when it is reached.
                        let value = match &d.bounds {
                            Some(b) if !b.is_empty() => match self.eval_bounds(b) {
                                Ok(bounds) => Array::new(bounds)
                                    .map(|a| Value::Array(Rc::new(RefCell::new(a))))
                                    .unwrap_or(Value::Empty),
                                Err(_) => Value::Empty,
                            },
                            Some(_) => {
                                Value::Array(Rc::new(RefCell::new(Array::from_values(Vec::new()))))
                            }
                            None => Value::Empty,
                        };
                        self.declare(&d.name, value);
                    }
                }
                // The blocks a declaration can hide in.
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    for (_, b) in branches {
                        self.hoist_declarations(b, constants);
                    }
                    if let Some(b) = else_body {
                        self.hoist_declarations(b, constants);
                    }
                }
                StmtKind::For { body, .. }
                | StmtKind::ForEach { body, .. }
                | StmtKind::Do { body, .. }
                | StmtKind::While { body, .. }
                | StmtKind::With { body, .. } => self.hoist_declarations(body, constants),
                StmtKind::Select { cases, default, .. } => {
                    for c in cases {
                        self.hoist_declarations(&c.body, constants);
                    }
                    if let Some(b) = default {
                        self.hoist_declarations(b, constants);
                    }
                }
                _ => {}
            }
        }
    }

    /// Statements and expressions run since the last time this was asked.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_work(&self) -> (u64, u64) {
        (self.stmts.replace(0), self.exprs.replace(0))
    }

    fn lookup_proc(&self, name: &str) -> Option<Rc<Proc>> {
        // Every call a script makes comes through here, so it folds on the
        // stack: see [`fold_into`].
        let mut buf = [0u8; MAX_NAME];
        match fold_into(name, &mut buf) {
            Some(key) => self.procs.borrow().get(key).cloned(),
            None => self.procs.borrow().get(fold(name).as_str()).cloned(),
        }
    }

    /// A method of the class whose method is running.
    ///
    /// Inside a class, one method calls another by bare name — `core.vbs`'s
    /// switch classes are full of `SetSw aNo, aStatus`. The name is not a
    /// local, not a field and not a global, so without this it resolves to
    /// nothing and the library fails at the first table that uses a switch.
    ///
    /// It is looked up **before** the global procedures, because a class member
    /// shadows a global of the same name inside that class.
    fn me_member(&self, name: &str) -> Option<(Rc<Proc>, Rc<Instance>)> {
        let me = self.current_me()?;
        let p = find_member(&me.def, name, |p| {
            p.property != Some(PropKind::Let) && p.property != Some(PropKind::Set)
        })?;
        Some((p, me))
    }

    /// The same, for the setter side of a property.
    fn me_setter(&self, name: &str) -> Option<(Rc<Proc>, Rc<Instance>)> {
        let me = self.current_me()?;
        let p = find_member(&me.def, name, |p| {
            matches!(p.property, Some(PropKind::Let) | Some(PropKind::Set))
        })?;
        Some((p, me))
    }

    // -- statements --------------------------------------------------------

    fn run_block(&self, body: &[Stmt]) -> Result<()> {
        for s in body {
            self.run_stmt(s)?;
        }
        Ok(())
    }

    fn run_stmt(&self, s: &Stmt) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        self.stmts.set(self.stmts.get() + 1);
        match self.run_stmt_inner(s) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Control flow is never an error and is never swallowed.
                if e.as_control().is_some() {
                    return Err(e);
                }
                let e = e.at_line(s.line);
                if self.resume_next() {
                    // `On Error Resume Next`: record it in `Err` and carry on
                    // with the next statement, which is the whole point.
                    self.set_err(&e);
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    fn run_stmt_inner(&self, s: &Stmt) -> Result<()> {
        match &s.kind {
            StmtKind::Nop => Ok(()),

            StmtKind::Dim(names) => {
                for d in names {
                    let v = match &d.bounds {
                        None => Value::Empty,
                        Some(b) if b.is_empty() => {
                            // `Dim a()`: an array with no size yet. `ReDim`
                            // will give it one; until then it is empty rather
                            // than absent, so `IsArray` is already true.
                            Value::Array(Rc::new(RefCell::new(Array::from_values(Vec::new()))))
                        }
                        Some(b) => {
                            Value::Array(Rc::new(RefCell::new(Array::new(self.eval_bounds(b)?)?)))
                        }
                    };
                    self.declare(&d.name, v);
                }
                Ok(())
            }

            StmtKind::ReDim { preserve, targets } => {
                for (name, bounds) in targets {
                    let bounds = self.eval_bounds(bounds)?;
                    let cell = self.lookup_or_declare(name);
                    let current = read(&cell);
                    match (preserve, &current) {
                        (true, Value::Array(a)) => a.borrow_mut().redim_preserve(bounds)?,
                        _ => {
                            *cell.borrow_mut() =
                                Value::Array(Rc::new(RefCell::new(Array::new(bounds)?)));
                        }
                    }
                }
                Ok(())
            }

            StmtKind::Const(items) => {
                for (name, e) in items {
                    let v = self.eval(e)?;
                    self.consts.borrow_mut().assign(name, v.clone());
                    self.declare(name, v);
                }
                Ok(())
            }

            StmtKind::Assign { target, value, set } => {
                let v = self.eval(value)?;
                // `x = obj` stores the object's default value; `Set x = obj`
                // stores the object. Getting this backwards is how a table ends
                // up with a string where it wanted a reference.
                //
                // Only where the target is a *variable*, though. Assigning to
                // something's property is a call — the value becomes an
                // argument — and an argument is passed as it is. Dereferencing
                // there is what broke the pattern half the modern tables are
                // built on:
                //
                //     Public Property Let Object(a) : Set Slingshot = a : End Property
                //     LS.Object = LeftSlingshot
                //
                // The property is a `Let`, so it takes a value; the value it
                // is given is a table part, and the first thing it does is
                // `Set` it. Turning that part into its default property on the
                // way in leaves the `Set` with nothing to set, and The Getaway
                // stopped at line 225 with "Object required".
                self.assign_to(target, v, *set)
            }

            StmtKind::Call(e) => {
                self.eval_call_statement(e)?;
                Ok(())
            }

            StmtKind::If {
                branches,
                else_body,
            } => {
                for (cond, body) in branches {
                    if self.eval(cond)?.to_bool()? {
                        return self.run_block(body);
                    }
                }
                match else_body {
                    Some(b) => self.run_block(b),
                    None => Ok(()),
                }
            }

            StmtKind::For {
                var,
                from,
                to,
                step,
                body,
            } => self.run_for(var, from, to, step.as_ref(), body),

            StmtKind::ForEach { var, seq, body } => self.run_for_each(var, seq, body),

            StmtKind::Do { cond, body } => self.run_do(cond.as_ref(), body),

            StmtKind::While { cond, body } => {
                while self.eval(cond)?.to_bool()? {
                    match self.run_block(body) {
                        Err(e) if e.as_control() == Some(Control::ExitDo) => break,
                        other => other?,
                    }
                }
                Ok(())
            }

            StmtKind::Select {
                subject,
                cases,
                default,
            } => self.run_select(subject, cases, default.as_deref()),

            StmtKind::With { subject, body } => {
                let v = self.eval(subject)?;
                self.with_stack.borrow_mut().push(v);
                let r = self.run_block(body);
                self.with_stack.borrow_mut().pop();
                r
            }

            // Already hoisted; reaching one at run time just means stepping
            // over its definition.
            StmtKind::Proc(_) | StmtKind::Class(_) => Ok(()),

            StmtKind::Exit(kind) => Err(Error::control(match kind {
                ExitKind::Sub => Control::ExitSub,
                ExitKind::Function => Control::ExitFunction,
                ExitKind::Property => Control::ExitProperty,
                ExitKind::For => Control::ExitFor,
                ExitKind::Do => Control::ExitDo,
            })),

            StmtKind::OnError { resume_next } => {
                self.set_resume_next(*resume_next);
                // **Both** forms clear `Err`. That is documented and it is
                // load-bearing: `s11.vbs` opens a procedure with
                // `On Error Resume Next` and then asks `If VPBuildVersion < 0
                // Or Err Then`, meaning "did *that* line fail". If `Err` still
                // held something from earlier in the run, the library takes a
                // path that reads files off a disk the browser does not have,
                // and the table reports that it cannot find `core.vbs`.
                self.clear_err();
                Ok(())
            }

            StmtKind::Erase(names) => {
                for name in names {
                    if let Some(cell) = self.lookup(name)
                        && let Value::Array(a) = read(&cell)
                    {
                        // `Erase` on a fixed-size array empties its elements
                        // and keeps its shape.
                        let mut a = a.borrow_mut();
                        for v in &mut a.items {
                            *v = Value::Empty;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn eval_bounds(&self, exprs: &[Expr]) -> Result<Vec<i32>> {
        exprs.iter().map(|e| self.eval(e)?.to_int()).collect()
    }

    /// `For i = a To b [Step s]`.
    ///
    /// The limit and the step are evaluated **once**, before the loop, so a
    /// body that changes them does not change how long it runs. The counter
    /// itself is an ordinary variable the body can read and even write.
    fn run_for(
        &self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
    ) -> Result<()> {
        let start = self.eval(from)?.to_number()?;
        let limit = self.eval(to)?.to_number()?;
        let step = match step {
            Some(e) => self.eval(e)?.to_number()?,
            None => 1.0,
        };
        // A zero step would spin forever. The real engine does exactly that;
        // we refuse, because a hung tab is worse than an error message.
        if step == 0.0 {
            return Err(Error::invalid_call());
        }

        let cell = self.lookup_or_declare(var);
        *cell.borrow_mut() = Value::from_number(start);

        loop {
            let current = read(&cell).to_number()?;
            let done = if step > 0.0 {
                current > limit
            } else {
                current < limit
            };
            if done {
                return Ok(());
            }
            match self.run_block(body) {
                Err(e) if e.as_control() == Some(Control::ExitFor) => return Ok(()),
                other => other?,
            }
            let current = read(&cell).to_number()?;
            *cell.borrow_mut() = Value::from_number(current + step);
        }
    }

    fn run_for_each(&self, var: &str, seq: &Expr, body: &[Stmt]) -> Result<()> {
        let items = match self.eval(seq)? {
            Value::Array(a) => a.borrow().items.clone(),
            Value::Object(o) => o.enumerate().ok_or_else(Error::type_mismatch)?,
            // `For Each` over `Empty` runs zero times, which is what a table
            // that has not filled its collection yet expects.
            Value::Empty | Value::Nothing => return Ok(()),
            _ => return Err(Error::type_mismatch()),
        };

        let cell = self.lookup_or_declare(var);
        for item in items {
            *cell.borrow_mut() = item;
            match self.run_block(body) {
                Err(e) if e.as_control() == Some(Control::ExitFor) => return Ok(()),
                other => other?,
            }
        }
        Ok(())
    }

    fn run_do(&self, cond: Option<&DoCond>, body: &[Stmt]) -> Result<()> {
        loop {
            if let Some(c) = cond
                && !c.at_end
                && !self.test(c)?
            {
                return Ok(());
            }
            match self.run_block(body) {
                Err(e) if e.as_control() == Some(Control::ExitDo) => return Ok(()),
                other => other?,
            }
            if let Some(c) = cond
                && c.at_end
                && !self.test(c)?
            {
                return Ok(());
            }
            // `Do ... Loop` with no condition at all only ends on `Exit Do`.
        }
    }

    /// A loop condition, with `Until` inverting it.
    fn test(&self, c: &DoCond) -> Result<bool> {
        let v = self.eval(&c.expr)?.to_bool()?;
        Ok(if c.until { !v } else { v })
    }

    fn run_select(&self, subject: &Expr, cases: &[Case], default: Option<&[Stmt]>) -> Result<()> {
        let v = self.eval(subject)?;
        for case in cases {
            for t in &case.tests {
                let t = self.eval(t)?;
                if ops::binary(BinOp::Eq, &v, &t)?.to_bool().unwrap_or(false) {
                    return self.run_block(&case.body);
                }
            }
        }
        match default {
            Some(b) => self.run_block(b),
            None => Ok(()),
        }
    }

    // -- assignment --------------------------------------------------------

    /// Writes a value into whatever the expression names, saying whether this
    /// was a `Set`.
    ///
    /// The difference is where the dereference happens. `x = obj` stores the
    /// object's *default property* and `Set x = obj` stores the object — but
    /// that rule is about writing a **variable**, and assigning to a property
    /// is not writing a variable, it is calling something with an argument.
    /// An argument is passed as it is.
    ///
    /// Dereferencing there is what broke the pattern half the modern tables
    /// are built on:
    ///
    /// ```vbs
    /// Public Property Let Object(a) : Set Slingshot = a : End Property
    /// LS.Object = LeftSlingshot
    /// ```
    ///
    /// The property is a `Let`, so it takes a value; the value it is given is
    /// a table part, and the first thing it does with it is `Set` it. Turning
    /// that part into its default property on the way in leaves the `Set` with
    /// nothing to set, and The Getaway stopped at its line 225 with "Object
    /// required" before the table had drawn anything.
    fn assign_to(&self, target: &Expr, v: Value, set: bool) -> Result<()> {
        // Only a cell gets the dereference, and only the two places below
        // write one.
        let into_cell = |v: Value| if set { Ok(v) } else { self.devalue(v) };
        match target {
            Expr::Ident(name) => {
                if self.consts.borrow().contains(name) {
                    return Err(Error::new(
                        500,
                        format!("cannot assign to the constant '{name}'"),
                    ));
                }
                // Inside a class, a bare name can be one of its own
                // `Property Let`s rather than a variable.
                if self.lookup(name).is_none()
                    && let Some((p, me)) = self.me_setter(name)
                {
                    self.invoke(&p, &[slot(v)], Some(me))?;
                    return Ok(());
                }
                let cell = self.lookup_or_declare_checked(name)?;
                *cell.borrow_mut() = into_cell(v)?;
                Ok(())
            }

            Expr::Member { base, name } => {
                let obj = self.eval(base)?;
                self.set_member(&obj, name, &[], v)
            }

            Expr::WithMember { name } => {
                let obj = self.current_with()?;
                self.set_member(&obj, name, &[], v)
            }

            // `a(1) = x` — a subscript, or a parameterised property on an
            // object, or a `Property Let` with an index.
            Expr::Index { base, args } => {
                let subs = self.eval_args(args)?;
                match &**base {
                    Expr::Member { base: b, name } => {
                        let obj = self.eval(b)?;
                        self.set_member(&obj, name, &subs, v)
                    }
                    Expr::WithMember { name } => {
                        let obj = self.current_with()?;
                        self.set_member(&obj, name, &subs, v)
                    }
                    Expr::Ident(name) => {
                        // A parameterised `Property Let` of the class we are
                        // inside: `CallBack(0) = x`, where `CallBack` is not a
                        // field but a setter that takes an index. `sam.vbs`
                        // does exactly this, and read as an array subscript it
                        // is an undefined variable.
                        if self.lookup(name).is_none()
                            && let Some((p, me)) = self.me_setter(name)
                        {
                            let mut slots: Vec<Slot> =
                                subs.iter().map(|a| slot(a.clone())).collect();
                            slots.push(slot(v));
                            self.invoke(&p, &slots, Some(me))?;
                            return Ok(());
                        }
                        let cell = self
                            .lookup(name)
                            .ok_or_else(|| Error::undefined_variable(name))?;
                        let current = read(&cell);
                        match current {
                            Value::Array(a) => {
                                let subs = ints(&subs)?;
                                a.borrow_mut().set(&subs, into_cell(v)?)
                            }
                            // An object with a parameterised property.
                            other if other.is_object() => self.set_member(&other, "", &subs, v),
                            _ => Err(Error::type_mismatch()),
                        }
                    }
                    _ => Err(Error::type_mismatch()),
                }
            }

            _ => Err(Error::new(500, "cannot assign to this expression")),
        }
    }

    fn set_member(&self, obj: &Value, name: &str, args: &[Value], v: Value) -> Result<()> {
        let by_ref = v.is_object();
        match obj {
            Value::Object(o) => o.set(name, args, v, by_ref),
            Value::Instance(inst) => self.set_instance_member(inst, name, args, v),
            Value::Nothing => Err(Error::object_variable_not_set()),
            _ => Err(Error::object_required()),
        }
    }

    fn set_instance_member(
        &self,
        inst: &Rc<Instance>,
        name: &str,
        args: &[Value],
        v: Value,
    ) -> Result<()> {
        // A `Property Let` or `Property Set` wins over a plain field, which is
        // how a class hides its storage behind a validating setter.
        if let Some(p) = find_member(&inst.def, name, |p| {
            matches!(p.property, Some(PropKind::Let) | Some(PropKind::Set))
        }) {
            let mut slots: Vec<Slot> = args.iter().map(|a| slot(a.clone())).collect();
            slots.push(slot(v));
            self.invoke(&p, &slots, Some(inst.clone()))?;
            return Ok(());
        }
        if let Some(cell) = inst.field(name) {
            if args.is_empty() {
                *cell.borrow_mut() = v;
                return Ok(());
            }
            // An indexed write into a field that holds an array.
            if let Value::Array(a) = read(&cell) {
                return a.borrow_mut().set(&ints(args)?, v);
            }
        }
        Err(Error::no_such_member(name))
    }

    // -- expressions -------------------------------------------------------

    pub fn eval(&self, e: &Expr) -> Result<Value> {
        #[cfg(not(target_arch = "wasm32"))]
        self.exprs.set(self.exprs.get() + 1);
        match e {
            Expr::Empty => Ok(Value::Empty),
            Expr::Null => Ok(Value::Null),
            Expr::Nothing => Ok(Value::Nothing),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Number(n) => Ok(Value::from_number(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),

            Expr::Ident(name) => self.eval_ident(name),

            Expr::Member { base, name } => {
                let obj = self.eval(base)?;
                self.get_member(&obj, name, &[])
            }
            Expr::WithMember { name } => {
                let obj = self.current_with()?;
                self.get_member(&obj, name, &[])
            }

            Expr::Index { base, args } => self.eval_index(base, args),

            Expr::New(name) => self.construct(name),

            Expr::Unary { op, operand } => {
                let v = self.eval(operand)?;
                ops::unary(*op, &v)
            }
            Expr::Binary { op, lhs, rhs } => {
                // Deliberately not short-circuiting: see `ops`.
                let l = self.eval(lhs)?;
                let r = self.eval(rhs)?;
                ops::binary(*op, &l, &r)
            }
        }
    }

    /// A bare name.
    ///
    /// The search order is the one VBScript uses, and it decides real
    /// behaviour: locals shadow globals, and a script's own procedure shadows a
    /// builtin of the same name — which tables rely on to override `MsgBox`.
    fn eval_ident(&self, name: &str) -> Result<Value> {
        if let Some(cell) = self.lookup(name) {
            return Ok(read(&cell));
        }
        // `Me` inside a class method is the instance, and it has to be found
        // here rather than left to the host. A host has its own idea of what
        // `Me` means — in a pinball table it is the part whose event is
        // running — and a class that hands `Me` to somebody else would
        // otherwise hand over the wrong object entirely. `core.vbs` does
        // exactly that: `vpmTimer.EnableUpdate Me, False, aEnabled` is how a
        // ball trough asks to be updated, and registering the table instead
        // means the trough never reports a ball and no game can start.
        if name.eq_ignore_ascii_case("me")
            && let Some(me) = self.current_me()
        {
            return Ok(Value::Instance(me));
        }
        if let Some((p, me)) = self.me_member(name) {
            return self.invoke(&p, &[], Some(me));
        }
        if let Some(p) = self.lookup_proc(name) {
            return self.invoke(&p, &[], self.current_me());
        }
        if let Some(v) = self.builtin_value(name)? {
            return Ok(v);
        }
        if let Some(v) = self.host_global(name) {
            return Ok(v);
        }
        if *self.option_explicit.borrow() {
            return Err(Error::undefined_variable(name));
        }
        // Without `Option Explicit`, reading an unknown name declares it as
        // `Empty` rather than failing. That is VBScript's default and it is why
        // a typo in a table is so hard to find.
        Ok(read(&self.declare(name, Value::Empty)))
    }

    /// `base(args)` — the ambiguous one.
    fn eval_index(&self, base: &Expr, args: &[Arg]) -> Result<Value> {
        // `obj.Member(args)` and `.Member(args)`: always the object's problem.
        match base {
            Expr::Member { base: b, name } => {
                let obj = self.eval(b)?;
                let a = self.eval_args(args)?;
                return self.get_member(&obj, name, &a);
            }
            Expr::WithMember { name } => {
                let obj = self.current_with()?;
                let a = self.eval_args(args)?;
                return self.get_member(&obj, name, &a);
            }
            _ => {}
        }

        if let Expr::Ident(name) = base {
            // A variable first: an array subscript, a call through a
            // procedure reference, or an object with a default indexed
            // property.
            if let Some(cell) = self.lookup(name) {
                let v = read(&cell);
                match v {
                    Value::Array(a) => {
                        let subs = ints(&self.eval_args(args)?)?;
                        // Bound in a `let` and returned separately so the
                        // borrow of the array ends before the caller runs.
                        let item = a.borrow().get(&subs);
                        return item;
                    }
                    // A variable holding what `GetRef` returned.
                    Value::Proc(p) => {
                        let slots = self.eval_arg_slots(&p, args)?;
                        return self.invoke(&p, &slots, self.current_me());
                    }
                    v if v.is_object() => {
                        let a = self.eval_args(args)?;
                        return self.get_member(&v, "", &a);
                    }
                    // Anything else: fall through to the procedure lookup
                    // below. That is what makes recursion work — inside
                    // `Function Fact`, the name `Fact` is also the variable
                    // holding the result, so `Fact(n - 1)` finds a plain
                    // `Empty` here and has to mean the call.
                    _ => {}
                }
            }
            // A method of the class we are inside, which shadows a global of
            // the same name.
            if let Some((p, me)) = self.me_member(name) {
                let slots = self.eval_arg_slots(&p, args)?;
                return self.invoke(&p, &slots, Some(me));
            }
            // Then a procedure the script declared.
            if let Some(p) = self.lookup_proc(name) {
                let slots = self.eval_arg_slots(&p, args)?;
                return self.invoke(&p, &slots, self.current_me());
            }
            // Then a builtin.
            let a = self.eval_args(args)?;
            if let Some(v) = self.call_builtin(name, &a)? {
                return Ok(v);
            }
            // Then something the host owns.
            if let Some(v) = self.host_global(name) {
                return self.get_member(&v, "", &a);
            }
            return Err(Error::undefined_procedure(name));
        }

        // Anything else: evaluate it and index the result.
        let v = self.eval(base)?;
        let a = self.eval_args(args)?;
        match v {
            Value::Array(arr) => {
                let subs = ints(&a)?;
                arr.borrow().get(&subs)
            }
            v if v.is_object() => self.get_member(&v, "", &a),
            _ => Err(Error::type_mismatch()),
        }
    }

    /// A statement that is a call, where a bare `Foo` means "call it".
    fn eval_call_statement(&self, e: &Expr) -> Result<Value> {
        // `Foo` on its own as a statement calls it even with no parentheses.
        if let Expr::Ident(name) = e
            && self.lookup(name).is_none()
        {
            if let Some((p, me)) = self.me_member(name) {
                return self.invoke(&p, &[], Some(me));
            }
            if let Some(p) = self.lookup_proc(name) {
                return self.invoke(&p, &[], self.current_me());
            }
        }
        // A variable holding a procedure reference, called with no arguments.
        if let Expr::Ident(name) = e
            && let Some(cell) = self.lookup(name)
            && matches!(read(&cell), Value::Proc(_))
        {
            return self.call_proc_value(&read(&cell), &[]);
        }
        // `obj.Method` with no arguments, as a statement.
        if let Expr::Member { base, name } = e {
            let obj = self.eval(base)?;
            return self.call_member(&obj, name, &[]);
        }
        if let Expr::WithMember { name } = e {
            let obj = self.current_with()?;
            return self.call_member(&obj, name, &[]);
        }
        // `obj.Method arg` arrives as an `Index` whose base is a `Member`.
        if let Expr::Index { base, args } = e {
            if let Expr::Member { base: b, name } = &**base {
                let obj = self.eval(b)?;
                let a = self.eval_args(args)?;
                return self.call_member(&obj, name, &a);
            }
            if let Expr::WithMember { name } = &**base {
                let obj = self.current_with()?;
                let a = self.eval_args(args)?;
                return self.call_member(&obj, name, &a);
            }
        }
        self.eval(e)
    }

    fn eval_args(&self, args: &[Arg]) -> Result<Vec<Value>> {
        args.iter()
            .map(|a| match a {
                // An omitted argument is `Empty`, which is what a callee that
                // checks `IsEmpty` is looking for.
                None => Ok(Value::Empty),
                Some(e) => self.eval(e),
            })
            .collect()
    }

    /// Evaluates arguments for a call, keeping `ByRef` ones aliased.
    ///
    /// A bare variable passed to a `ByRef` parameter shares its cell, so the
    /// callee writing to the parameter writes to the caller's variable. Any
    /// other expression has nowhere to write back to and gets a fresh cell,
    /// which is also what the real engine does.
    fn eval_arg_slots(&self, p: &Proc, args: &[Arg]) -> Result<Vec<Slot>> {
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let by_val = p.params.get(i).is_some_and(|param| param.by_val);
            match a {
                None => out.push(slot(Value::Empty)),
                Some(Expr::Ident(name)) if !by_val => match self.lookup(name) {
                    Some(cell) => out.push(cell),
                    None => {
                        let v = self.eval_ident(name)?;
                        out.push(slot(v));
                    }
                },
                Some(e) => out.push(slot(self.eval(e)?)),
            }
        }
        Ok(out)
    }

    // -- members -----------------------------------------------------------

    fn get_member(&self, obj: &Value, name: &str, args: &[Value]) -> Result<Value> {
        match obj {
            // A procedure reference reached with `""` as the member is the
            // reference being called: `f 7` after `Set f = GetRef("Handler")`.
            Value::Proc(_) if name.is_empty() => self.call_proc_value(obj, args),
            Value::Object(o) => o.get(name, args),
            Value::Instance(inst) => self.get_instance_member(inst, name, args),
            Value::Nothing => Err(Error::object_variable_not_set()),
            // Reaching for a member of a non-object.
            _ => Err(Error::object_required()),
        }
    }

    /// Calls whatever a `GetRef` reference points at.
    fn call_proc_value(&self, v: &Value, args: &[Value]) -> Result<Value> {
        let Value::Proc(p) = v else {
            return Err(Error::object_required());
        };
        let slots: Vec<Slot> = args.iter().map(|a| slot(a.clone())).collect();
        self.invoke(p, &slots, self.current_me())
    }

    fn call_member(&self, obj: &Value, name: &str, args: &[Value]) -> Result<Value> {
        match obj {
            Value::Object(o) => o.call(name, args),
            Value::Instance(inst) => self.get_instance_member(inst, name, args),
            Value::Nothing => Err(Error::object_variable_not_set()),
            _ => Err(Error::object_required()),
        }
    }

    fn get_instance_member(
        &self,
        inst: &Rc<Instance>,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        if let Some(p) = find_member(&inst.def, name, |p| {
            p.property != Some(PropKind::Let) && p.property != Some(PropKind::Set)
        }) {
            let slots: Vec<Slot> = args.iter().map(|a| slot(a.clone())).collect();
            return self.invoke(&p, &slots, Some(inst.clone()));
        }
        // No name at all: the instance is being used where a value is wanted,
        // or called outright — `Set d = (new DropTarget)(a, b, c)`. What runs
        // is whichever member the class marked `Public Default`.
        if name.is_empty()
            && let Some(p) = inst.def.members.iter().find(|p| p.is_default)
        {
            let slots: Vec<Slot> = args.iter().map(|a| slot(a.clone())).collect();
            return self.invoke(p, &slots, Some(inst.clone()));
        }
        if let Some(cell) = inst.field(name) {
            let v = read(&cell);
            if args.is_empty() {
                return Ok(v);
            }
            if let Value::Array(a) = v {
                let subs = ints(args)?;
                let r = a.borrow().get(&subs);
                return r;
            }
        }
        Err(Error::no_such_member(name))
    }

    /// `New Foo`.
    fn construct(&self, name: &str) -> Result<Value> {
        let def = self
            .classes
            .borrow()
            .get(fold(name).as_str())
            .cloned()
            .ok_or_else(|| Error::new(500, format!("class not defined: '{name}'")))?;

        let inst = Rc::new(Instance::new(def.clone()));
        {
            let mut fields = inst.fields.borrow_mut();
            for f in &def.fields {
                let v = match &f.bounds {
                    None => Value::Empty,
                    Some(b) => {
                        let bounds = self.eval_bounds(b)?;
                        Value::Array(Rc::new(RefCell::new(Array::new(bounds)?)))
                    }
                };
                fields.declare(&f.name, v);
            }
        }

        // `Class_Initialize` is the constructor. It takes no arguments, which
        // is why VBScript classes so often have an `Init` method as well.
        if let Some(p) = find_member(&def, "Class_Initialize", |_| true) {
            self.invoke(&p, &[], Some(inst.clone()))?;
        }
        Ok(Value::Instance(inst))
    }

    // -- calling -----------------------------------------------------------

    /// Runs a procedure.
    fn invoke(&self, p: &Rc<Proc>, args: &[Slot], me: Option<Rc<Instance>>) -> Result<Value> {
        // Too many arguments is an error; too few is not. VBScript leaves the
        // missing ones `Empty`, and tables call handlers with fewer arguments
        // than declared all the time.
        if args.len() > p.params.len() {
            return Err(Error::wrong_argument_count(&p.name));
        }

        {
            let mut d = self.depth.borrow_mut();
            *d += 1;
            if *d > MAX_CALL_DEPTH {
                *d -= 1;
                return Err(Error::out_of_stack());
            }
        }

        // The rule the procedure was written under, not the one in force where
        // it is called from. A handler defined in `core.vbs` keeps `core.vbs`'s
        // rule even when the table that calls it never said `Option Explicit`,
        // and the other way round.
        let _rule = self.with_option_explicit(p.option_explicit);

        let mut frame = Frame::new();
        frame.me = me;
        for (i, param) in p.params.iter().enumerate() {
            let cell = match args.get(i) {
                // `ByVal` copies into a cell of its own; `ByRef` aliases.
                Some(a) if param.by_val => slot(read(a)),
                Some(a) => a.clone(),
                None => slot(Value::Empty),
            };
            frame.locals.set_slot(&param.name, cell);
        }
        if p.is_function {
            // A function returns whatever it assigned to its own name, and
            // `Empty` if it assigned nothing.
            frame.result = Some(frame.locals.declare(&p.name, Value::Empty));
        }

        self.frames.borrow_mut().push(frame);
        // A procedure's own `Dim`s are hoisted to the top of the procedure,
        // exactly as the module's are to the top of the module.
        self.hoist_declarations(&p.body, true);
        self.hoist_declarations(&p.body, false);
        let outcome = self.run_block(&p.body);
        let frame = self.frames.borrow_mut().pop().expect("frame stack");
        *self.depth.borrow_mut() -= 1;

        match outcome {
            Ok(()) => {}
            Err(e) => match e.as_control() {
                // The `Exit` that belongs to this procedure ends it normally.
                Some(Control::ExitSub | Control::ExitFunction | Control::ExitProperty) => {}
                // A stray `Exit For` outside a loop: the real engine rejects it
                // at compile time. Treat it as ending the procedure rather than
                // letting it escape and end the caller's loop.
                Some(Control::ExitFor | Control::ExitDo) => {}
                None => return Err(e.at_line(p.line)),
            },
        }

        Ok(frame.result.map(|r| read(&r)).unwrap_or(Value::Empty))
    }

    // -- scopes ------------------------------------------------------------

    fn current_me(&self) -> Option<Rc<Instance>> {
        self.frames.borrow().last().and_then(|f| f.me.clone())
    }

    fn current_with(&self) -> Result<Value> {
        self.with_stack
            .borrow()
            .last()
            .cloned()
            .ok_or_else(|| Error::new(500, "'.' used outside a With block"))
    }

    /// Finds a variable: locals, then the instance's fields, then globals.
    fn lookup(&self, name: &str) -> Option<Slot> {
        if let Some(f) = self.frames.borrow().last() {
            if let Some(s) = f.locals.get(name) {
                return Some(s.clone());
            }
            if let Some(me) = &f.me
                && let Some(s) = me.field(name)
            {
                return Some(s);
            }
        }
        self.globals.borrow().get(name).cloned()
    }

    /// Declares in the innermost scope.
    fn declare(&self, name: &str, v: Value) -> Slot {
        match self.frames.borrow_mut().last_mut() {
            Some(f) => f.locals.declare(name, v),
            None => self.globals.borrow_mut().declare(name, v),
        }
    }

    fn lookup_or_declare(&self, name: &str) -> Slot {
        self.lookup(name)
            .unwrap_or_else(|| self.declare(name, Value::Empty))
    }

    /// The same, but honouring `Option Explicit`.
    fn lookup_or_declare_checked(&self, name: &str) -> Result<Slot> {
        if let Some(s) = self.lookup(name) {
            return Ok(s);
        }
        if *self.option_explicit.borrow() {
            return Err(Error::undefined_variable(name));
        }
        Ok(self.declare(name, Value::Empty))
    }

    fn resume_next(&self) -> bool {
        self.frames
            .borrow()
            .last()
            .map(|f| f.resume_next)
            .unwrap_or_else(|| *self.global_resume_next.borrow())
    }

    fn set_resume_next(&self, on: bool) {
        match self.frames.borrow_mut().last_mut() {
            Some(f) => f.resume_next = on,
            None => *self.global_resume_next.borrow_mut() = on,
        }
    }

    // -- the Err object ----------------------------------------------------

    fn set_err(&self, e: &Error) {
        let mut err = self.err.borrow_mut();
        err.number = e.number;
        err.description = e.description.clone();
        err.source = e.source.clone();
    }

    fn clear_err(&self) {
        *self.err.borrow_mut() = ErrState::default();
    }

    /// Turns an object into the value it stands for, for a plain `=`
    /// assignment. Non-objects pass through untouched.
    fn devalue(&self, v: Value) -> Result<Value> {
        match &v {
            Value::Object(o) => o.default_value(),
            // A script class with no default member cannot be assigned without
            // `Set`, and saying so is more useful than storing something odd.
            Value::Instance(i) => Err(Error::new(
                438,
                format!(
                    "'{}' has no default property; did you mean 'Set'?",
                    i.def.name
                ),
            )),
            _ => Ok(v),
        }
    }

    /// A name the script used and never declared, from the host.
    ///
    /// **Not cached**, and that is the point. Some host globals mean something
    /// different in every handler — `Me` is the part whose handler is running,
    /// `ActiveBall` is the ball that caused the event — and a cache here
    /// freezes them at whatever they were the first time anything asked. The
    /// symptom is not an error: the script gets a perfectly good object that is
    /// the wrong one, and `core.vbs` ends up wiring the table's own timer to
    /// itself instead of to the part that owns it.
    ///
    /// Caching belongs to the host, which knows which of its answers are
    /// stable. Ours does exactly that.
    fn host_global(&self, name: &str) -> Option<Value> {
        self.host.global(name)
    }
}

/// Restores `Option Explicit` however the block it guards ends — including
/// when the script raises, which is the case that matters.
struct OptionExplicitGuard<'a> {
    interp: &'a Interpreter,
    was: bool,
}

impl Drop for OptionExplicitGuard<'_> {
    fn drop(&mut self) {
        *self.interp.option_explicit.borrow_mut() = self.was;
    }
}

/// Puts the offending source into the message of an error that came out of
/// `Execute`.
///
/// Without this the report is "line 1: expected a name", where line 1 is the
/// first line of a string the script built at run time — which appears nowhere
/// in any file and leaves nothing to search for. Tables generate their event
/// handlers this way, so the case is common enough to be worth the noise.
fn blame_generated(e: Error, src: &str) -> Error {
    if e.number != 1002 {
        return e;
    }
    let snippet: String = src.chars().take(120).collect();
    let ellipsis = if src.chars().count() > 120 { "..." } else { "" };
    Error::syntax(
        format!("{} — in Execute of: {snippet}{ellipsis}", e.description),
        e.line.unwrap_or(0),
    )
}

/// The subscripts of an array access.
fn ints(vals: &[Value]) -> Result<Vec<i32>> {
    vals.iter().map(Value::to_int).collect()
}

/// Finds a class member by name, among those the filter accepts.
fn find_member(def: &ClassDef, name: &str, accept: impl Fn(&Proc) -> bool) -> Option<Rc<Proc>> {
    def.members
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name) && accept(p))
        .cloned()
}

// ---------------------------------------------------------------------------
// The builtins that need the interpreter
//
// Most of the standard library is pure and lives in `crate::builtins`. What is
// here is what cannot be: anything that reads or writes interpreter state, runs
// more script, or talks to the host.
// ---------------------------------------------------------------------------

impl Interpreter {
    /// A builtin used as a bare name, with no arguments.
    fn builtin_value(&self, name: &str) -> Result<Option<Value>> {
        match &*name.to_ascii_lowercase() {
            // `Err` is an object, not a function, and it is read far more often
            // than anything else in this list.
            "err" => Ok(Some(Value::Object(Rc::new(ErrObject(self.err.clone()))))),
            "timer" => Ok(Some(Value::from_number(self.host.seconds()))),
            "rnd" => Ok(Some(Value::Double(self.next_random(None)))),
            _ => {
                if let Some(v) = builtins::constant(name) {
                    return Ok(Some(v));
                }
                // A builtin written with no arguments and no parentheses.
                // `Randomize` on a line of its own is the common one, and it
                // is one of the builtins that needs interpreter state, so the
                // lookup has to go through the same door a call does.
                self.call_builtin(name, &[])
            }
        }
    }

    /// A builtin called with arguments.
    ///
    /// Returns `None` when there is no builtin of that name, so the caller can
    /// carry on looking. That ordering matters: a script's own procedure wins
    /// over a builtin of the same name, which is how tables override `MsgBox`.
    fn call_builtin(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        let lower = name.to_ascii_lowercase();
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Empty);

        match &*lower {
            "rnd" => {
                let seed = match args.first() {
                    None | Some(Value::Empty) => None,
                    Some(v) => Some(v.to_number()?),
                };
                Ok(Some(Value::Double(self.next_random(seed))))
            }
            "randomize" => {
                // With an argument, seed from it; without, the real engine uses
                // the system timer. Either way the sequence stays reproducible
                // for a given seed, which is what makes a bug in a table's
                // logic reproducible too.
                let seed = match args.first() {
                    None | Some(Value::Empty) => self.host.seconds(),
                    Some(v) => v.to_number()?,
                };
                *self.rng.borrow_mut() = (seed.to_bits() as u32) | 1;
                Ok(Some(Value::Empty))
            }
            "timer" => Ok(Some(Value::from_number(self.host.seconds()))),

            // The locale, which a great many tables set on their first line.
            //
            // They do it for one reason: VBScript formats and parses numbers
            // the way the machine's regional settings say, so on a German
            // Windows `CDbl("1.5")` reads a thousands separator and a table's
            // own `"0.5"` stops meaning a half. `SetLocale 1033` forces US
            // English and makes the script's numbers mean what its author
            // typed.
            //
            // Everything here is already invariant — numbers are parsed and
            // printed one way, the way US English does it — so this is exactly
            // the state those tables are asking for and there is nothing to
            // change. What matters is that it *answers*: without it the call
            // is an undefined procedure, and since it sits on the script's
            // first line the rest of the table never runs at all. The value is
            // kept so `GetLocale` can say what it was given, and the old one
            // is returned because that is what `SetLocale` evaluates to.
            "setlocale" => {
                let was = self.locale.get();
                let next = match arg(0) {
                    // `SetLocale(0)` means "back to the system default", and
                    // ours is the only one there is.
                    Value::Empty => US_ENGLISH,
                    v => match v.to_number() {
                        Ok(n) if n as i32 == 0 => US_ENGLISH,
                        Ok(n) => n as i32,
                        // A name rather than a number: `SetLocale("en-us")` is
                        // legal. Nothing downstream reads it, so the point is
                        // only not to fail.
                        Err(_) => US_ENGLISH,
                    },
                };
                self.locale.set(next);
                Ok(Some(Value::Long(was)))
            }
            "getlocale" => Ok(Some(Value::Long(self.locale.get()))),

            "msgbox" => {
                // Not a dialog: a browser has nowhere to put one and nobody to
                // dismiss it. Tables use `MsgBox` for real diagnostics —
                // a missing ROM, a bad option — so it goes to the host, which
                // decides where a person will see it.
                let text = arg(0).to_str().unwrap_or_else(|_| Rc::from(""));
                self.host.message(&text);
                // The button the user "pressed". `vbOK`.
                Ok(Some(Value::Long(1)))
            }
            "inputbox" => {
                // There is nobody to type into it. A browser has nowhere to
                // put a modal prompt and the script is not waiting for a
                // person; what it is waiting for is a value it can carry on
                // with, so it gets the default it offered. `core.vbs` asks
                // this way for a volume level and passes the current one as
                // the default, which makes the answer "leave it alone" —
                // exactly what a cancelled dialog should mean.
                let prompt = arg(0).to_str().unwrap_or_else(|_| Rc::from(""));
                self.host.message(&prompt);
                Ok(Some(match args.get(2) {
                    Some(v) => Value::Str(v.to_str()?),
                    None => Value::str(""),
                }))
            }
            "createobject" | "getobject" => {
                let id = arg(0).to_str()?;
                self.host.create_object(&id).map(Some)
            }

            "getref" => {
                let target = arg(0).to_str()?;
                let p = self
                    .lookup_proc(&target)
                    .ok_or_else(|| Error::undefined_procedure(&target))?;
                Ok(Some(Value::Proc(p)))
            }

            // `Eval` evaluates an expression; `Execute` runs statements. Both
            // re-enter the parser, which is why they live here.
            "eval" => {
                let src = arg(0).to_str()?;
                let e = parser::parse_expression(&src)?;
                let _relaxed = self.without_option_explicit();
                self.eval(&e).map(Some)
            }
            "execute" => {
                let src = arg(0).to_str()?;
                let program = parser::parse(&src).map_err(|e| blame_generated(e, &src))?;
                let _relaxed = self.without_option_explicit();
                self.hoist(&program.body);
                // Blamed on the way out: the line number belongs to *this*
                // text, and nothing further up the stack has it to look at.
                self.run_block(&program.body)
                    .map_err(|e| e.blame("in an executed script", &src))?;
                Ok(Some(Value::Empty))
            }
            "executeglobal" => {
                let src = arg(0).to_str()?;
                // The difference from `Execute` is the scope it runs in, and it
                // is the reason `ExecuteGlobal` exists: Visual Pinball's core
                // script builds event handlers as text and needs them to land
                // in the script's own scope, not in the caller's frame.
                let saved = std::mem::take(&mut *self.frames.borrow_mut());
                let _relaxed = self.without_option_explicit();
                let program = parser::parse(&src).map_err(|e| blame_generated(e, &src));
                let r = program.and_then(|program| {
                    self.hoist(&program.body);
                    self.run_block(&program.body)
                });
                *self.frames.borrow_mut() = saved;
                // The libraries come in through here — `LoadVPM` is
                // `ExecuteGlobal GetTextFile("sega.vbs")` — so an error in
                // `core.vbs` is an error in a text only this frame can still
                // see. Naming it here is what stops "line 1972" from being a
                // number nobody can place.
                r.map_err(|e| e.blame("in a loaded library", &src))?;
                Ok(Some(Value::Empty))
            }

            _ => match builtins::call(name, args) {
                Some(r) => r.map(Some),
                None => Ok(None),
            },
        }
    }

    /// Sets `Option Explicit` for as long as the guard lives.
    fn with_option_explicit(&self, on: bool) -> OptionExplicitGuard<'_> {
        let was = std::mem::replace(&mut *self.option_explicit.borrow_mut(), on);
        OptionExplicitGuard { interp: self, was }
    }

    /// Turns `Option Explicit` off for as long as the guard lives.
    ///
    /// `Eval` and `Execute` compile a **separate** unit, and `Option Explicit`
    /// is a property of the unit it appears in — so code run through them may
    /// use a name nobody declared. That is not a footnote: Visual Pinball's
    /// `core.vbs` opens with `Option Explicit` and then asks
    /// `If IsEmpty(Eval("BallSize"))` to find out whether the host defined
    /// something. Under the outer rule that is error 500 and the library will
    /// not load at all.
    fn without_option_explicit(&self) -> OptionExplicitGuard<'_> {
        let was = *self.option_explicit.borrow();
        *self.option_explicit.borrow_mut() = false;
        OptionExplicitGuard { interp: self, was }
    }

    /// `Rnd`, with VBScript's argument rules.
    ///
    /// `Rnd` and `Rnd(x)` for positive `x` give the next number; `Rnd(0)`
    /// repeats the last one; a negative argument restarts the sequence from a
    /// seed derived from it. Tables use `Rnd(0)` to look at what they just got
    /// without disturbing the sequence.
    ///
    /// The generator is ours and not the original's, so a table that leans on
    /// the exact sequence will differ. That is unavoidable — the original's is
    /// undocumented — and it does not matter: what tables need from `Rnd` is
    /// that it be random and repeatable from a seed, and both hold.
    fn next_random(&self, arg: Option<f64>) -> f64 {
        match arg {
            Some(0.0) => return *self.last_rnd.borrow(),
            Some(x) if x < 0.0 => *self.rng.borrow_mut() = (x.to_bits() as u32) | 1,
            _ => {}
        }
        let mut s = self.rng.borrow_mut();
        // xorshift32: cheap, and with a period far longer than a game.
        *s ^= *s << 13;
        *s ^= *s >> 17;
        *s ^= *s << 5;
        // Scaled into [0, 1), which is what `Rnd` promises.
        let r = f64::from(*s) / f64::from(u32::MAX) * (1.0 - f64::EPSILON);
        *self.last_rnd.borrow_mut() = r;
        r
    }
}
