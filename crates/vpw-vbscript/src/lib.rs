//! VBScript, enough of it to run a pinball table.
//!
//! Every VPX table is, functionally, a VBScript program. Without an engine for
//! it there is no game: no bumper that scores, no light that turns on, no
//! kicker that ever lets the ball go. This is that engine.
//!
//! # Why our own, and not Wine's
//!
//! Visual Pinball does not implement VBScript either. On Windows it uses the
//! system's `IActiveScript`; for the standalone builds it links `libwinevbs`,
//! which is Wine's engine packaged as a library. `docs/porting-plan.md` §3.1
//! lays out the three ways to get one, and this is the second: our own
//! interpreter in Rust.
//!
//! The deciding argument was the boundary rather than the language. A table's
//! script crosses into the table's objects constantly — `LeftFlipper.RotateToEnd`,
//! `Bumper1.TimerEnabled = True` — and both of the other options put a foreign
//! calling convention on that path: a C bridge for the Wine engine, the JS↔wasm
//! boundary for a transpiler. Here it is [`object::Object`], a Rust trait with
//! three methods.
//!
//! # What it runs
//!
//! Everything Visual Pinball's own scripts use, which is a bounded subset:
//! procedures, classes with properties, arrays, `With`, `Select Case`, all the
//! loop forms, `On Error Resume Next` with a real `Err` object, `Eval`,
//! `Execute`/`ExecuteGlobal`, and `GetRef`. The standard library is in
//! [`builtins`]; the parts that need interpreter state — `Rnd`, `Timer`,
//! `Eval`, `CreateObject`, `MsgBox` — live in [`interp`].
//!
//! It is checked against the real thing rather than against my idea of it. The
//! tests parse all seventy scripts Visual Pinball ships, `core.vbs` included,
//! and load and run published tables. Two of the bugs that found would never
//! have turned up otherwise: a script stored with bare carriage returns as line
//! endings, and `SetLamp (118), 0` — a call whose first argument is
//! parenthesised, which reads as an index if you are not careful.
//!
//! # What it does not run
//!
//! No `Date` subtype, so no `Now`, `DateAdd` or friends. No regular
//! expressions (`RegExp`), no `Scripting.Dictionary` — a host can supply both
//! as objects if a table turns out to need them. `Integer` and `Long` are one
//! type, so `TypeName(1)` answers `"Long"`. And there is no `GoTo` beyond
//! `On Error GoTo 0`, which is all VBScript has anyway.
//!
//! # Where to start reading
//!
//! [`value`] first: VBScript's conversion rules decide the behaviour of
//! everything above them, and they are where a subtle wrong answer comes from.
//! Then [`ops`], then [`interp`].

pub mod ast;
pub mod builtins;
pub mod dates;
pub mod error;
pub mod instance;
pub mod interp;
pub mod lexer;
pub mod object;
pub mod ops;
pub mod parser;
pub mod scripting;
pub mod value;

pub use error::{Error, Result};
pub use interp::Interpreter;
pub use object::{Host, Object};
pub use value::Value;
