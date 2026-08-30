//! The shape of a parsed script.
//!
//! Two things about this tree are worth knowing before reading it.
//!
//! **A call and an array index look identical.** `Foo(1)` is a subscript if
//! `Foo` is an array and a call if it is a procedure, and VBScript does not
//! decide until it runs — the same syntax means different things depending on
//! what the name turns out to be. So there is one [`Expr::Index`] node and the
//! interpreter resolves it. Trying to split them at parse time is where a
//! VBScript parser usually goes wrong.
//!
//! **Assignment is a statement and `=` is also a comparison.** `a = b` at the
//! start of a statement assigns; anywhere else it compares. The parser handles
//! that by parsing an expression and then looking for a trailing `=`, which is
//! also how it copes with `a(i).b = c`.

use std::rc::Rc;

/// A whole script.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub body: Vec<Stmt>,
    /// `Option Explicit` at the top: using an undeclared variable is an error.
    pub option_explicit: bool,
}

/// A statement, with the line it came from so errors can point at it.
#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    /// `Dim a, b(3), c()`.
    Dim(Vec<DimName>),
    /// `ReDim [Preserve] a(3, 4)`.
    ReDim {
        preserve: bool,
        targets: Vec<(Rc<str>, Vec<Expr>)>,
    },
    /// `Const a = 1, b = 2`.
    Const(Vec<(Rc<str>, Expr)>),
    /// `a = b`, `a.b = c`, `a(i) = c`, and the `Set` forms of all three.
    Assign {
        target: Expr,
        value: Expr,
        /// `Set a = b` binds the object; `a = b` binds its default value.
        set: bool,
    },
    /// A statement that is just a call: `Foo 1, 2` or `Call Foo(1)`.
    Call(Expr),
    If {
        /// The `If` and every `ElseIf`, in order.
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    For {
        var: Rc<str>,
        from: Expr,
        to: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    ForEach {
        var: Rc<str>,
        seq: Expr,
        body: Vec<Stmt>,
    },
    /// Every shape of `Do ... Loop`. A condition can sit at either end, and
    /// which end it is on decides whether the body runs at least once.
    Do {
        cond: Option<DoCond>,
        body: Vec<Stmt>,
    },
    /// `While ... Wend`, the older spelling.
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Select {
        subject: Expr,
        cases: Vec<Case>,
        default: Option<Vec<Stmt>>,
    },
    /// `With obj ... End With`. Inside, `.Foo` means `obj.Foo`.
    With {
        subject: Expr,
        body: Vec<Stmt>,
    },
    /// A `Sub` or `Function` definition.
    Proc(Rc<Proc>),
    Class(Rc<ClassDef>),
    Exit(ExitKind),
    /// `On Error Resume Next` / `On Error GoTo 0`.
    OnError {
        resume_next: bool,
    },
    /// `Erase a, b`: empties the arrays.
    Erase(Vec<Rc<str>>),
    /// A statement that does nothing but is legal: a stray `Option Explicit`
    /// after the first, a lone `:`.
    Nop,
}

/// One name in a `Dim`, with its bounds if it was declared as an array.
#[derive(Debug, Clone)]
pub struct DimName {
    pub name: Rc<str>,
    /// `None` for a plain variable, `Some(vec![])` for `Dim a()` — an array
    /// that only `ReDim` will give a size — and the bounds otherwise.
    pub bounds: Option<Vec<Expr>>,
}

/// Where a `Do` loop's condition sits and how it reads.
#[derive(Debug, Clone)]
pub struct DoCond {
    pub expr: Expr,
    /// `Until` instead of `While`: the condition is inverted.
    pub until: bool,
    /// At the `Loop` rather than at the `Do`, so the body always runs once.
    pub at_end: bool,
}

#[derive(Debug, Clone)]
pub struct Case {
    /// `Case 1, 2, 3` — any of them matches.
    pub tests: Vec<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    Sub,
    Function,
    Property,
    For,
    Do,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

/// A `Sub` or a `Function`.
#[derive(Debug, Clone)]
pub struct Proc {
    pub name: Rc<str>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// A `Function` has a return value, assigned to its own name. A `Sub` does
    /// not, and calling one in an expression is an error.
    pub is_function: bool,
    pub visibility: Visibility,
    /// Set for `Property Get/Let/Set` inside a class.
    pub property: Option<PropKind>,
    /// `Public Default` — the member an instance answers to when it is used
    /// where a value is wanted, or called with no member name at all.
    ///
    /// `Set d = (new DropTarget)(a, b, c)` is the second of those: the class
    /// is constructed and then *called*, and what runs is whichever member
    /// was marked default. Visual Pinball's own script library does it twice
    /// and half the tables written since do it once.
    pub is_default: bool,
    pub line: u32,
    /// Whether the unit this was written in opened with `Option Explicit`.
    ///
    /// It travels with the procedure because that is where the rule lives:
    /// VBScript settles it when it compiles the unit, so a procedure keeps
    /// whatever its own file said no matter who calls it later or what has been
    /// loaded since.
    pub option_explicit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Get,
    Let,
    Set,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Rc<str>,
    /// `ByVal` copies; `ByRef`, the default, lets the callee write back.
    pub by_val: bool,
}

/// A `Class ... End Class`.
#[derive(Debug, Clone)]
pub struct ClassDef {
    pub name: Rc<str>,
    /// Member variables, in declaration order.
    pub fields: Vec<ClassField>,
    /// Methods and properties. Several can share a name — a `Property Get` and
    /// a `Property Let` always do — so this is a list and not a map.
    pub members: Vec<Rc<Proc>>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ClassField {
    pub name: Rc<str>,
    pub visibility: Visibility,
    pub bounds: Option<Vec<Expr>>,
}

/// An argument at a call site. `None` is an omitted one: `Foo(1, , 3)`.
pub type Arg = Option<Expr>;

#[derive(Debug, Clone)]
pub enum Expr {
    Empty,
    Null,
    Nothing,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    /// A bare name: a variable, a constant, a procedure or a host global.
    Ident(Rc<str>),
    /// `base(args)`. An array subscript, a call, or a parameterised property —
    /// only the interpreter can tell.
    Index {
        base: Box<Expr>,
        args: Vec<Arg>,
    },
    /// `base.name`.
    Member {
        base: Box<Expr>,
        name: Rc<str>,
    },
    /// `.name` inside a `With`.
    WithMember {
        name: Rc<str>,
    },
    /// `New Foo`.
    New(Rc<str>),
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    /// Accepted and ignored, as in the real engine.
    Plus,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Pow,
    Mul,
    Div,
    /// `\`: integer division. Rounds both operands first.
    IntDiv,
    Mod,
    Add,
    Sub,
    /// `&`: always concatenation, never addition.
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `Is`: reference identity, not equality.
    Is,
    And,
    Or,
    Xor,
    /// `Eqv`: bitwise equivalence. Rare, but it is in the language.
    Eqv,
    /// `Imp`: bitwise implication. Rarer still.
    Imp,
}

impl BinOp {
    /// Binding power. Higher binds tighter.
    ///
    /// The order is VBScript's, which differs from C's in ways that change what
    /// real scripts mean:
    ///
    /// - **Comparison binds tighter than `And` and `Or`**, so `a = 1 And b = 2`
    ///   is `(a = 1) And (b = 2)` and needs no parentheses. In C the same
    ///   shape would be a bitwise mess.
    /// - **`&` binds looser than `+`**, so `"n=" & a + b` concatenates the sum.
    /// - **The logical operators are not one level.** From tightest to
    ///   loosest: `Not`, `And`, `Or`, `Xor`, `Eqv`, `Imp`. Putting `Or` and
    ///   `Xor` together, which is the easy mistake, changes what
    ///   `a Or b Xor c` means.
    /// - **All the comparisons sit at one level and are left-associative**, so
    ///   `a < b < c` parses as `(a < b) < c` — legal, and almost never what
    ///   was meant.
    ///
    /// `Not` is unary and sits between the comparisons and `And`; it is handled
    /// where unary operators are parsed, not here.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Pow => 12,
            BinOp::Mul | BinOp::Div => 10,
            BinOp::IntDiv => 9,
            BinOp::Mod => 8,
            BinOp::Add | BinOp::Sub => 7,
            BinOp::Concat => 6,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Is => 5,
            BinOp::And => 4,
            BinOp::Or => 3,
            BinOp::Xor => 2,
            BinOp::Eqv => 1,
            BinOp::Imp => 0,
        }
    }
}
