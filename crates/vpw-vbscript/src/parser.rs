//! Tokens to a tree.
//!
//! A plain recursive-descent parser with a precedence climber for expressions.
//! What makes VBScript awkward is not the grammar but that several constructs
//! are only distinguishable after the fact:
//!
//! - `Foo 1, 2` is a call with arguments and no parentheses. `Foo (1)` is a
//!   call with **one** parenthesised argument, not a call with a tuple. And
//!   `Foo(1) = 2` is an assignment to a subscript. The parser reads an
//!   expression first and then decides by what follows it.
//! - `If x Then y = 1` on one line has no `End If`; `If x Then` followed by a
//!   newline does. The difference is whether anything comes after `Then` on the
//!   same line.
//! - `Property Get`, `Property Let` and `Property Set` are three procedures
//!   that share a name.
//!
//! Errors carry a line number and a description, and they are raised as
//! VBScript compilation errors (1002), so a table with a typo reports the same
//! way the real engine does.

use std::rc::Rc;

use crate::ast::*;
use crate::error::{Error, Result};
use crate::lexer::{Punct, Tok, Token, lex};

/// Parses a whole script.
pub fn parse(src: &str) -> Result<Program> {
    let tokens = lex(src)?;
    Parser::new(tokens).program()
}

/// Parses a single expression, which is what `Eval` needs.
pub fn parse_expression(src: &str) -> Result<Expr> {
    let tokens = lex(src)?;
    let mut p = Parser::new(tokens);
    let e = p.expression()?;
    if !p.at_eos() {
        return Err(p.err(format!("unexpected {} after expression", p.at().tok)));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// How deep we are in nested expressions, to stop a pathological or
    /// malicious script from blowing the native stack while parsing.
    depth: u32,
    /// Whether this unit has said `Option Explicit` yet. Stamped onto every
    /// procedure so the rule travels with it.
    option_explicit: bool,
}

/// The nesting limit for expressions. Real scripts do not come close; a file
/// made of ten thousand open parentheses would otherwise abort the process
/// instead of reporting an error.
const MAX_DEPTH: u32 = 256;

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Self {
            toks,
            pos: 0,
            depth: 0,
            option_explicit: false,
        }
    }

    // -- token helpers -----------------------------------------------------

    fn at(&self) -> &Token {
        // The lexer always ends with `Eof`, so this never runs off the end.
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn line(&self) -> u32 {
        self.at().line
    }

    fn bump(&mut self) -> Token {
        let t = self.at().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        self.at().tok == Tok::Eof
    }

    /// At a statement boundary of any kind.
    fn at_eos(&self) -> bool {
        matches!(self.at().tok, Tok::Eos | Tok::Eol | Tok::Eof)
    }

    /// At the end of the **line**, as opposed to at a `:`.
    ///
    /// A single-line `If` is the only construct that needs the distinction,
    /// and it needs it badly: `If a Then b = 1 : c = 2` puts both statements
    /// in the branch.
    fn at_eol(&self) -> bool {
        matches!(self.at().tok, Tok::Eol | Tok::Eof)
    }

    /// Whether anything but colons is left on this line.
    ///
    /// Which is not the same question as [`Self::at_eos`], and the difference
    /// is what tells a single-line `If` from a block one when the line is
    /// written `If c Then: a = 1: Else: a = 2`. That is one line and one
    /// statement; read as a block it is an `If` that never ends, and the
    /// parser runs off the bottom of the file looking for its `End If`.
    fn line_is_spent(&self) -> bool {
        let last = self.toks.len() - 1;
        let line = self.toks[self.pos.min(last)].line;
        // Colons only, and only the ones on this line. The lexer drops the end
        // of a line that already has a `:` on it, so `Then:` at the end of a
        // line and `Then:` in the middle of one are the same two tokens —
        // which line they sit on is the only thing that tells them apart.
        let mut i = self.pos;
        while matches!(self.toks[i.min(last)].tok, Tok::Eos) && self.toks[i.min(last)].line == line
        {
            i += 1;
        }
        let next = &self.toks[i.min(last)];
        matches!(next.tok, Tok::Eol | Tok::Eof) || next.line != line
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if self.at().is_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: Punct) -> Result<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{p}', found {}", self.at().tok)))
        }
    }

    /// Consumes the keyword if it is there.
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at().is_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> Result<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{kw}', found {}", self.at().tok)))
        }
    }

    /// Consumes two keywords in a row, or nothing. `End If`, `Exit Sub`.
    fn eat_kw2(&mut self, a: &str, b: &str) -> bool {
        if self.at().is_kw(a) && self.toks[(self.pos + 1).min(self.toks.len() - 1)].is_kw(b) {
            self.bump();
            self.bump();
            true
        } else {
            false
        }
    }

    fn peek_kw(&self, ahead: usize, kw: &str) -> bool {
        self.toks[(self.pos + ahead).min(self.toks.len() - 1)].is_kw(kw)
    }

    fn expect_ident(&mut self) -> Result<Rc<str>> {
        match self.bump().tok {
            Tok::Ident(s) => Ok(s),
            other => Err(self.err(format!("expected a name, found {other}"))),
        }
    }

    fn expect_eos(&mut self) -> Result<()> {
        if self.at_eos() {
            if !self.at_eof() {
                self.bump();
            }
            Ok(())
        } else {
            Err(self.err(format!("unexpected {} after statement", self.at().tok)))
        }
    }

    fn skip_eos(&mut self) {
        while matches!(self.at().tok, Tok::Eos | Tok::Eol) {
            self.bump();
        }
    }

    fn err(&self, msg: impl AsRef<str>) -> Error {
        Error::syntax(msg, self.line())
    }

    // -- program -----------------------------------------------------------

    fn program(mut self) -> Result<Program> {
        let mut prog = Program::default();
        self.skip_eos();
        while !self.at_eof() {
            if self.at().is_kw("option") {
                self.bump();
                self.expect_kw("explicit")?;
                prog.option_explicit = true;
                self.option_explicit = true;
                self.expect_eos()?;
                self.skip_eos();
                continue;
            }
            prog.body.push(self.statement()?);
            self.skip_eos();
        }
        Ok(prog)
    }

    /// Parses statements until one of the given keywords starts a line.
    ///
    /// The terminator is left unconsumed so the caller can tell which one it
    /// was — `If` needs to distinguish `ElseIf` from `Else` from `End If`.
    fn block(&mut self, stop: &[&str]) -> Result<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            self.skip_eos();
            if self.at_eof() {
                return Ok(out);
            }
            if stop.iter().any(|k| self.block_ends_with(k)) {
                return Ok(out);
            }
            out.push(self.statement()?);
        }
    }

    /// Whether the current position is the given block terminator.
    ///
    /// The two-word ones have to be checked as a pair: a variable called `End`
    /// is illegal, but `Next` is a perfectly ordinary word in `Next = 3`, and
    /// `Loop` is a name some tables use.
    fn block_ends_with(&self, kw: &str) -> bool {
        match kw {
            "end if" => self.eat_kw2_peek("end", "if"),
            "end select" => self.eat_kw2_peek("end", "select"),
            "end with" => self.eat_kw2_peek("end", "with"),
            "end sub" => self.eat_kw2_peek("end", "sub"),
            "end function" => self.eat_kw2_peek("end", "function"),
            "end property" => self.eat_kw2_peek("end", "property"),
            "end class" => self.eat_kw2_peek("end", "class"),
            _ => self.at().is_kw(kw),
        }
    }

    fn eat_kw2_peek(&self, a: &str, b: &str) -> bool {
        self.at().is_kw(a) && self.peek_kw(1, b)
    }

    // -- statements --------------------------------------------------------

    fn statement(&mut self) -> Result<Stmt> {
        let line = self.line();
        let kind = self.statement_kind()?;
        Ok(Stmt { kind, line })
    }

    fn statement_kind(&mut self) -> Result<StmtKind> {
        // Declarations that can carry a visibility keyword in front.
        if self.at().is_kw("public") || self.at().is_kw("private") {
            // `Public` alone declares variables; `Public Sub`/`Function`/
            // `Property` declares a procedure. `Private Const` exists too.
            if self.peek_kw(1, "sub")
                || self.peek_kw(1, "function")
                || self.peek_kw(1, "property")
                || self.peek_kw(1, "class")
                || self.peek_kw(1, "default")
            {
                let visibility = self.visibility();
                // Outside a class `Default` means nothing: there is no
                // instance for it to be the default of. Inside one it is read
                // where the members are, in `class_def`.
                self.eat_kw("default");
                return self.procedure_or_class(visibility);
            }
            let visibility = self.visibility();
            if self.at().is_kw("const") {
                self.bump();
                return Ok(StmtKind::Const(self.const_list()?));
            }
            // `Public a, b` is a `Dim` that is visible outside.
            let _ = visibility;
            return Ok(StmtKind::Dim(self.dim_list()?));
        }

        if self.at().is_kw("sub")
            || self.at().is_kw("function")
            || self.at().is_kw("property")
            || self.at().is_kw("class")
        {
            return self.procedure_or_class(Visibility::Public);
        }

        if self.eat_kw("dim") {
            return Ok(StmtKind::Dim(self.dim_list()?));
        }
        if self.eat_kw("const") {
            return Ok(StmtKind::Const(self.const_list()?));
        }
        if self.eat_kw("redim") {
            return self.redim();
        }
        if self.at().is_kw("if") {
            return self.if_stmt();
        }
        if self.at().is_kw("for") {
            return self.for_stmt();
        }
        if self.at().is_kw("do") {
            return self.do_stmt();
        }
        if self.at().is_kw("while") {
            return self.while_stmt();
        }
        if self.at().is_kw("select") {
            return self.select_stmt();
        }
        if self.at().is_kw("with") {
            return self.with_stmt();
        }
        if self.at().is_kw("exit") {
            return self.exit_stmt();
        }
        if self.at().is_kw("on") {
            return self.on_error();
        }
        if self.eat_kw("erase") {
            let mut names = vec![self.expect_ident()?];
            while self.eat_punct(Punct::Comma) {
                names.push(self.expect_ident()?);
            }
            return Ok(StmtKind::Erase(names));
        }
        if self.eat_kw("call") {
            // `Call Foo(1)` — the parentheses are part of the call here, not a
            // parenthesised first argument.
            let e = self.expression()?;
            return Ok(StmtKind::Call(e));
        }
        if self.at().is_kw("option") {
            // A stray `Option Explicit` below the first statement. The real
            // engine rejects it; accepting it costs nothing and a few tables
            // have one.
            self.bump();
            self.eat_kw("explicit");
            return Ok(StmtKind::Nop);
        }
        // `Set x = y`. Guarded on a name following, because `Set` is not a
        // reserved word: a table with a variable called `Set` can write
        // `Set = 3` and mean an assignment.
        if self.at().is_kw("set")
            && matches!(
                self.toks[(self.pos + 1).min(self.toks.len() - 1)].tok,
                Tok::Ident(_)
            )
            && !self.peek_kw(1, "set")
        {
            self.bump();
            return self.assignment(true);
        }

        // Anything else is an assignment or a call.
        self.assignment(false)
    }

    fn visibility(&mut self) -> Visibility {
        if self.eat_kw("private") {
            Visibility::Private
        } else {
            self.eat_kw("public");
            Visibility::Public
        }
    }

    /// An assignment or a bare call, told apart by what follows the target.
    ///
    /// The head is parsed with [`Parser::postfix`] and **not** with a full
    /// expression, which matters more than it looks: `=` is also the comparison
    /// operator, so a full expression parse would swallow it and turn
    /// `Set x = Nothing` into the expression `x = Nothing` with nothing left to
    /// assign to. A statement's head is always a name followed by `.member` and
    /// `(args)`, which is exactly what `postfix` accepts.
    ///
    /// It also gets `a = b = c` right for free: the target is `a` and the value
    /// is the comparison `b = c`, which is what VBScript does.
    fn assignment(&mut self, set: bool) -> Result<StmtKind> {
        let target = self.postfix()?;

        if self.at().is_punct(Punct::Eq) {
            self.bump();
            let value = self.expression()?;
            return Ok(StmtKind::Assign { target, value, set });
        }
        if set {
            return Err(self.err("expected '=' after Set"));
        }

        // `Foo (1), 2` — a call whose **first** argument happens to be
        // parenthesised.
        //
        // `postfix` has already eaten the `(1)` as if it were indexing the
        // result of `Foo`, because at that point the two are the same syntax.
        // A comma right after settles it: `Foo(1)` cannot be indexed and then
        // followed by a comma, so what was read as a subscript is really the
        // first argument. Terminator 2's script is full of `SetLamp (118),0`,
        // and read the other way it calls `SetLamp` with one argument and then
        // indexes the nothing it returns.
        if self.at().is_punct(Punct::Comma)
            && let Expr::Index { base, args } = target
        {
            self.bump();
            let mut args = args;
            args.extend(self.bare_args()?);
            return Ok(StmtKind::Call(Expr::Index { base, args }));
        }

        // A call without parentheses: `Foo 1, 2`. The name has already been
        // parsed as an expression, and the arguments follow it bare.
        if !self.at_eos() && !self.at().is_kw("then") && !self.at().is_kw("else") {
            let args = self.bare_args()?;
            return Ok(StmtKind::Call(Expr::Index {
                base: Box::new(target),
                args,
            }));
        }

        Ok(StmtKind::Call(target))
    }

    /// The arguments of a call written without parentheses.
    fn bare_args(&mut self) -> Result<Vec<Arg>> {
        let mut args = Vec::new();
        loop {
            if self.at().is_punct(Punct::Comma) {
                args.push(None);
            } else {
                args.push(Some(self.expression()?));
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(args)
    }

    fn dim_list(&mut self) -> Result<Vec<DimName>> {
        let mut out = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let bounds = if self.eat_punct(Punct::LParen) {
                let mut b = Vec::new();
                if !self.at().is_punct(Punct::RParen) {
                    b.push(self.expression()?);
                    while self.eat_punct(Punct::Comma) {
                        b.push(self.expression()?);
                    }
                }
                self.expect_punct(Punct::RParen)?;
                Some(b)
            } else {
                None
            };
            out.push(DimName { name, bounds });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(out)
    }

    fn const_list(&mut self) -> Result<Vec<(Rc<str>, Expr)>> {
        let mut out = Vec::new();
        loop {
            let name = self.expect_ident()?;
            self.expect_punct(Punct::Eq)?;
            out.push((name, self.expression()?));
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(out)
    }

    fn redim(&mut self) -> Result<StmtKind> {
        let preserve = self.eat_kw("preserve");
        let mut targets = Vec::new();
        loop {
            let name = self.expect_ident()?;
            self.expect_punct(Punct::LParen)?;
            let mut bounds = vec![self.expression()?];
            while self.eat_punct(Punct::Comma) {
                bounds.push(self.expression()?);
            }
            self.expect_punct(Punct::RParen)?;
            targets.push((name, bounds));
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(StmtKind::ReDim { preserve, targets })
    }

    fn if_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("if")?;
        let cond = self.expression()?;
        self.expect_kw("then")?;

        // A single-line `If`: everything up to the end of the line, and there
        // is no `End If`. What decides it is whether anything follows `Then`
        // *on this line*, and a `:` is not the end of a line.
        if !self.line_is_spent() {
            return self.single_line_if(cond);
        }

        let mut branches = Vec::new();
        let mut else_body = None;
        let mut current = cond;
        loop {
            let body = self.block(&["elseif", "else", "end if"])?;
            branches.push((current, body));

            if self.eat_kw("elseif") {
                current = self.expression()?;
                self.expect_kw("then")?;
                continue;
            }
            if self.eat_kw("else") {
                else_body = Some(self.block(&["end if"])?);
            }
            if !self.eat_kw2("end", "if") {
                return Err(self.err("expected 'End If'"));
            }
            break;
        }
        Ok(StmtKind::If {
            branches,
            else_body,
        })
    }

    /// `If c Then a = 1 Else b = 2`, all on one line.
    ///
    /// The tail can hold several statements separated by `:`, and the `Else`
    /// branch the same. What it cannot hold is another block `If`.
    fn single_line_if(&mut self, cond: Expr) -> Result<StmtKind> {
        let then_body = self.inline_statements()?;
        let else_body = if self.eat_kw("else") {
            Some(self.inline_statements()?)
        } else {
            None
        };
        // Some tables close a single-line `If` with `End If` anyway.
        self.eat_kw2("end", "if");
        Ok(StmtKind::If {
            branches: vec![(cond, then_body)],
            else_body,
        })
    }

    /// The statements packed onto the rest of one line, separated by `:`.
    ///
    /// They all belong to the branch. `If c Then a = 1 : b = 2` makes **both**
    /// assignments conditional, which surprises everyone who reads it as one
    /// conditional statement followed by an unconditional one — and is exactly
    /// why the lexer keeps `:` and a newline as different tokens.
    fn inline_statements(&mut self) -> Result<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            // Stray or repeated colons.
            while matches!(self.at().tok, Tok::Eos) {
                self.bump();
            }
            if self.at_eol() || self.at().is_kw("else") || self.eat_kw2_peek("end", "if") {
                break;
            }
            out.push(self.statement()?);
            if !matches!(self.at().tok, Tok::Eos) {
                break;
            }
        }
        Ok(out)
    }

    fn for_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("for")?;

        if self.eat_kw("each") {
            let var = self.expect_ident()?;
            self.expect_kw("in")?;
            let seq = self.expression()?;
            let body = self.block(&["next"])?;
            self.expect_kw("next")?;
            return Ok(StmtKind::ForEach { var, seq, body });
        }

        let var = self.expect_ident()?;
        self.expect_punct(Punct::Eq)?;
        let from = self.expression()?;
        self.expect_kw("to")?;
        let to = self.expression()?;
        let step = if self.eat_kw("step") {
            Some(self.expression()?)
        } else {
            None
        };
        let body = self.block(&["next"])?;
        self.expect_kw("next")?;
        // `Next i` names the variable again; it carries no meaning.
        if !self.at_eos() && self.at().ident().is_some() {
            self.bump();
        }
        Ok(StmtKind::For {
            var,
            from,
            to,
            step,
            body,
        })
    }

    fn do_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("do")?;

        let head = self.loop_condition(false)?;
        let body = self.block(&["loop"])?;
        self.expect_kw("loop")?;
        let tail = self.loop_condition(true)?;

        if head.is_some() && tail.is_some() {
            return Err(self.err("a Do loop cannot have a condition at both ends"));
        }
        Ok(StmtKind::Do {
            cond: head.or(tail),
            body,
        })
    }

    fn loop_condition(&mut self, at_end: bool) -> Result<Option<DoCond>> {
        let until = if self.eat_kw("while") {
            false
        } else if self.eat_kw("until") {
            true
        } else {
            return Ok(None);
        };
        Ok(Some(DoCond {
            expr: self.expression()?,
            until,
            at_end,
        }))
    }

    fn while_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("while")?;
        let cond = self.expression()?;
        let body = self.block(&["wend"])?;
        self.expect_kw("wend")?;
        Ok(StmtKind::While { cond, body })
    }

    fn select_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("select")?;
        self.expect_kw("case")?;
        let subject = self.expression()?;

        let mut cases = Vec::new();
        let mut default = None;
        loop {
            self.skip_eos();
            if self.eat_kw2("end", "select") {
                break;
            }
            if self.at_eof() {
                return Err(self.err("expected 'End Select'"));
            }
            self.expect_kw("case")?;

            if self.eat_kw("else") {
                default = Some(self.block(&["case", "end select"])?);
                continue;
            }
            let mut tests = vec![self.expression()?];
            while self.eat_punct(Punct::Comma) {
                tests.push(self.expression()?);
            }
            let body = self.block(&["case", "end select"])?;
            cases.push(Case { tests, body });
        }
        Ok(StmtKind::Select {
            subject,
            cases,
            default,
        })
    }

    fn with_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("with")?;
        let subject = self.expression()?;
        let body = self.block(&["end with"])?;
        if !self.eat_kw2("end", "with") {
            return Err(self.err("expected 'End With'"));
        }
        Ok(StmtKind::With { subject, body })
    }

    fn exit_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw("exit")?;
        let kind = if self.eat_kw("sub") {
            ExitKind::Sub
        } else if self.eat_kw("function") {
            ExitKind::Function
        } else if self.eat_kw("property") {
            ExitKind::Property
        } else if self.eat_kw("for") {
            ExitKind::For
        } else if self.eat_kw("do") {
            ExitKind::Do
        } else {
            return Err(self.err("expected Sub, Function, Property, For or Do after Exit"));
        };
        Ok(StmtKind::Exit(kind))
    }

    fn on_error(&mut self) -> Result<StmtKind> {
        self.expect_kw("on")?;
        self.expect_kw("error")?;
        if self.eat_kw("resume") {
            self.expect_kw("next")?;
            return Ok(StmtKind::OnError { resume_next: true });
        }
        // `On Error GoTo 0` turns the handler off. It is the only `GoTo`
        // VBScript has, and the only number allowed after it is zero.
        self.expect_kw("goto")?;
        match self.bump().tok {
            Tok::Number(0.0) => Ok(StmtKind::OnError { resume_next: false }),
            other => Err(self.err(format!("expected 'GoTo 0', found 'GoTo {other}'"))),
        }
    }

    // -- procedures and classes --------------------------------------------

    fn procedure_or_class(&mut self, visibility: Visibility) -> Result<StmtKind> {
        if self.at().is_kw("class") {
            return Ok(StmtKind::Class(Rc::new(self.class_def()?)));
        }
        Ok(StmtKind::Proc(Rc::new(self.proc_def(visibility)?)))
    }

    fn proc_def(&mut self, visibility: Visibility) -> Result<Proc> {
        let line = self.line();

        let (is_function, property) = if self.eat_kw("sub") {
            (false, None)
        } else if self.eat_kw("function") {
            (true, None)
        } else if self.eat_kw("property") {
            let kind = if self.eat_kw("get") {
                PropKind::Get
            } else if self.eat_kw("let") {
                PropKind::Let
            } else if self.eat_kw("set") {
                PropKind::Set
            } else {
                return Err(self.err("expected Get, Let or Set after Property"));
            };
            // A `Property Get` returns a value like a function; `Let` and `Set`
            // take one like a sub.
            (kind == PropKind::Get, Some(kind))
        } else {
            return Err(self.err("expected Sub, Function or Property"));
        };

        let name = self.expect_ident()?;
        let params = self.param_list()?;

        let (end_kw, closer) = match property {
            Some(_) => ("end property", "End Property"),
            None if is_function => ("end function", "End Function"),
            None => ("end sub", "End Sub"),
        };
        let body = self.block(&[end_kw])?;
        let (a, b) = end_kw.split_once(' ').unwrap();
        if !self.eat_kw2(a, b) {
            return Err(self.err(format!("expected '{closer}' for '{name}'")));
        }

        Ok(Proc {
            is_default: false,
            name,
            params,
            body,
            is_function,
            visibility,
            property,
            line,
            option_explicit: self.option_explicit,
        })
    }

    fn param_list(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        if !self.eat_punct(Punct::LParen) {
            // A parameterless `Sub Foo` needs no parentheses.
            return Ok(params);
        }
        if self.eat_punct(Punct::RParen) {
            return Ok(params);
        }
        loop {
            let by_val = if self.eat_kw("byval") {
                true
            } else {
                self.eat_kw("byref");
                false
            };
            let name = self.expect_ident()?;
            // `Sub Foo(a())` declares an array parameter; the parentheses carry
            // no extra information for us.
            if self.eat_punct(Punct::LParen) {
                self.expect_punct(Punct::RParen)?;
            }
            params.push(Param { name, by_val });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(params)
    }

    fn class_def(&mut self) -> Result<ClassDef> {
        let line = self.line();
        self.expect_kw("class")?;
        let name = self.expect_ident()?;

        let mut fields = Vec::new();
        let mut members = Vec::new();
        loop {
            self.skip_eos();
            if self.eat_kw2("end", "class") {
                break;
            }
            if self.at_eof() {
                return Err(self.err(format!("expected 'End Class' for '{name}'")));
            }

            let visibility = if self.at().is_kw("public") || self.at().is_kw("private") {
                self.visibility()
            } else {
                Visibility::Public
            };
            // `Public Default Function` marks the class's default member:
            // the one an instance answers to when it is used where a value is
            // wanted, or called outright.
            let default = self.eat_kw("default");

            if self.at().is_kw("sub") || self.at().is_kw("function") || self.at().is_kw("property")
            {
                let mut proc = self.proc_def(visibility)?;
                proc.is_default = default;
                members.push(Rc::new(proc));
                continue;
            }
            // Anything else at class level is a field. `Dim` is optional inside
            // a class and `Private x` is the usual spelling.
            self.eat_kw("dim");
            for d in self.dim_list()? {
                fields.push(ClassField {
                    name: d.name,
                    visibility,
                    bounds: d.bounds,
                });
            }
        }
        Ok(ClassDef {
            name,
            fields,
            members,
            line,
        })
    }

    // -- expressions -------------------------------------------------------

    /// Where `Not` sits: below every comparison and above `And`.
    ///
    /// Equal to the comparisons' own precedence, which is what makes
    /// `Not a = b` take the whole comparison as its operand while `Not a And b`
    /// stops at the `And`.
    const NOT_PREC: u8 = 5;

    fn expression(&mut self) -> Result<Expr> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(self.err("expression nested too deeply"));
        }
        let r = self.binary(0);
        self.depth -= 1;
        r
    }

    /// Precedence climbing. Every operator here is left-associative, including
    /// `^` — VBScript's `2 ^ 3 ^ 2` is 64, not 512.
    fn binary(&mut self, min_prec: u8) -> Result<Expr> {
        // `Not` is a prefix operator that binds **looser than the
        // comparisons**: `Not cb Is Nothing` is `Not (cb Is Nothing)`, not
        // `(Not cb) Is Nothing`. Parsing it with the other unary operators —
        // which is where it looks like it belongs — gives the second reading,
        // and the second reading raises rather than answering False, because
        // `Not` on an object reference is a type error.
        //
        // That is not a corner case: `If Not cb Is Nothing Then cb state` is
        // how `core.vbs` dispatches every solenoid callback, so on a ROM table
        // it means no flipper, no popper and no kicker ever fires.
        let mut lhs = if min_prec <= Self::NOT_PREC && self.at().is_kw("not") {
            self.bump();
            Expr::Unary {
                op: UnOp::Not,
                operand: Box::new(self.binary(Self::NOT_PREC)?),
            }
        } else {
            self.unary()?
        };
        while let Some(op) = self.peek_binop() {
            let prec = op.precedence();
            if prec < min_prec {
                break;
            }
            self.bump_binop(op);
            let rhs = self.binary(prec + 1)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn peek_binop(&self) -> Option<BinOp> {
        let t = self.at();
        if let Tok::Punct(p) = &t.tok {
            return Some(match p {
                Punct::Caret => BinOp::Pow,
                Punct::Star => BinOp::Mul,
                Punct::Slash => BinOp::Div,
                Punct::Backslash => BinOp::IntDiv,
                Punct::Plus => BinOp::Add,
                Punct::Minus => BinOp::Sub,
                Punct::Amp => BinOp::Concat,
                Punct::Eq => BinOp::Eq,
                Punct::Ne => BinOp::Ne,
                Punct::Lt => BinOp::Lt,
                Punct::Gt => BinOp::Gt,
                Punct::Le => BinOp::Le,
                Punct::Ge => BinOp::Ge,
                _ => return None,
            });
        }
        // The word operators.
        for (kw, op) in [
            ("mod", BinOp::Mod),
            ("is", BinOp::Is),
            ("and", BinOp::And),
            ("or", BinOp::Or),
            ("xor", BinOp::Xor),
            ("eqv", BinOp::Eqv),
            ("imp", BinOp::Imp),
        ] {
            if t.is_kw(kw) {
                return Some(op);
            }
        }
        None
    }

    fn bump_binop(&mut self, _op: BinOp) {
        self.bump();
    }

    fn unary(&mut self) -> Result<Expr> {
        // `Not` where an operand is required rather than an expression:
        // `x = Not y`. [`Self::binary`] takes it first everywhere it can, so
        // this only sees the positions where the looser reading is impossible.
        if self.eat_kw("not") {
            return Ok(Expr::Unary {
                op: UnOp::Not,
                operand: Box::new(self.unary()?),
            });
        }
        if self.at().is_punct(Punct::Minus) {
            self.bump();
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                operand: Box::new(self.unary()?),
            });
        }
        if self.at().is_punct(Punct::Plus) {
            self.bump();
            return Ok(Expr::Unary {
                op: UnOp::Plus,
                operand: Box::new(self.unary()?),
            });
        }
        self.postfix()
    }

    /// A primary expression followed by any number of `.name` and `(args)`.
    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            // A dot with a gap before it is not a member access. See
            // [`vpw_vbscript::lexer::Token::spaced`]: inside a `With`, a
            // leading dot starts a statement, and `s11.vbs` writes one on the
            // same line as the `Case` it belongs to.
            if self.at().is_punct(Punct::Dot) && !self.at().spaced {
                self.bump();
                let name = self.expect_ident()?;
                e = Expr::Member {
                    base: Box::new(e),
                    name,
                };
                continue;
            }
            if self.at().is_punct(Punct::LParen) {
                self.bump();
                let mut args = Vec::new();
                if !self.at().is_punct(Punct::RParen) {
                    loop {
                        if self.at().is_punct(Punct::Comma) {
                            args.push(None);
                        } else {
                            args.push(Some(self.expression()?));
                        }
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                }
                self.expect_punct(Punct::RParen)?;
                e = Expr::Index {
                    base: Box::new(e),
                    args,
                };
                continue;
            }
            break;
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr> {
        // `.Foo` inside a `With`.
        if self.at().is_punct(Punct::Dot) {
            self.bump();
            let name = self.expect_ident()?;
            return Ok(Expr::WithMember { name });
        }
        if self.eat_punct(Punct::LParen) {
            let e = self.expression()?;
            self.expect_punct(Punct::RParen)?;
            return Ok(e);
        }
        if self.eat_kw("new") {
            let name = self.expect_ident()?;
            return Ok(Expr::New(name));
        }

        let t = self.bump();
        match t.tok {
            Tok::Number(n) => Ok(Expr::Number(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Ident(s) => Ok(match &*s.to_ascii_lowercase() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                "empty" => Expr::Empty,
                "null" => Expr::Null,
                "nothing" => Expr::Nothing,
                _ => Expr::Ident(s),
            }),
            other => Err(Error::syntax(
                format!("unexpected {other} in an expression"),
                t.line,
            )),
        }
    }
}
