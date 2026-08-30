//! The language, exercised end to end.
//!
//! Each test runs a snippet and reads a variable back out. That is deliberate:
//! it is what a table does — the host sets things up, the script runs, and the
//! host reads the result — so a test written this way fails for the same
//! reasons a table would.
//!
//! The cases were chosen from what Visual Pinball's own scripts actually
//! contain. Where VBScript does something surprising, the test says so and says
//! what it would break.

use std::cell::RefCell;
use std::rc::Rc;

use vpw_vbscript::error::Result;
use vpw_vbscript::interp::Interpreter;
use vpw_vbscript::object::{Host, Object};
use vpw_vbscript::value::Value;

/// Runs a script and reads back one global.
fn run(src: &str, name: &str) -> Value {
    let i = Interpreter::default();
    match i.load(src) {
        Ok(()) => {}
        Err(e) => panic!("script failed: {e}\n--- source ---\n{src}"),
    }
    i.get_global(name)
        .unwrap_or_else(|| panic!("'{name}' was never defined by:\n{src}"))
}

/// The same, expecting the script to fail.
fn fails(src: &str) -> vpw_vbscript::error::Error {
    let i = Interpreter::default();
    match i.load(src) {
        Ok(()) => panic!("expected this to fail:\n{src}"),
        Err(e) => e,
    }
}

fn num(src: &str, name: &str) -> f64 {
    run(src, name).to_number().expect("expected a number")
}

fn text(src: &str, name: &str) -> String {
    run(src, name)
        .to_str()
        .expect("expected a string")
        .to_string()
}

fn truth(src: &str, name: &str) -> bool {
    run(src, name).to_bool().expect("expected a boolean")
}

// ------------------------------------------------------------ the basics ---

#[test]
fn assignment_and_arithmetic() {
    assert_eq!(num("a = 1 + 2 * 3", "a"), 7.0);
    assert_eq!(num("a = (1 + 2) * 3", "a"), 9.0);
    assert_eq!(num("a = 2 ^ 10", "a"), 1024.0);
    assert_eq!(num("a = 7 \\ 2", "a"), 3.0);
    assert_eq!(num("a = 7 Mod 3", "a"), 1.0);
}

#[test]
fn exponentiation_is_left_associative() {
    // VBScript is unusual here: `2 ^ 3 ^ 2` is 64, not 512.
    assert_eq!(num("a = 2 ^ 3 ^ 2", "a"), 64.0);
}

#[test]
fn concatenation_binds_looser_than_addition() {
    // So `"n=" & a + b` concatenates the sum rather than adding to the label.
    assert_eq!(text("a = \"n=\" & 1 + 2", "a"), "n=3");
}

#[test]
fn comparison_binds_tighter_than_and() {
    // The reason tables write `If a = 1 And b = 2 Then` without parentheses.
    assert!(truth("x = 1 : y = 2 : a = x = 1 And y = 2", "a"));
}

#[test]
fn names_are_case_insensitive() {
    assert_eq!(num("SolFlipper = 3 : a = solflipper", "a"), 3.0);
}

#[test]
fn a_line_can_be_continued() {
    assert_eq!(num("a = 1 + _\n    2", "a"), 3.0);
}

#[test]
fn several_statements_fit_on_a_line() {
    assert_eq!(num("a = 1 : a = a + 1 : a = a * 10", "a"), 20.0);
}

// -------------------------------------------------------------- branching ---

#[test]
fn if_elseif_else() {
    let src = "
        x = 2
        If x = 1 Then
            a = \"one\"
        ElseIf x = 2 Then
            a = \"two\"
        Else
            a = \"other\"
        End If";
    assert_eq!(text(src, "a"), "two");
}

#[test]
fn a_single_line_if_needs_no_end_if() {
    assert_eq!(num("a = 0 : If True Then a = 1", "a"), 1.0);
    assert_eq!(num("a = 0 : If False Then a = 1", "a"), 0.0);
    assert_eq!(num("If False Then a = 1 Else a = 2", "a"), 2.0);
}

#[test]
fn a_single_line_if_swallows_the_rest_of_the_line() {
    // This is the one that catches people: both statements are conditional,
    // even though the second looks like it stands on its own.
    assert_eq!(num("a = 0 : b = 0 : If False Then a = 1 : b = 1", "b"), 0.0);
    assert_eq!(num("a = 0 : b = 0 : If True Then a = 1 : b = 1", "b"), 1.0);
}

#[test]
fn select_case() {
    let src = "
        x = 3
        Select Case x
            Case 1, 2
                a = \"low\"
            Case 3
                a = \"three\"
            Case Else
                a = \"high\"
        End Select";
    assert_eq!(text(src, "a"), "three");
}

#[test]
fn select_case_falls_through_to_case_else() {
    let src = "
        Select Case 99
            Case 1
                a = \"one\"
            Case Else
                a = \"other\"
        End Select";
    assert_eq!(text(src, "a"), "other");
}

#[test]
fn select_case_compares_with_vbscript_rules() {
    // `"10"` and `10` are the same case, because a numeric string compares as
    // a number.
    let src = "
        Select Case \"10\"
            Case 10
                a = 1
            Case Else
                a = 2
        End Select";
    assert_eq!(num(src, "a"), 1.0);
}

// ----------------------------------------------------------------- loops ---

#[test]
fn for_next() {
    assert_eq!(num("a = 0\nFor i = 1 To 5\n a = a + i\nNext", "a"), 15.0);
}

#[test]
fn for_with_a_step() {
    assert_eq!(
        num("a = 0\nFor i = 10 To 1 Step -2\n a = a + 1\nNext", "a"),
        5.0
    );
    assert_eq!(
        num("a = 0\nFor i = 1 To 10 Step 3\n a = a + 1\nNext", "a"),
        4.0
    );
}

#[test]
fn the_limit_is_evaluated_once() {
    // Changing the limit inside the body must not change how long the loop
    // runs. A table that resizes a collection while walking it depends on it.
    let src = "
        n = 3
        a = 0
        For i = 1 To n
            n = 10
            a = a + 1
        Next";
    assert_eq!(num(src, "a"), 3.0);
}

#[test]
fn a_zero_step_is_refused_rather_than_hanging() {
    // The real engine loops forever. A hung browser tab is worse than an error.
    assert_eq!(fails("For i = 1 To 5 Step 0\nNext").number, 5);
}

#[test]
fn exit_for_leaves_the_loop() {
    let src = "
        a = 0
        For i = 1 To 100
            If i > 3 Then Exit For
            a = a + 1
        Next";
    assert_eq!(num(src, "a"), 3.0);
}

#[test]
fn do_while_and_do_until() {
    assert_eq!(num("a = 0\nDo While a < 5\n a = a + 1\nLoop", "a"), 5.0);
    assert_eq!(num("a = 0\nDo Until a >= 5\n a = a + 1\nLoop", "a"), 5.0);
}

#[test]
fn a_condition_at_the_loop_runs_the_body_once() {
    // The whole reason both spellings exist.
    assert_eq!(num("a = 0\nDo\n a = a + 1\nLoop While False", "a"), 1.0);
    assert_eq!(num("a = 0\nDo While False\n a = a + 1\nLoop", "a"), 0.0);
}

#[test]
fn while_wend() {
    assert_eq!(num("a = 0\nWhile a < 3\n a = a + 1\nWend", "a"), 3.0);
}

#[test]
fn exit_do_leaves_the_loop() {
    let src = "
        a = 0
        Do
            a = a + 1
            If a = 4 Then Exit Do
        Loop";
    assert_eq!(num(src, "a"), 4.0);
}

// ------------------------------------------------------------ procedures ---

#[test]
fn a_sub_and_a_function() {
    // `Dim a` at script level, on purpose: an undeclared name assigned inside
    // a procedure is **local** to it, so without this the script would set a
    // local and the global would never appear. It is the same rule that makes
    // a typo inside a sub invisible.
    let src = "
        Dim a
        Function Double(x)
            Double = x * 2
        End Function
        Sub Store(v)
            a = v
        End Sub
        Store Double(21)";
    assert_eq!(num(src, "a"), 42.0);
}

#[test]
fn a_function_that_assigns_nothing_returns_empty() {
    let src = "
        Function F()
        End Function
        a = F()";
    assert!(matches!(run(src, "a"), Value::Empty));
}

#[test]
fn procedures_are_visible_before_they_are_declared() {
    // A table calls helpers defined further down the file all the time.
    let src = "
        a = F()
        Function F()
            F = 7
        End Function";
    assert_eq!(num(src, "a"), 7.0);
}

#[test]
fn arguments_are_by_reference_by_default() {
    // This is how the standard scripts return several values at once.
    let src = "
        Sub Bump(x)
            x = x + 1
        End Sub
        a = 1
        Bump a";
    assert_eq!(num(src, "a"), 2.0);
}

#[test]
fn byval_copies() {
    let src = "
        Sub Bump(ByVal x)
            x = x + 1
        End Sub
        a = 1
        Bump a";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn a_missing_argument_arrives_empty() {
    // Tables call handlers with fewer arguments than declared constantly.
    let src = "
        Function F(a, b)
            F = IsEmpty(b)
        End Function
        a = F(1)";
    assert!(truth(src, "a"));
}

#[test]
fn exit_sub_returns_early() {
    let src = "
        Dim a
        Sub S()
            a = 1
            Exit Sub
            a = 2
        End Sub
        S";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn a_call_with_no_parentheses_is_a_call() {
    let src = "
        Dim a
        Sub S(x)
            a = x
        End Sub
        S 5";
    assert_eq!(num(src, "a"), 5.0);
}

#[test]
fn a_parenthesised_first_argument_is_still_an_argument() {
    // `SetLamp (118), 0` is a call with two arguments, the first of which
    // happens to be parenthesised — not a call with one argument whose result
    // is then indexed. Terminator 2's script is full of this, and read the
    // other way every one of them is a type mismatch.
    let src = "
        Dim a
        Sub SetLamp(n, v)
            a = n * 1000 + v
        End Sub
        SetLamp (118),7";
    assert_eq!(num(src, "a"), 118_007.0);
}

#[test]
fn a_fully_parenthesised_call_still_works() {
    // The reading that has to keep working: one set of parentheses around the
    // whole argument list.
    let src = "
        Dim a
        Sub SetLamp(n, v)
            a = n * 1000 + v
        End Sub
        SetLamp(118, 7)";
    assert_eq!(num(src, "a"), 118_007.0);
}

#[test]
fn indexing_an_array_is_not_confused_with_a_call() {
    let src = "
        Dim arr(3), a
        arr(1) = 5
        a = arr(1)";
    assert_eq!(num(src, "a"), 5.0);
}

#[test]
fn a_script_written_with_bare_carriage_returns_runs() {
    // Terminator 2's script is stored this way. Treating `\r` as whitespace
    // collapses the file onto one line and nothing parses.
    assert_eq!(num("a = 1\rb = 2\ra = a + b", "a"), 3.0);
}

#[test]
fn recursion_works_and_is_bounded() {
    let src = "
        Function Fact(n)
            If n <= 1 Then
                Fact = 1
            Else
                Fact = n * Fact(n - 1)
            End If
        End Function
        a = Fact(6)";
    assert_eq!(num(src, "a"), 720.0);

    // And runaway recursion reports instead of taking the process down.
    let runaway = "
        Sub S()
            S
        End Sub
        S";
    assert_eq!(fails(runaway).number, 28);
}

// ---------------------------------------------------------------- arrays ---

#[test]
fn an_array_has_an_inclusive_upper_bound() {
    // `Dim a(3)` has four elements. The single most common off-by-one when
    // reading VBScript from another language.
    let src = "
        Dim arr(3)
        arr(0) = 10
        arr(3) = 40
        a = arr(0) + arr(3)";
    assert_eq!(num(src, "a"), 50.0);
}

#[test]
fn a_subscript_out_of_range_reports_nine() {
    let src = "
        Dim arr(2)
        a = arr(5)";
    assert_eq!(fails(src).number, 9);
}

#[test]
fn a_two_dimensional_array() {
    let src = "
        Dim g(2, 3)
        g(1, 2) = 7
        a = g(1, 2)";
    assert_eq!(num(src, "a"), 7.0);
}

#[test]
fn redim_and_redim_preserve() {
    let src = "
        Dim arr()
        ReDim arr(2)
        arr(0) = 1
        ReDim Preserve arr(5)
        a = arr(0)";
    assert_eq!(num(src, "a"), 1.0);

    let wiped = "
        Dim arr(2)
        arr(0) = 1
        ReDim arr(5)
        a = IsEmpty(arr(0))";
    assert!(truth(wiped, "a"));
}

#[test]
fn an_array_is_shared_when_passed() {
    // VBScript hands arrays around by reference, so a sub can fill one in.
    let src = "
        Sub Fill(x)
            x(0) = 9
        End Sub
        Dim arr(2)
        Fill arr
        a = arr(0)";
    assert_eq!(num(src, "a"), 9.0);
}

#[test]
fn for_each_over_an_array() {
    let src = "
        Dim arr(2)
        arr(0) = 1 : arr(1) = 2 : arr(2) = 3
        a = 0
        For Each v In arr
            a = a + v
        Next";
    assert_eq!(num(src, "a"), 6.0);
}

// --------------------------------------------------------------- classes ---

#[test]
fn a_class_with_fields_and_methods() {
    let src = "
        Class Counter
            Private mCount
            Private Sub Class_Initialize
                mCount = 0
            End Sub
            Public Sub Bump
                mCount = mCount + 1
            End Sub
            Public Function Value
                Value = mCount
            End Function
        End Class
        Dim c
        Set c = New Counter
        c.Bump
        c.Bump
        a = c.Value";
    assert_eq!(num(src, "a"), 2.0);
}

#[test]
fn properties_get_and_let() {
    let src = "
        Class Box
            Private mV
            Public Property Get Value
                Value = mV * 10
            End Property
            Public Property Let Value(v)
                mV = v
            End Property
        End Class
        Dim b
        Set b = New Box
        b.Value = 4
        a = b.Value";
    assert_eq!(num(src, "a"), 40.0);
}

#[test]
fn two_instances_do_not_share_state() {
    let src = "
        Class C
            Public V
        End Class
        Dim x, y
        Set x = New C
        Set y = New C
        x.V = 1
        y.V = 2
        a = x.V";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn is_compares_references_and_not_contents() {
    let src = "
        Class C
            Public V
        End Class
        Dim x, y, z
        Set x = New C
        Set y = New C
        Set z = x
        same = (z Is x)
        different = (y Is x)
        a = same
        b = different";
    assert!(truth(src, "a"));
    assert!(!truth(src, "b"));
}

#[test]
fn nothing_is_nothing() {
    let src = "
        Dim x
        Set x = Nothing
        a = (x Is Nothing)";
    assert!(truth(src, "a"));
}

// ------------------------------------------------------------------ with ---

#[test]
fn with_shortens_member_access() {
    let src = "
        Class C
            Public X
            Public Y
        End Class
        Dim c
        Set c = New C
        With c
            .X = 3
            .Y = 4
        End With
        a = c.X + c.Y";
    assert_eq!(num(src, "a"), 7.0);
}

#[test]
fn with_blocks_nest() {
    let src = "
        Class C
            Public X
        End Class
        Dim p, q
        Set p = New C
        Set q = New C
        With p
            .X = 1
            With q
                .X = 2
            End With
            .X = .X + 10
        End With
        a = p.X
        b = q.X";
    assert_eq!(num(src, "a"), 11.0);
    assert_eq!(num(src, "b"), 2.0);
}

// ----------------------------------------------------------- error handling ---

#[test]
fn on_error_resume_next_swallows_and_records() {
    let src = "
        On Error Resume Next
        a = 1 / 0
        n = Err.Number
        On Error GoTo 0";
    assert_eq!(num(src, "n"), 11.0);
}

#[test]
fn on_error_goto_zero_restores_the_default() {
    let src = "
        On Error Resume Next
        x = 1 / 0
        On Error GoTo 0
        y = 1 / 0";
    assert_eq!(fails(src).number, 11);
}

#[test]
fn err_clear_resets_it() {
    let src = "
        On Error Resume Next
        x = 1 / 0
        Err.Clear
        a = Err.Number";
    assert_eq!(num(src, "a"), 0.0);
}

#[test]
fn err_raise_is_catchable() {
    let src = "
        On Error Resume Next
        Err.Raise 5
        a = Err.Number";
    assert_eq!(num(src, "a"), 5.0);
}

#[test]
fn err_read_bare_is_its_number() {
    // Visual Pinball's own scripts write `If Err Then`.
    let src = "
        On Error Resume Next
        x = 1 / 0
        If Err Then a = 1 Else a = 0";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn resume_next_does_not_swallow_exit_sub() {
    // `Exit Sub` unwinds on the same channel as an error; if the handler
    // swallowed it, the rest of the sub would run.
    let src = "
        Dim a
        Sub S()
            On Error Resume Next
            a = 1
            Exit Sub
            a = 2
        End Sub
        S";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn a_handler_does_not_leak_into_the_caller() {
    // `On Error Resume Next` is per procedure. If it leaked, an error after
    // the call would be silently swallowed.
    let src = "
        Sub Protected()
            On Error Resume Next
            x = 1 / 0
        End Sub
        Protected
        y = 1 / 0";
    assert_eq!(fails(src).number, 11);
}

// -------------------------------------------------------- option explicit ---

#[test]
fn option_explicit_rejects_an_undeclared_name() {
    let src = "
        Option Explicit
        a = 1";
    assert_eq!(fails(src).number, 500);
}

#[test]
fn option_explicit_accepts_a_declared_one() {
    let src = "
        Option Explicit
        Dim a
        a = 1";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn without_option_explicit_a_typo_is_silently_empty() {
    // The reason a typo in a table is so hard to find.
    assert!(truth("a = IsEmpty(nosuchthing)", "a"));
}

// ----------------------------------------------------- eval and execute ---

#[test]
fn eval_evaluates_an_expression() {
    assert_eq!(num("a = Eval(\"1 + 2\")", "a"), 3.0);
}

#[test]
fn execute_runs_statements() {
    assert_eq!(num("Execute \"a = 5\"", "a"), 5.0);
}

#[test]
fn execute_global_lands_in_the_script_scope() {
    // What Visual Pinball's core script uses to build event handlers as text:
    // a sub writes the source of another sub and needs it to end up in the
    // script's own scope rather than in the caller's frame.
    let src = "
        Dim a
        Sub Build()
            ExecuteGlobal \"Sub Made() : a = 1 : End Sub\"
        End Sub
        Build
        Made";
    assert_eq!(num(src, "a"), 1.0);
}

#[test]
fn getref_makes_a_procedure_a_value() {
    let src = "
        Dim a
        Sub Handler(x)
            a = x
        End Sub
        Dim f
        Set f = GetRef(\"Handler\")
        f 7";
    assert_eq!(num(src, "a"), 7.0);
}

// ------------------------------------------------------------ the host ---

/// A host with one object on it, enough to check the boundary.
struct TestHost {
    flipper: Rc<Flipper>,
    messages: RefCell<Vec<String>>,
}

struct Flipper {
    angle: RefCell<f64>,
    rotated: RefCell<u32>,
}

impl Object for Flipper {
    fn type_name(&self) -> &'static str {
        "Flipper"
    }
    fn get(&self, name: &str, _args: &[Value]) -> Result<Value> {
        match &*name.to_ascii_lowercase() {
            "currentangle" => Ok(Value::Double(*self.angle.borrow())),
            "rotatetoend" => {
                *self.rotated.borrow_mut() += 1;
                Ok(Value::Empty)
            }
            "rotations" => Ok(Value::Long(*self.rotated.borrow() as i32)),
            _ => Err(vpw_vbscript::error::Error::no_such_member(name)),
        }
    }
    fn set(&self, name: &str, _args: &[Value], value: Value, _by_ref: bool) -> Result<()> {
        match &*name.to_ascii_lowercase() {
            "currentangle" => {
                *self.angle.borrow_mut() = value.to_number()?;
                Ok(())
            }
            _ => Err(vpw_vbscript::error::Error::no_such_member(name)),
        }
    }
}

impl Host for TestHost {
    fn global(&self, name: &str) -> Option<Value> {
        if name.eq_ignore_ascii_case("LeftFlipper") {
            return Some(Value::Object(self.flipper.clone()));
        }
        None
    }
    fn message(&self, text: &str) {
        self.messages.borrow_mut().push(text.to_string());
    }
    fn seconds(&self) -> f64 {
        1234.5
    }
}

fn with_host() -> (Interpreter, Rc<TestHost>) {
    let host = Rc::new(TestHost {
        flipper: Rc::new(Flipper {
            angle: RefCell::new(0.0),
            rotated: RefCell::new(0),
        }),
        messages: RefCell::new(Vec::new()),
    });
    (Interpreter::new(host.clone()), host)
}

#[test]
fn a_script_reaches_a_host_object() {
    let (i, host) = with_host();
    i.load("LeftFlipper.RotateToEnd").unwrap();
    assert_eq!(*host.flipper.rotated.borrow(), 1);
}

#[test]
fn a_script_writes_a_host_property() {
    let (i, host) = with_host();
    i.load("LeftFlipper.CurrentAngle = 45").unwrap();
    assert_eq!(*host.flipper.angle.borrow(), 45.0);
}

#[test]
fn a_script_reads_a_host_property() {
    let (i, _host) = with_host();
    i.load("LeftFlipper.CurrentAngle = 12 : a = LeftFlipper.CurrentAngle")
        .unwrap();
    assert_eq!(i.get_global("a").unwrap().to_number().unwrap(), 12.0);
}

#[test]
fn with_works_on_a_host_object() {
    let (i, host) = with_host();
    i.load("With LeftFlipper\n .CurrentAngle = 30\n .RotateToEnd\nEnd With")
        .unwrap();
    assert_eq!(*host.flipper.angle.borrow(), 30.0);
    assert_eq!(*host.flipper.rotated.borrow(), 1);
}

#[test]
fn the_host_delivers_events_by_calling_a_handler() {
    // This is how a table's rules actually run.
    let (i, _host) = with_host();
    i.load("Dim hits\nSub Bumper1_Hit()\n hits = hits + 1\nEnd Sub")
        .unwrap();
    i.call("Bumper1_Hit", &[]).unwrap();
    i.call("Bumper1_Hit", &[]).unwrap();
    assert_eq!(i.get_global("hits").unwrap().to_number().unwrap(), 2.0);
}

#[test]
fn a_handler_a_table_did_not_define_is_not_an_error() {
    // A table only writes the handlers it cares about, and the host should not
    // have to ask first.
    let (i, _host) = with_host();
    i.load("").unwrap();
    assert!(i.call("Bumper9_Hit", &[]).unwrap().is_none());
}

#[test]
fn an_event_handler_receives_its_arguments() {
    let (i, _host) = with_host();
    i.load("Dim last\nSub OnSwitch(n)\n last = n\nEnd Sub")
        .unwrap();
    i.call("OnSwitch", &[Value::Long(17)]).unwrap();
    assert_eq!(i.get_global("last").unwrap().to_number().unwrap(), 17.0);
}

#[test]
fn msgbox_goes_to_the_host() {
    let (i, host) = with_host();
    i.load("MsgBox \"no ROM found\"").unwrap();
    assert_eq!(&host.messages.borrow()[0], "no ROM found");
}

#[test]
fn timer_comes_from_the_host_clock() {
    let (i, _host) = with_host();
    i.load("a = Timer").unwrap();
    assert_eq!(i.get_global("a").unwrap().to_number().unwrap(), 1234.5);
}

#[test]
fn an_unknown_host_name_is_reported() {
    let (i, _host) = with_host();
    let e = i.load("NoSuchTable.Foo = 1").unwrap_err();
    // "Object required": the name resolved to nothing.
    assert_eq!(e.number, 424);
}

// ------------------------------------------------------------- rnd ---

#[test]
fn rnd_is_repeatable_from_a_seed() {
    // Not the original's sequence — that is undocumented — but repeatable,
    // which is what makes a bug in a table's logic reproducible.
    let one = num(
        "Randomize 42\na = 0\nFor i = 1 To 5\n a = a + Rnd\nNext",
        "a",
    );
    let two = num(
        "Randomize 42\na = 0\nFor i = 1 To 5\n a = a + Rnd\nNext",
        "a",
    );
    assert_eq!(one, two);
    assert!(one > 0.0 && one < 5.0);
}

#[test]
fn rnd_zero_repeats_the_last_number() {
    let src = "
        Randomize 1
        x = Rnd
        y = Rnd(0)
        a = (x = y)";
    assert!(truth(src, "a"));
}

/// `Not` binds looser than the comparisons.
///
/// This is the precedence bug that hides best: put `Not` with the other unary
/// operators and every one of these still parses, most still give the right
/// answer, and the one that does not — `Not x Is Nothing` — is the form
/// `core.vbs` uses to dispatch every solenoid callback on a ROM table. There it
/// does not answer wrongly, it raises: `Not <object reference>` is a type
/// error, and the `On Error Resume Next` around it turns that into a table
/// where no flipper, popper or kicker ever fires.
#[test]
fn not_binds_looser_than_the_comparisons() {
    // `Not (1 = 2)` is True. `(Not 1) = 2` would be False, because `Not 1`
    // is -2 and -2 is not 2.
    assert_eq!(num("x = Not 1 = 2", "x"), -1.0);
    assert_eq!(num("x = Not 2 = 2", "x"), 0.0);

    // Looser than `Is`, which is the case that raises rather than answering
    // wrongly.
    assert!(!truth("x = Not Nothing Is Nothing", "x"));

    // Still tighter than `And`: `(Not 0) And 1`, not `Not (0 And 1)`.
    assert_eq!(num("x = Not 0 And 1", "x"), 1.0);

    // And where an operand is genuinely required it is still a prefix operator.
    assert_eq!(num("x = 2 + (Not 0)", "x"), 1.0);
}

/// The shape the bug above was found in, end to end.
#[test]
fn a_reference_can_be_tested_for_nothing_and_then_called() {
    let src = r#"
Dim log
Sub Handler(state)
  log = log & "called:" & state & ";"
End Sub

Dim refs(4), cb, i
For i = 0 To 4
  Set refs(i) = Nothing
Next
Set refs(2) = GetRef("Handler")

For i = 0 To 4
  Set cb = refs(i)
  If Not cb Is Nothing Then cb i
Next
"#;
    assert_eq!(text(src, "log"), "called:2;");
}

#[test]
fn option_explicit_does_not_escape_the_file_it_is_written_in() {
    // It belongs to the unit it appears in. Leaving it on afterwards made it
    // everybody's, and the order is what turns that into a real failure: Visual
    // Pinball's `core.vbs` opens with `Option Explicit` and is loaded first, so
    // every table script after it inherited a rule it never asked for and died
    // at its first undeclared name — reported against the table's own line,
    // which reads as the table being broken.
    let i = Interpreter::default();
    i.load(
        "Option Explicit
Dim declared
declared = 1",
    )
    .expect("the strict unit is fine on its own");
    i.load("undeclared = 2")
        .expect("a later file never said Option Explicit");
    assert_eq!(
        i.get_global("undeclared").unwrap().to_number().unwrap(),
        2.0
    );
}

#[test]
fn a_procedure_keeps_the_rule_of_its_own_file() {
    // Which is what VBScript does by settling this when it compiles the unit:
    // where the call comes from does not enter into it. Both directions matter,
    // because a table and its libraries disagree about this all the time.
    let i = Interpreter::default();
    i.load(
        "Option Explicit
Sub Strict
  typo = 1
End Sub",
    )
    .expect("defining it is fine; the body has not run");
    i.load(
        "Dim GotPast
Sub Relaxed
  alsotypo = 2
  GotPast = 1
End Sub",
    )
    .expect("a file with no rule");

    // Called from a file with no rule, the strict one is still strict.
    let e = i.load("Strict").expect_err("it should have complained");
    assert_eq!(e.number, 500, "{e}");

    // And the other way round: the relaxed one stays relaxed even though the
    // caller is strict.
    i.load(
        "Option Explicit
Relaxed",
    )
    .expect("the callee's own file never asked for this");
    // Past the undeclared name and out the other side. `alsotypo` itself is
    // not worth looking for: an undeclared name inside a procedure is a local
    // of that procedure, which is VBScript's rule and not what is on trial.
    assert_eq!(i.get_global("GotPast").unwrap().to_number().unwrap(), 1.0);
}

/// The statement `cvpmTimer` builds and `Execute`s for every ball a stack
/// kicks out (`core.vbs:707`): a sub call whose target is the **result of a
/// function call**, arguments space-separated, a comment hanging off the end.
/// South Park's SuperVUK release is exactly this string, and a parser that
/// cannot say it leaves the ball standing in the kicker for ever.
#[test]
fn a_sub_call_on_a_function_result_with_bare_arguments() {
    let v = run(
        r#"
Class K
    Public Sub Kick(a, b, c)
        result = a * 100 + b * 10 + c
    End Sub
End Class
Function GetK(x) : Set GetK = x : End Function
Dim k : Set k = New K
Dim result : result = 0
Execute "GetK(k).Kick 1,2,3 ' 0 "
"#,
        "result",
    );
    assert_eq!(v.to_number().unwrap(), 123.0);
}

/// The same statement, but through the `GetRef` variable `core.vbs` actually
/// routes it through (`Dim vpmCreateBall : Set vpmCreateBall =
/// GetRef("vpmDefCreateBall3")`, `core.vbs:2341`). The function's return
/// value must survive the pointer, or `.Kick` lands on Empty and the ball
/// stands in the kicker for ever.
#[test]
fn a_sub_call_through_a_getref_variable_keeps_the_return_value() {
    let v = run(
        r#"
Class K
    Public Sub Kick(a, b, c)
        result = a * 100 + b * 10 + c
    End Sub
End Class
Function GetK(x) : Set GetK = x : End Function
Dim ptr : Set ptr = GetRef("GetK")
Dim k : Set k = New K
Dim result : result = 0
Execute "ptr(k).Kick 4,5,6 ' 0 "
"#,
        "result",
    );
    assert_eq!(v.to_number().unwrap(), 456.0);
}

/// `SetLocale` on the first line, which is where tables put it.
///
/// A user reported that most of the tables they tried failed with "Sub or
/// Function not defined 'Setlocale'". It is the first statement of a great
/// many published tables, so the whole script died on line one and the table
/// came up empty — the worst possible place for a missing builtin.
///
/// Both call forms have to work: as a statement without parentheses, which is
/// how it is nearly always written, and as a function whose value is read.
#[test]
fn a_table_may_set_its_locale_on_the_first_line() {
    assert!(truth(
        "SetLocale 1033
Dim ok : ok = True
",
        "ok"
    ));

    // It answers with the locale it replaced, and `GetLocale` with the one it
    // was given. A table that saves the old one to put it back afterwards —
    // and some do — needs both halves to line up.
    assert_eq!(
        text(
            "Dim was : was = SetLocale(1031)
             Dim now : now = GetLocale()
             Dim both : both = was & \"/\" & now
",
            "both"
        ),
        "1033/1031"
    );

    // Zero is "the system default", which here is the only one there is.
    assert_eq!(
        num(
            "SetLocale 1031
SetLocale 0
Dim n : n = GetLocale()
",
            "n"
        ),
        1033.0
    );

    // And a locale named rather than numbered is legal VBScript. Nothing
    // downstream reads it; the point is that it does not stop the script.
    assert!(truth(
        "SetLocale \"en-gb\"
Dim ok : ok = True
",
        "ok"
    ));
}

/// The engine-version guard at the top of `controller.vbs`.
///
/// `If ScriptEngineMajorVersion < 5 Then MsgBox ...` sits under
/// `On Error Resume Next`, so leaving it undefined does not stop the script.
/// It does something quieter: it sets `Err`, and the very next line's
/// `If Err Then MsgBox "Unable to open " & VBSfile` then blames a file that
/// opened perfectly well.
#[test]
fn the_engine_says_which_version_it_is() {
    assert_eq!(num("v = ScriptEngineMajorVersion", "v"), 5.0);
    assert_eq!(num("v = ScriptEngineMinorVersion", "v"), 8.0);
    assert_eq!(text("v = ScriptEngine", "v"), "VBScript");

    // The shape it is actually written in, error trapping and all: the guard
    // must pass and must leave no error behind it.
    assert!(truth(
        "On Error Resume Next
         If ScriptEngineMajorVersion < 5 Then Err.Raise 5
         Dim ok : ok = (Err.Number = 0)
",
        "ok"
    ));
}

/// `InputBox` answers with the default it was offered.
///
/// There is nobody to type into it. `core.vbs` asks this way for a volume
/// level and passes the current one as the default, so answering with the
/// default means "leave it alone" — which is what a dialog nobody saw should
/// do.
#[test]
fn a_prompt_nobody_can_answer_returns_its_default() {
    assert_eq!(
        text("a = InputBox(\"Enter a volume\", \"Volume\", \"-12\")", "a"),
        "-12"
    );
    assert_eq!(text("a = InputBox(\"Anything?\")", "a"), "");
}

/// An error out of a loaded library names the library, not the table.
///
/// A user reported "the table's script failed: line 1972: Object required".
/// Line 1972 of *what*? The table's own script, `core.vbs` and the machine
/// library are all running under one interpreter and all three are long, so
/// the number on its own places nothing — and the person reading it is usually
/// not the person who can open all three.
///
/// The blame is taken where the source is still at hand, and the line number
/// is consumed as it goes, so an outer frame cannot claim a line that belongs
/// to a library.
#[test]
fn an_error_says_which_script_and_quotes_the_line() {
    let e = fails(
        "Dim lib\n\
         lib = \"Dim a\" & vbNewLine & \"a = Nothing.Missing\"\n\
         ExecuteGlobal lib\n",
    );
    let said = e.to_string();
    assert!(said.contains("in a loaded library"), "{said}");
    assert!(said.contains("line 2"), "{said}");
    // The line itself, which is the part somebody can search for.
    assert!(said.contains("a = Nothing.Missing"), "{said}");
    // The line the *caller* was on is still there, and that is the
    // `ExecuteGlobal` itself: inner blame first, outer after, which reads as a
    // stack rather than as two claims on the same number.
    assert_eq!(e.line, Some(3), "{said}");
}

/// A `Property Let` is handed the object, not its default value.
///
/// `x = obj` stores the object's default property and `Set x = obj` stores the
/// object — but that rule is about writing a *variable*. Assigning to a
/// property is a call, and a call's argument is passed as it is.
///
/// Half the modern tables are built on the difference:
///
/// ```vbs
/// Public Property Let Object(a) : Set Slingshot = a : End Property
/// LS.Object = LeftSlingshot
/// ```
///
/// The property is a `Let`, so it takes a value; the value is a table part,
/// and the first thing it does with it is `Set` it. Dereference on the way in
/// and the `Set` has nothing to set — The Getaway stopped at its line 225 with
/// "Object required" before drawing anything.
#[test]
fn a_property_let_is_handed_the_object_itself() {
    let src = "Class Holder
       Public Kept
       Public Property Let Object(a) : Set Kept = a : End Property
       End Class
       Class Thing : Public Tag : End Class
       Dim t : Set t = New Thing : t.Tag = 7
       Dim h : Set h = New Holder
       h.Object = t
       Dim got : got = h.Kept.Tag
";
    assert_eq!(num(src, "got"), 7.0);

    // And a variable still dereferences, which is the other half of the rule:
    // a class with no default property cannot be assigned without `Set`.
    let e = fails(
        "Class Thing : End Class
Dim t : Set t = New Thing
Dim v : v = t
",
    );
    assert_eq!(e.number, 438, "{e}");
}

/// A single-line `If` whose statements are introduced by colons.
///
/// `If c Then: a: Else: b` is one line and one statement, and the colon after
/// `Then` is a separator rather than the end of anything. Read as a block `If`
/// — which is what happens if the parser asks "is the next token an end of
/// statement" instead of "is there anything left on this line" — it is an `If`
/// that never ends, and the parser runs off the bottom of the file looking for
/// an `End If`. Circus writes it that way eleven times.
#[test]
fn a_single_line_if_may_put_a_colon_after_then() {
    assert_eq!(num("a = 0 : If 1 = 1 Then: a = 7: Else: a = 9", "a"), 7.0);
    assert_eq!(num("a = 0 : If 1 = 2 Then: a = 7: Else: a = 9", "a"), 9.0);

    // And both halves keep taking several statements.
    assert_eq!(
        num("If 1 = 1 Then: a = 1: b = 2: Else: a = 3: b = 4", "b"),
        2.0
    );
    assert_eq!(
        num("If 1 = 2 Then: a = 1: b = 2: Else: a = 3: b = 4", "b"),
        4.0
    );

    // A block `If` is still a block `If` when its line ends in a stray colon.
    let src = "If 1 = 1 Then:
        a = 5
    End If";
    assert_eq!(num(src, "a"), 5.0);
}

/// `Public Default` marks the member an instance answers to with no name at
/// all — which is what `(new Thing)(a, b)` calls. Visual Pinball's own script
/// library uses it twice and the tables built on it use it constantly.
#[test]
fn a_class_can_have_a_default_member() {
    let src = "
        Class Adder
            Private total
            Public Default Function init(a, b)
                total = a + b
                Set init = Me
            End Function
            Public Property Get Sum(): Sum = total: End Property
        End Class
        Dim x
        Set x = (new Adder)(3, 4)
        a = x.Sum";
    assert_eq!(num(src, "a"), 7.0);
}
