//! Loads a `.vpx`'s script into the interpreter and reports how far it gets.
//!
//! ```text
//! cargo run --release -p vpw-vbscript --example run_table -- table.vpx [lib.vbs ...]
//! ```
//!
//! A real table's script is not self-contained: it opens by pulling in
//! `core.vbs`, the three-thousand-line library every table shares, and often
//! `controller.vbs` as well. Pass those first or the table's own script will
//! stop at the first name it expects the library to have declared.
//!
//! Parsing a script proves the grammar; running it proves everything else. The
//! host here answers every name with a stub object that accepts any property
//! and any method, so what this measures is the **language**, not the binding:
//! anything that fails is the interpreter's fault, not a missing flipper.

use std::cell::RefCell;
use std::rc::Rc;

use vpw_vbscript::error::Result;
use vpw_vbscript::interp::Interpreter;
use vpw_vbscript::object::{Host, Object};
use vpw_vbscript::value::Value;

/// An object that says yes to everything.
///
/// It has to know its own name, and that is not a detail. Tables build their
/// event handlers as text — `Execute "Sub " & obj.Name & "_Timer : ..."` — so a
/// stub that answers `.Name` with a placeholder generates `Sub 0_Timer`, which
/// is not a legal name, and the failure surfaces as a syntax error in code that
/// exists in no file.
struct Anything(String);

impl Object for Anything {
    fn type_name(&self) -> &'static str {
        "Anything"
    }
    fn get(&self, name: &str, _args: &[Value]) -> Result<Value> {
        if name.eq_ignore_ascii_case("name") {
            return Ok(Value::str(&self.0));
        }
        Ok(Value::Object(Rc::new(Anything(format!(
            "{}_{name}",
            self.0
        )))))
    }
    fn set(&self, _n: &str, _a: &[Value], _v: Value, _r: bool) -> Result<()> {
        Ok(())
    }
    fn default_value(&self) -> Result<Value> {
        Ok(Value::Long(0))
    }
    fn enumerate(&self) -> Option<Vec<Value>> {
        Some(Vec::new())
    }
}

struct StubHost {
    messages: RefCell<Vec<String>>,
}

impl Host for StubHost {
    fn global(&self, name: &str) -> Option<Value> {
        Some(Value::Object(Rc::new(Anything(name.to_string()))))
    }
    fn create_object(&self, prog_id: &str) -> Result<Value> {
        Ok(Value::Object(Rc::new(Anything(prog_id.replace('.', "_")))))
    }
    fn message(&self, text: &str) {
        self.messages.borrow_mut().push(text.to_string());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: run_table <table.vpx> [lib.vbs ...]");
    let libs: Vec<String> = args.collect();
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let code = vpx.gamedata.code.string;

    println!(
        "table   {}",
        vpx.info.table_name.clone().unwrap_or_default()
    );
    println!("script  {} lines", code.lines().count());

    let host = Rc::new(StubHost {
        messages: RefCell::new(Vec::new()),
    });
    let interp = Interpreter::new(host.clone());

    for lib in &libs {
        let src = String::from_utf8_lossy(
            &std::fs::read(lib).unwrap_or_else(|e| panic!("could not read {lib}: {e}")),
        )
        .into_owned();
        match interp.load(&src) {
            Ok(()) => println!("library {lib}: ok ({} lines)", src.lines().count()),
            Err(e) => {
                println!("library {lib}: FAILED: {e}");
                if let Some(line) = e.line {
                    for (n, text) in src.lines().enumerate() {
                        let n = n as u32 + 1;
                        if n + 2 >= line && n <= line + 2 {
                            let mark = if n == line { ">>" } else { "  " };
                            println!("{mark}{n:>6}: {}", text.trim_end());
                        }
                    }
                }
                std::process::exit(1);
            }
        }
    }

    match interp.load(&code) {
        Ok(()) => println!("load    ok"),
        Err(e) => {
            println!("LOAD FAILED: {e}");
            if let Some(line) = e.line {
                for (n, text) in code.lines().enumerate() {
                    let n = n as u32 + 1;
                    if n + 2 >= line && n <= line + 2 {
                        let mark = if n == line { ">>" } else { "  " };
                        println!("{mark}{n:>6}: {}", text.trim_end());
                    }
                }
            }
            std::process::exit(1);
        }
    }

    // The handlers a player would actually call. Running them is the real
    // test: `Table1_Init` is where a table sets its whole game up.
    let mut ok = 0;
    let mut failed = Vec::new();
    for name in ["Table1_Init", "Table1_KeyDown", "Table1_KeyUp"] {
        if !interp.has_proc(name) {
            continue;
        }
        // `KeyDown` takes a key code; the plunger key is as good as any.
        let args = if name.ends_with("Down") || name.ends_with("Up") {
            vec![Value::Long(4)]
        } else {
            Vec::new()
        };
        match interp.call(name, &args) {
            Ok(_) => {
                ok += 1;
                println!("call    {name}: ok");
            }
            Err(e) => {
                failed.push(name);
                println!("call    {name}: {e}");
            }
        }
    }
    println!("handlers ran  {ok} ok, {} failed", failed.len());

    let messages = host.messages.borrow();
    if !messages.is_empty() {
        println!("MsgBox:");
        for m in messages.iter().take(5) {
            println!("  {m}");
        }
    }
}
