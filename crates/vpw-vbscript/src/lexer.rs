//! Turning VBScript source into tokens.
//!
//! # The things that bite
//!
//! - **Newlines are syntax.** A statement ends at the end of the line, so the
//!   lexer cannot throw line breaks away the way most lexers do. `:` also
//!   separates statements, but it gets a **different** token: a single-line
//!   `If` runs to the end of the line, so the parser has to be able to tell a
//!   colon from a line break.
//! - **All three line endings count.** `\r\n`, `\n` and a bare `\r` each end a
//!   line. That last one is not a hypothetical: the script inside Terminator 2
//!   uses bare carriage returns, and treating them as whitespace collapses the
//!   file onto a single line, at which point nothing parses and the error
//!   points at the end of the file.
//! - **A line can be continued** with a trailing underscore, which swallows the
//!   newline. Tables use it constantly for long `If` conditions.
//! - **Comments** start with `'` or with the keyword `Rem`, and run to the end
//!   of the line. A `'` inside a string is not a comment, which is why comments
//!   cannot be stripped before lexing.
//! - **Everything is case-insensitive**, keywords and identifiers alike. A
//!   table that declares `Sub SolFlipper` and calls `solflipper` is correct
//!   VBScript, and tables do exactly that.
//! - **Strings double their quotes** to escape them: `"say ""hi"""`. There are
//!   no backslash escapes at all.
//! - **`&` is both** the concatenation operator and the prefix of a hex or
//!   octal literal. `a &H10` is a concatenation and `a & H10` would be too;
//!   what decides is whether the `&` is glued to an `H` and a digit.
//!
//! Keywords are not recognised here. The lexer emits identifiers and the parser
//! decides what is a keyword, because VBScript lets you use many keyword-ish
//! words as member names — `x.Error`, `x.Property` — and a lexer that hard-coded
//! them would break on real tables.

use std::fmt;
use std::rc::Rc;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// An identifier or a keyword. Stored as written; compare with
    /// [`Token::is_kw`], never with `==`.
    Ident(Rc<str>),
    Number(f64),
    Str(Rc<str>),
    /// A `:`, which separates statements **within** a line.
    Eos,
    /// A newline, which ends a line.
    ///
    /// It is a different token from `Eos` for one reason, and it is not
    /// cosmetic: a single-line `If` runs to the end of the **line**, so
    /// `If a Then b = 1 : c = 2` puts both statements in the branch while
    /// `If a Then b = 1` followed by a newline and `c = 2` does not. Collapsing
    /// the two into one token makes that distinction unrecoverable.
    Eol,
    /// Punctuation and operators.
    Punct(Punct),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punct {
    LParen,
    RParen,
    Comma,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    Amp,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl fmt::Display for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Punct::LParen => "(",
            Punct::RParen => ")",
            Punct::Comma => ",",
            Punct::Dot => ".",
            Punct::Plus => "+",
            Punct::Minus => "-",
            Punct::Star => "*",
            Punct::Slash => "/",
            Punct::Backslash => "\\",
            Punct::Caret => "^",
            Punct::Amp => "&",
            Punct::Eq => "=",
            Punct::Ne => "<>",
            Punct::Lt => "<",
            Punct::Gt => ">",
            Punct::Le => "<=",
            Punct::Ge => ">=",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    /// Whether anything separated this token from the one before it.
    ///
    /// Needed for exactly one decision, and it is not a nicety. Inside a `With`
    /// block, a leading dot starts a statement, and `s11.vbs` puts one on the
    /// same line as the `Case` it belongs to:
    ///
    /// ```vbscript
    /// Case StartGameKey    .Switch(swStartButton) = True
    /// ```
    ///
    /// Read greedily that is `StartGameKey.Switch(...)` — a member of a number,
    /// which raises "object required" and takes the whole keyboard handler with
    /// it. What tells the two apart is the gap: nobody writes `a . b`, and the
    /// original's own scripts never do.
    pub spaced: bool,
}

impl Token {
    /// Whether this token is the given keyword, compared case-insensitively.
    pub fn is_kw(&self, kw: &str) -> bool {
        matches!(&self.tok, Tok::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    pub fn is_punct(&self, p: Punct) -> bool {
        self.tok == Tok::Punct(p)
    }

    pub fn ident(&self) -> Option<&Rc<str>> {
        match &self.tok {
            Tok::Ident(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Number(n) => write!(f, "{n}"),
            Tok::Str(s) => write!(f, "{s:?}"),
            Tok::Eos => write!(f, "':'"),
            Tok::Eol => write!(f, "end of line"),
            Tok::Punct(p) => write!(f, "{p}"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

/// Tokenises a whole script.
pub fn lex(src: &str) -> Result<Vec<Token>> {
    Lexer::new(src).run()
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    out: Vec<Token>,
    /// Whether whitespace, a comment or a line continuation has gone by since
    /// the last token. Consumed by the next [`Lexer::push`].
    gap: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            gap: false,
            out: Vec::new(),
        }
    }

    fn peek(&self) -> u8 {
        self.src.get(self.pos).copied().unwrap_or(0)
    }

    fn peek_at(&self, n: usize) -> u8 {
        self.src.get(self.pos + n).copied().unwrap_or(0)
    }

    fn push(&mut self, tok: Tok) {
        let line = self.line;
        let spaced = std::mem::take(&mut self.gap);
        self.out.push(Token { tok, line, spaced });
    }

    /// Emits a separator, collapsing runs of them.
    ///
    /// Blank lines are extremely common and a run of separators carries no
    /// information beyond "something ended", so the parser is spared having to
    /// skip them everywhere. A separator at the very start is dropped too.
    fn push_sep(&mut self, tok: Tok) {
        if !matches!(
            self.out.last().map(|t| &t.tok),
            None | Some(Tok::Eos) | Some(Tok::Eol)
        ) {
            self.push(tok);
        }
    }

    fn run(mut self) -> Result<Vec<Token>> {
        while self.pos < self.src.len() {
            let c = self.peek();
            match c {
                b' ' | b'\t' => {
                    self.gap = true;
                    self.pos += 1;
                }
                b'\n' => {
                    self.push_sep(Tok::Eol);
                    self.pos += 1;
                    self.line += 1;
                }
                b'\r' => {
                    self.push_sep(Tok::Eol);
                    // `\r\n` is one ending, not two.
                    self.pos += if self.peek_at(1) == b'\n' { 2 } else { 1 };
                    self.line += 1;
                }
                b':' => {
                    self.push_sep(Tok::Eos);
                    self.pos += 1;
                }
                b'\'' => {
                    self.gap = true;
                    self.skip_comment();
                }
                b'_' if self.is_line_continuation() => {
                    self.gap = true;
                    self.skip_continuation();
                }
                b'"' => self.string()?,
                b'0'..=b'9' => self.number()?,
                b'.' if self.peek_at(1).is_ascii_digit() => self.number()?,
                b'&' if self.is_based_literal() => self.number()?,
                c if is_ident_start(c) => self.word(),
                _ => self.punct()?,
            }
        }
        self.push_sep(Tok::Eol);
        self.push(Tok::Eof);
        Ok(self.out)
    }

    /// A trailing `_` continues the line. It only counts when nothing but
    /// whitespace follows it, otherwise it is an identifier character.
    fn is_line_continuation(&self) -> bool {
        let mut i = self.pos + 1;
        while matches!(self.src.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        matches!(self.src.get(i), Some(b'\n' | b'\r') | None)
    }

    fn skip_continuation(&mut self) {
        self.skip_to_end_of_line();
        if matches!(self.peek(), b'\r') {
            self.pos += if self.peek_at(1) == b'\n' { 2 } else { 1 };
            self.line += 1;
        } else if self.peek() == b'\n' {
            self.pos += 1;
            self.line += 1;
        }
    }

    fn skip_comment(&mut self) {
        self.skip_to_end_of_line();
    }

    fn skip_to_end_of_line(&mut self) {
        while self.pos < self.src.len() && !matches!(self.peek(), b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    /// `&H1F` / `&O17`, as opposed to the `&` operator.
    fn is_based_literal(&self) -> bool {
        match self.peek_at(1).to_ascii_lowercase() {
            b'h' => self.peek_at(2).is_ascii_hexdigit(),
            b'o' => self.peek_at(2).is_ascii_digit(),
            _ => false,
        }
    }

    fn word(&mut self) {
        let start = self.pos;
        while is_ident_char(self.peek()) {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];

        // `Rem` is a comment keyword, not an identifier — but only when it
        // stands alone. `Remaining = 1` is a variable.
        if text.eq_ignore_ascii_case(b"rem") {
            self.skip_comment();
            return;
        }
        let s: Rc<str> = Rc::from(std::str::from_utf8(text).unwrap_or_default());
        self.push(Tok::Ident(s));
    }

    fn number(&mut self) -> Result<()> {
        let start = self.pos;
        if self.peek() == b'&' {
            self.pos += 2; // `&H` or `&O`
            while self.peek().is_ascii_alphanumeric() {
                self.pos += 1;
            }
            // A trailing `&` is a Long marker and carries no value.
            if self.peek() == b'&' {
                self.pos += 1;
            }
        } else {
            while self.peek().is_ascii_digit() {
                self.pos += 1;
            }
            if self.peek() == b'.' && self.peek_at(1).is_ascii_digit() {
                self.pos += 1;
                while self.peek().is_ascii_digit() {
                    self.pos += 1;
                }
            } else if self.peek() == b'.' && !is_ident_start(self.peek_at(1)) {
                // `1.` is a valid literal; `1.Foo` is not a number at all.
                self.pos += 1;
            }
            // An exponent, but only if it really is one: `1e` on its own is an
            // identifier glued to a number, and `2elements` must not be eaten.
            if matches!(self.peek().to_ascii_lowercase(), b'e') {
                let after_sign = usize::from(matches!(self.peek_at(1), b'+' | b'-'));
                if self.peek_at(1 + after_sign).is_ascii_digit() {
                    self.pos += 1 + after_sign;
                    while self.peek().is_ascii_digit() {
                        self.pos += 1;
                    }
                }
            }
        }

        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or_default();
        let n = crate::value::parse_number(text)
            .ok_or_else(|| Error::syntax(format!("bad number literal '{text}'"), self.line))?;
        self.push(Tok::Number(n));
        Ok(())
    }

    fn string(&mut self) -> Result<()> {
        self.pos += 1;
        let mut s = String::new();
        loop {
            match self.src.get(self.pos) {
                None | Some(b'\n' | b'\r') => {
                    return Err(Error::syntax("unterminated string literal", self.line));
                }
                Some(b'"') => {
                    // A doubled quote is an escaped quote; a single one ends it.
                    if self.peek_at(1) == b'"' {
                        s.push('"');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        break;
                    }
                }
                Some(_) => {
                    // Step by whole UTF-8 characters so a table with an accent
                    // in a message does not come out mangled.
                    let rest = &self.src[self.pos..];
                    let len = utf8_len(rest[0]);
                    let chunk = std::str::from_utf8(&rest[..len.min(rest.len())])
                        .map_err(|_| Error::syntax("invalid UTF-8 in string", self.line))?;
                    s.push_str(chunk);
                    self.pos += len;
                }
            }
        }
        self.push(Tok::Str(Rc::from(s.as_str())));
        Ok(())
    }

    fn punct(&mut self) -> Result<()> {
        let two = |a: u8, b: u8| (a as u16) << 8 | b as u16;
        let pair = two(self.peek(), self.peek_at(1));
        let (p, len) = match pair {
            p if p == two(b'<', b'>') => (Punct::Ne, 2),
            p if p == two(b'<', b'=') => (Punct::Le, 2),
            p if p == two(b'>', b'=') => (Punct::Ge, 2),
            // `=<` and `=>` are accepted by the real engine.
            p if p == two(b'=', b'<') => (Punct::Le, 2),
            p if p == two(b'=', b'>') => (Punct::Ge, 2),
            _ => {
                let p = match self.peek() {
                    b'(' => Punct::LParen,
                    b')' => Punct::RParen,
                    b',' => Punct::Comma,
                    b'.' => Punct::Dot,
                    b'+' => Punct::Plus,
                    b'-' => Punct::Minus,
                    b'*' => Punct::Star,
                    b'/' => Punct::Slash,
                    b'\\' => Punct::Backslash,
                    b'^' => Punct::Caret,
                    b'&' => Punct::Amp,
                    b'=' => Punct::Eq,
                    b'<' => Punct::Lt,
                    b'>' => Punct::Gt,
                    c => {
                        return Err(Error::syntax(
                            format!("unexpected character '{}'", c as char),
                            self.line,
                        ));
                    }
                };
                (p, 1)
            }
        };
        self.push(Tok::Punct(p));
        self.pos += len;
        Ok(())
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn a_newline_ends_a_line() {
        assert_eq!(
            toks("a\nb"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eol,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn a_colon_separates_statements_within_a_line() {
        assert_eq!(
            toks("a : b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eos,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn runs_of_separators_collapse() {
        // Blank lines carry no information and the parser should not have to
        // skip them at every turn.
        assert_eq!(
            toks("\n\n:\na\n\n\n"),
            vec![Tok::Ident("a".into()), Tok::Eol, Tok::Eof]
        );
    }

    #[test]
    fn an_underscore_continues_the_line() {
        assert_eq!(
            toks("a + _\n  b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Punct(Punct::Plus),
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn an_underscore_inside_a_name_is_not_a_continuation() {
        assert_eq!(
            toks("my_var"),
            vec![Tok::Ident("my_var".into()), Tok::Eol, Tok::Eof]
        );
    }

    #[test]
    fn a_leading_underscore_is_a_name() {
        assert_eq!(
            toks("_x = 1"),
            vec![
                Tok::Ident("_x".into()),
                Tok::Punct(Punct::Eq),
                Tok::Number(1.0),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        assert_eq!(
            toks("a ' this is ignored\nb"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eol,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
        assert_eq!(toks("Rem nothing here\n"), vec![Tok::Eof]);
    }

    #[test]
    fn rem_is_only_a_comment_on_its_own() {
        // `Remaining` is a perfectly good variable name.
        assert_eq!(
            toks("Remaining = 1"),
            vec![
                Tok::Ident("Remaining".into()),
                Tok::Punct(Punct::Eq),
                Tok::Number(1.0),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn a_quote_inside_a_string_is_not_a_comment() {
        assert_eq!(
            toks("\"it's fine\""),
            vec![Tok::Str("it's fine".into()), Tok::Eol, Tok::Eof]
        );
    }

    #[test]
    fn doubled_quotes_escape() {
        assert_eq!(
            toks("\"say \"\"hi\"\"\""),
            vec![Tok::Str("say \"hi\"".into()), Tok::Eol, Tok::Eof]
        );
        assert_eq!(toks("\"\""), vec![Tok::Str("".into()), Tok::Eol, Tok::Eof]);
    }

    #[test]
    fn there_are_no_backslash_escapes() {
        // A path in a table's script is written plainly, and `\n` is two
        // characters.
        assert_eq!(
            toks(r#""c:\tables\n""#),
            vec![Tok::Str(r"c:\tables\n".into()), Tok::Eol, Tok::Eof]
        );
    }

    #[test]
    fn an_unterminated_string_is_a_syntax_error() {
        assert!(lex("\"oops\n").is_err());
        assert!(lex("\"oops").is_err());
    }

    #[test]
    fn numbers_cover_what_tables_write() {
        assert_eq!(toks("1")[0], Tok::Number(1.0));
        assert_eq!(toks("1.5")[0], Tok::Number(1.5));
        assert_eq!(toks(".5")[0], Tok::Number(0.5));
        assert_eq!(toks("1e3")[0], Tok::Number(1000.0));
        assert_eq!(toks("1.5E-2")[0], Tok::Number(0.015));
        assert_eq!(toks("&H1F")[0], Tok::Number(31.0));
        assert_eq!(toks("&O17")[0], Tok::Number(15.0));
        assert_eq!(toks("&H20&")[0], Tok::Number(32.0));
    }

    #[test]
    fn an_e_that_is_not_an_exponent_does_not_eat_the_name() {
        // `2elements` is a number followed by a name, not a bad exponent.
        assert_eq!(
            toks("2elements"),
            vec![
                Tok::Number(2.0),
                Tok::Ident("elements".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn ampersand_is_concatenation_unless_it_starts_a_literal() {
        assert_eq!(
            toks("a & b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Punct(Punct::Amp),
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
        assert_eq!(
            toks("a &H10"),
            vec![
                Tok::Ident("a".into()),
                Tok::Number(16.0),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn a_number_followed_by_a_member_is_not_a_decimal_point() {
        assert_eq!(
            toks("x.Foo"),
            vec![
                Tok::Ident("x".into()),
                Tok::Punct(Punct::Dot),
                Tok::Ident("Foo".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn comparison_operators() {
        assert_eq!(
            toks("< > <= >= <> = =< =>"),
            vec![
                Tok::Punct(Punct::Lt),
                Tok::Punct(Punct::Gt),
                Tok::Punct(Punct::Le),
                Tok::Punct(Punct::Ge),
                Tok::Punct(Punct::Ne),
                Tok::Punct(Punct::Eq),
                Tok::Punct(Punct::Le),
                Tok::Punct(Punct::Ge),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn keywords_are_just_identifiers_here() {
        // The parser decides. A member called `Error` or `Property` is legal
        // and tables have them.
        let t = toks("x.Error");
        assert_eq!(t[2], Tok::Ident("Error".into()));
    }

    #[test]
    fn lines_are_counted_for_error_messages() {
        let t = lex("a\n\nb\n_ \nc").unwrap();
        let b = t.iter().find(|t| t.is_kw("b")).unwrap();
        assert_eq!(b.line, 3);
        let c = t.iter().find(|t| t.is_kw("c")).unwrap();
        assert_eq!(c.line, 5);
    }

    #[test]
    fn a_bare_carriage_return_ends_a_line() {
        // Terminator 2's script is written this way. Treating `\r` as
        // whitespace collapses the whole file onto one line.
        assert_eq!(
            toks("a\rb"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eol,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
        // And `\r\n` is still one ending.
        assert_eq!(
            toks("a\r\nb"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eol,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
        // A comment ends at a bare `\r` too.
        assert_eq!(
            toks("a ' note\rb"),
            vec![
                Tok::Ident("a".into()),
                Tok::Eol,
                Tok::Ident("b".into()),
                Tok::Eol,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn non_ascii_survives_a_round_trip() {
        assert_eq!(
            toks("\"añejo\""),
            vec![Tok::Str("añejo".into()), Tok::Eol, Tok::Eof]
        );
    }
}
