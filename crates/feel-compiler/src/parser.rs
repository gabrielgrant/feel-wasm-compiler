//! Recursive-descent / precedence-climbing parser for FEEL (DMN 1.5 clause
//! 10.3.1.2) with scope-aware multi-word name recognition (10.3.1.4, 10.3.1.6).
//!
//! Precedence, lowest to highest (rule 2 and the "additional syntax rules"):
//! `for`/`if`/quantified/`function` < `or` < `and` < comparison (`= != < <= >
//! >= between in instance of`) < `+ -` < `* /` < `**` (left associative) <
//! unary `-` < postfix (path, filter, invocation) < primary.
//!
//! Note: the textual order of rule 2 lists `instance of` after arithmetic; like
//! the reference implementations we parse it at comparison level, which only
//! differs for ill-typed inputs such as `1 + 2 instance of number`.

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{tokenize, Tok, Token};
use std::collections::HashSet;

/// Words that are literal terminal symbols of the grammar. A name may not
/// start with one (10.3.1.4); unknown multi-word names never contain one.
pub const KEYWORDS: &[&str] = &[
    "and",
    "or",
    "in",
    "for",
    "return",
    "if",
    "then",
    "else",
    "some",
    "every",
    "satisfies",
    "instance",
    "of",
    "between",
    "function",
    "external",
    "null",
    "true",
    "false",
];

pub fn is_keyword(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

/// The set of names in scope, used to recognise multi-word names.
#[derive(Clone, Debug, Default)]
pub struct NameSet {
    full: HashSet<Vec<String>>,
    prefixes: HashSet<Vec<String>>,
}

impl NameSet {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert_tokens(&mut self, toks: &[String]) {
        if toks.is_empty() {
            return;
        }
        for i in 1..=toks.len() {
            self.prefixes.insert(toks[..i].to_vec());
        }
        self.full.insert(toks.to_vec());
    }
    /// Insert a name given as text (it is tokenized with the FEEL lexer).
    pub fn insert(&mut self, name: &str) {
        let toks = name_tokens(name);
        self.insert_tokens(&toks);
    }
    pub fn contains(&self, toks: &[String]) -> bool {
        self.full.contains(toks)
    }
    pub fn has_prefix(&self, toks: &[String]) -> bool {
        self.prefixes.contains(toks)
    }
    pub fn len(&self) -> usize {
        self.full.len()
    }
    pub fn is_empty(&self) -> bool {
        self.full.is_empty()
    }
}

/// Tokenize a name string into its name tokens. Falls back to splitting on
/// white space if the text is not lexable.
pub fn name_tokens(name: &str) -> Vec<String> {
    match tokenize(name) {
        Ok(ts) => ts
            .into_iter()
            .filter(|t| t.tok != Tok::Eof)
            .map(|t| t.name_text())
            .collect(),
        Err(_) => name.split_whitespace().map(|s| s.to_string()).collect(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseOptions {
    /// Names visible to the expression (inputs, DRG elements, host functions).
    pub names: NameSet,
    /// Known context field names (used after `.`).
    pub fields: NameSet,
    /// Known type names besides the built-in ones (e.g. item definitions).
    pub type_names: NameSet,
}

pub struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    next_id: NodeId,
    scopes: Vec<NameSet>,
    global: &'a ParseOptions,
    builtins: &'a NameSet,
    /// When > 0, a `[` does not start a filter (used for interval end points
    /// such as `[1..10[`) and `]`/`)` are not consumed as postfix.
    no_bracket_postfix: u32,
    pub diagnostics: Vec<Diagnostic>,
}

type PResult<T> = Result<T, Diagnostic>;

fn builtin_type_names() -> NameSet {
    let mut s = NameSet::new();
    for n in [
        "number",
        "string",
        "boolean",
        "date",
        "time",
        "date and time",
        "dateTime",
        "days and time duration",
        "years and months duration",
        "dayTimeDuration",
        "yearMonthDuration",
        "duration",
        "Any",
        "Null",
        "list",
        "context",
        "range",
        "function",
    ] {
        s.insert(n);
    }
    s
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str, opts: &'a ParseOptions, builtins: &'a NameSet) -> PResult<Parser<'a>> {
        let toks = tokenize(src)?;
        Ok(Parser {
            src,
            toks,
            pos: 0,
            next_id: 0,
            scopes: vec![NameSet::new()],
            global: opts,
            builtins,
            no_bracket_postfix: 0,
            diagnostics: Vec::new(),
        })
    }

    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn peek_at(&self, n: usize) -> &Token {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i]
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at_sym(&self, s: &str) -> bool {
        self.peek().is_sym(s)
    }
    fn at_word(&self, s: &str) -> bool {
        self.peek().is_word(s)
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.at_sym(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, s: &str) -> bool {
        if self.at_word(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> PResult<Span> {
        if self.at_sym(s) {
            Ok(self.bump().span)
        } else {
            Err(self.err_expected(&format!("'{s}'")))
        }
    }
    fn expect_word(&mut self, s: &str) -> PResult<Span> {
        if self.at_word(s) {
            Ok(self.bump().span)
        } else {
            Err(self.err_expected(&format!("'{s}'")))
        }
    }
    fn err_expected(&self, what: &str) -> Diagnostic {
        let t = self.peek();
        let found = match &t.tok {
            Tok::Eof => "end of input".to_string(),
            Tok::Word(w) => format!("'{w}'"),
            Tok::Number(n) => format!("number {n}"),
            Tok::Str(_) => "string literal".to_string(),
            Tok::Sym(s) => format!("'{s}'"),
        };
        Diagnostic::error(
            "FEEL-S010",
            format!("expected {what}, found {found}"),
            t.span,
        )
    }
    fn mk(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = self.next_id;
        self.next_id += 1;
        Expr { id, span, kind }
    }
    fn last_span(&self) -> Span {
        if self.pos == 0 {
            Span::new(0, 0)
        } else {
            self.toks[self.pos - 1].span
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(NameSet::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn declare(&mut self, n: &Name) {
        self.scopes.last_mut().unwrap().insert_tokens(&n.tokens);
    }
    fn known_full(&self, toks: &[String]) -> bool {
        self.scopes.iter().any(|s| s.contains(toks))
            || self.global.names.contains(toks)
            || self.builtins.contains(toks)
    }
    fn known_prefix(&self, toks: &[String]) -> bool {
        self.scopes.iter().any(|s| s.has_prefix(toks))
            || self.global.names.has_prefix(toks)
            || self.builtins.has_prefix(toks)
    }

    // ---------------------------------------------------------------- entry points

    pub fn parse_expression_program(&mut self) -> PResult<Expr> {
        let e = self.parse_expr()?;
        if !matches!(self.peek().tok, Tok::Eof) {
            return Err(self.err_expected("end of expression"));
        }
        Ok(e)
    }

    pub fn parse_unary_tests_program(&mut self) -> PResult<UnaryTests> {
        // rule 15.c: "-"
        if self.at_sym("-") && matches!(self.peek_at(1).tok, Tok::Eof) {
            self.bump();
            return Ok(UnaryTests::Any);
        }
        self.push_scope();
        self.declare(&Name::simple("?"));
        // rule 15.b: not( positive unary tests )
        let res = if self.at_word("not") && self.peek_at(1).is_sym("(") {
            let save = self.pos;
            self.bump();
            self.bump();
            let tests = self.parse_positive_unary_tests()?;
            if self.eat_sym(")") && matches!(self.peek().tok, Tok::Eof) {
                UnaryTests::Negated(tests)
            } else {
                // e.g. `not(x) = y`, a positive test that starts with a call to not()
                self.pos = save;
                UnaryTests::Positive(self.parse_positive_unary_tests()?)
            }
        } else {
            UnaryTests::Positive(self.parse_positive_unary_tests()?)
        };
        self.pop_scope();
        if !matches!(self.peek().tok, Tok::Eof) {
            return Err(self.err_expected("',' or end of unary tests"));
        }
        Ok(res)
    }

    fn parse_positive_unary_tests(&mut self) -> PResult<Vec<UnaryTest>> {
        let mut v = vec![self.parse_positive_unary_test()?];
        while self.eat_sym(",") {
            v.push(self.parse_positive_unary_test()?);
        }
        Ok(v)
    }

    /// rule 13/7: a positive unary test.
    fn parse_positive_unary_test(&mut self) -> PResult<UnaryTest> {
        let start = self.peek().span;
        let op = if self.at_sym("<") {
            Some(CmpOp::Lt)
        } else if self.at_sym("<=") {
            Some(CmpOp::Le)
        } else if self.at_sym(">") {
            Some(CmpOp::Gt)
        } else if self.at_sym(">=") {
            Some(CmpOp::Ge)
        } else if self.at_sym("=") {
            Some(CmpOp::Eq)
        } else if self.at_sym("!=") {
            Some(CmpOp::Ne)
        } else {
            None
        };
        if let Some(op) = op {
            self.bump();
            let e = self.parse_endpoint()?;
            let span = start.to(e.span);
            return Ok(UnaryTest::Compare(op, e, span));
        }
        let e = self.parse_expr()?;
        if let ExprKind::Interval(_) = &e.kind {
            let span = e.span;
            if let ExprKind::Interval(iv) = e.kind {
                return Ok(UnaryTest::Interval(iv, span));
            }
        }
        if let ExprKind::UnaryRange(op, inner) = e.kind {
            let span = e.span;
            return Ok(UnaryTest::Compare(op, *inner, span));
        }
        let q = e.mentions_q();
        Ok(UnaryTest::Expr(e, q))
    }

    /// rule 16: endpoint. We parse at additive level so that `< 5, > 10` and
    /// `< 5 and ...` are handled as expected.
    fn parse_endpoint(&mut self) -> PResult<Expr> {
        self.parse_additive()
    }

    // ---------------------------------------------------------------- expressions

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        if let Tok::Word(w) = &self.peek().tok {
            match w.as_str() {
                "for" if !self.name_starts_here() => return self.parse_for(),
                "if" if !self.name_starts_here() => return self.parse_if(),
                "some" | "every" if self.is_quantifier_start() => return self.parse_quantified(),
                "function" if self.peek_at(1).is_sym("(") => return self.parse_function_def(),
                _ => {}
            }
        }
        self.parse_or()
    }

    /// Whether a known multi-word name starts at the current (keyword) token.
    fn name_starts_here(&self) -> bool {
        let mut toks = Vec::new();
        let mut j = self.pos;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if !Self::tok_can_be_in_name(t) {
                break;
            }
            toks.push(t.name_text());
            if !self.known_prefix(&toks) {
                break;
            }
            if toks.len() > 1 && self.known_full(&toks) {
                return true;
            }
            j += 1;
        }
        false
    }

    fn is_quantifier_start(&self) -> bool {
        // some <name> in ...
        matches!(self.peek_at(1).tok, Tok::Word(_)) && !self.name_starts_here()
    }

    fn parse_for(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        let mut its = Vec::new();
        self.push_scope();
        loop {
            let (name, nspan) = self.parse_binder_name(&["in"])?;
            self.expect_word("in")?;
            let first = self.parse_iteration_operand()?;
            let ic = if self.eat_sym("..") {
                let second = self.parse_iteration_operand()?;
                IterCtx::Range(first, second)
            } else {
                IterCtx::Expr(first)
            };
            self.declare(&name);
            its.push((name, nspan, ic));
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_word("return")?;
        self.declare(&Name::simple("partial"));
        let body = self.parse_expr()?;
        self.pop_scope();
        let span = start.to(body.span);
        Ok(self.mk(ExprKind::For(its, Box::new(body)), span))
    }

    fn parse_iteration_operand(&mut self) -> PResult<Expr> {
        self.parse_expr()
    }

    fn parse_if(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        let c = self.parse_expr()?;
        self.expect_word("then")?;
        let t = self.parse_expr()?;
        self.expect_word("else")?;
        let e = self.parse_expr()?;
        let span = start.to(e.span);
        Ok(self.mk(ExprKind::If(Box::new(c), Box::new(t), Box::new(e)), span))
    }

    fn parse_quantified(&mut self) -> PResult<Expr> {
        let tok = self.bump();
        let q = if tok.is_word("some") {
            Quantifier::Some
        } else {
            Quantifier::Every
        };
        let mut its = Vec::new();
        self.push_scope();
        loop {
            let (name, nspan) = self.parse_binder_name(&["in"])?;
            self.expect_word("in")?;
            let e = self.parse_expr()?;
            self.declare(&name);
            its.push((name, nspan, e));
            if self.eat_sym(",") {
                continue;
            }
            if matches!(self.peek().tok, Tok::Word(ref w) if !is_keyword(w))
                && self.peek_at(1).is_word("in")
            {
                continue;
            }
            break;
        }
        self.expect_word("satisfies")?;
        let body = self.parse_expr()?;
        self.pop_scope();
        let span = tok.span.to(body.span);
        Ok(self.mk(ExprKind::Quantified(q, its, Box::new(body)), span))
    }

    /// A binder name (for/some/every variable, parameter): tokens up to one of
    /// the given terminator keywords or a structural symbol.
    fn parse_binder_name(&mut self, stop_words: &[&str]) -> PResult<(Name, Span)> {
        let start = self.peek().span;
        let mut toks: Vec<Token> = Vec::new();
        loop {
            let t = self.peek();
            match &t.tok {
                Tok::Word(w) if stop_words.contains(&w.as_str()) => break,
                Tok::Word(_) | Tok::Number(_) => {}
                Tok::Sym(s) if is_name_symbol(s) => {}
                _ => break,
            }
            if toks.is_empty() && matches!(&t.tok, Tok::Word(w) if is_keyword(w)) {
                break;
            }
            toks.push(self.bump());
        }
        if toks.is_empty() {
            return Err(Diagnostic::error("FEEL-S011", "expected a name", start));
        }
        Ok(self.name_from_tokens(&toks))
    }

    fn name_from_tokens(&self, toks: &[Token]) -> (Name, Span) {
        let span = toks[0].span.to(toks[toks.len() - 1].span);
        let raw = &self.src[span.start..span.end];
        let text = collapse_ws(raw);
        (
            Name {
                text,
                tokens: toks.iter().map(|t| t.name_text()).collect(),
            },
            span,
        )
    }

    fn parse_function_def(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        self.expect_sym("(")?;
        let mut params = Vec::new();
        self.push_scope();
        if !self.at_sym(")") {
            loop {
                let mut toks = Vec::new();
                while !(self.at_sym(":")
                    || self.at_sym(",")
                    || self.at_sym(")")
                    || matches!(self.peek().tok, Tok::Eof))
                {
                    toks.push(self.bump());
                }
                if toks.is_empty() {
                    return Err(Diagnostic::error(
                        "FEEL-S011",
                        "expected a parameter name",
                        self.peek().span,
                    ));
                }
                let (name, span) = self.name_from_tokens(&toks);
                let ty = if self.eat_sym(":") {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.declare(&name);
                params.push(Param { name, span, ty });
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        self.expect_sym(")")?;
        let external = self.eat_word("external");
        let body = self.parse_expr()?;
        self.pop_scope();
        let span = start.to(body.span);
        Ok(self.mk(
            ExprKind::FunctionDef(FunctionDef {
                params,
                body: Box::new(body),
                external,
            }),
            span,
        ))
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut l = self.parse_and()?;
        while self.at_word("or") {
            self.bump();
            let r = self.parse_and()?;
            let span = l.span.to(r.span);
            l = self.mk(ExprKind::Or(Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut l = self.parse_comparison()?;
        while self.at_word("and") {
            self.bump();
            let r = self.parse_comparison()?;
            let span = l.span.to(r.span);
            l = self.mk(ExprKind::And(Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut l = self.parse_additive()?;
        loop {
            let op = match &self.peek().tok {
                Tok::Sym("=") => Some(CmpOp::Eq),
                Tok::Sym("!=") => Some(CmpOp::Ne),
                Tok::Sym("<") => Some(CmpOp::Lt),
                Tok::Sym("<=") => Some(CmpOp::Le),
                Tok::Sym(">") => Some(CmpOp::Gt),
                Tok::Sym(">=") => Some(CmpOp::Ge),
                _ => None,
            };
            if let Some(op) = op {
                // `>` may close a type in `instance of list<...>`; not here.
                self.bump();
                let r = self.parse_additive()?;
                let span = l.span.to(r.span);
                l = self.mk(ExprKind::Compare(op, Box::new(l), Box::new(r)), span);
                continue;
            }
            if self.at_word("between") {
                self.bump();
                let lo = self.parse_additive()?;
                self.expect_word("and")?;
                let hi = self.parse_additive()?;
                let span = l.span.to(hi.span);
                l = self.mk(
                    ExprKind::Between(Box::new(l), Box::new(lo), Box::new(hi)),
                    span,
                );
                continue;
            }
            if self.at_word("in") {
                self.bump();
                let tests = self.parse_in_rhs()?;
                let end = tests.last().map(|t| t.span()).unwrap_or(l.span);
                let span = l.span.to(end).to(self.last_span());
                l = self.mk(ExprKind::In(Box::new(l), tests), span);
                continue;
            }
            if self.at_word("instance") && self.peek_at(1).is_word("of") {
                self.bump();
                self.bump();
                let ty = self.parse_type()?;
                let span = l.span.to(self.last_span());
                l = self.mk(ExprKind::InstanceOf(Box::new(l), ty), span);
                continue;
            }
            break;
        }
        Ok(l)
    }

    /// Right-hand side of `in`: a positive unary test, or a parenthesized
    /// list of positive unary tests (rule 49 c, d).
    fn parse_in_rhs(&mut self) -> PResult<Vec<UnaryTest>> {
        if self.at_sym("(") {
            let save = self.pos;
            let save_diag = self.diagnostics.len();
            // Try: "(" positive unary tests ")"
            self.bump();
            let attempt = (|| -> PResult<Vec<UnaryTest>> {
                let tests = self.parse_positive_unary_tests()?;
                self.expect_sym(")")?;
                Ok(tests)
            })();
            match attempt {
                Ok(tests) => {
                    // `x in (1..5]` would have failed above; `(a)` style is fine.
                    return Ok(tests);
                }
                Err(_) => {
                    self.pos = save;
                    self.diagnostics.truncate(save_diag);
                }
            }
        }
        Ok(vec![self.parse_positive_unary_test()?])
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut l = self.parse_multiplicative()?;
        loop {
            let op = if self.at_sym("+") {
                BinOp::Add
            } else if self.at_sym("-") {
                BinOp::Sub
            } else {
                break;
            };
            self.bump();
            let r = self.parse_multiplicative()?;
            let span = l.span.to(r.span);
            l = self.mk(ExprKind::Binary(op, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn parse_multiplicative(&mut self) -> PResult<Expr> {
        let mut l = self.parse_power()?;
        loop {
            let op = if self.at_sym("*") {
                BinOp::Mul
            } else if self.at_sym("/") {
                BinOp::Div
            } else {
                break;
            };
            self.bump();
            let r = self.parse_power()?;
            let span = l.span.to(r.span);
            l = self.mk(ExprKind::Binary(op, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn parse_power(&mut self) -> PResult<Expr> {
        let mut l = self.parse_unary()?;
        while self.at_sym("**") {
            let opspan = self.bump().span;
            let r = self.parse_unary()?;
            let neg_lhs = matches!(l.kind, ExprKind::Neg(_))
                || matches!(&l.kind, ExprKind::Number(n) if n.starts_with('-'));
            if neg_lhs {
                self.diagnostics.push(
                    Diagnostic::warning(
                        "FEEL-W201",
                        "arithmetic negation binds tighter than '**' in FEEL: -a ** b is (-a) ** b",
                        l.span.to(opspan),
                    )
                    .with_suggestion("add explicit parentheses, e.g. -(a ** b) or (-a) ** b"),
                );
            }
            let span = l.span.to(r.span);
            l = self.mk(ExprKind::Binary(BinOp::Pow, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        if self.at_sym("-") {
            let start = self.bump().span;
            let e = self.parse_unary()?;
            let span = start.to(e.span);
            // fold `-<number literal>` into the literal (rule 35 allows a sign)
            if let ExprKind::Number(n) = &e.kind {
                if !n.starts_with('-') {
                    let text = format!("-{n}");
                    return Ok(self.mk(ExprKind::Number(text), span));
                }
            }
            return Ok(self.mk(ExprKind::Neg(Box::new(e)), span));
        }
        if self.at_sym("+") {
            // tolerate unary plus on numeric literals ("+5"), as in rule 35 exponents
            if matches!(self.peek_at(1).tok, Tok::Number(_)) {
                self.bump();
            }
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            if self.at_sym(".") {
                self.bump();
                let (name, nspan) = self.parse_field_name()?;
                let span = e.span.to(nspan);
                e = self.mk(ExprKind::Path(Box::new(e), name), span);
                continue;
            }
            if self.at_sym("[") && self.no_bracket_postfix == 0 {
                self.bump();
                let saved = self.no_bracket_postfix;
                self.no_bracket_postfix = 0;
                self.push_scope();
                self.declare(&Name::simple("item"));
                let f = self.parse_expr();
                self.pop_scope();
                self.no_bracket_postfix = saved;
                let f = f?;
                let end = self.expect_sym("]")?;
                let span = e.span.to(end);
                e = self.mk(ExprKind::Filter(Box::new(e), Box::new(f)), span);
                continue;
            }
            if self.at_sym("(") {
                self.bump();
                let saved = self.no_bracket_postfix;
                self.no_bracket_postfix = 0;
                let args = self.parse_args();
                self.no_bracket_postfix = saved;
                let args = args?;
                let end = self.expect_sym(")")?;
                let span = e.span.to(end);
                e = self.mk(ExprKind::Call(Box::new(e), args), span);
                continue;
            }
            break;
        }
        Ok(e)
    }

    fn parse_field_name(&mut self) -> PResult<(Name, Span)> {
        // longest known field name, else join plain words
        let start = self.pos;
        let mut best: Option<usize> = None;
        let mut toks = Vec::new();
        let mut j = self.pos;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if !Self::tok_can_be_in_name(t) {
                break;
            }
            toks.push(t.name_text());
            if !self.global.fields.has_prefix(&toks) {
                break;
            }
            if self.global.fields.contains(&toks) {
                best = Some(j);
            }
            j += 1;
        }
        let end = match best {
            Some(b) => b + 1,
            None => {
                let mut k = start;
                while k < self.toks.len() {
                    match &self.toks[k].tok {
                        Tok::Word(w) if k == start || !is_keyword(w) => k += 1,
                        Tok::Number(n) if k > start && n.chars().all(|c| c.is_ascii_digit()) => {
                            k += 1
                        }
                        _ => break,
                    }
                }
                k
            }
        };
        if end == start {
            return Err(self.err_expected("a name after '.'"));
        }
        let toks: Vec<Token> = self.toks[start..end].to_vec();
        self.pos = end;
        Ok(self.name_from_tokens(&toks))
    }

    fn tok_can_be_in_name(t: &Token) -> bool {
        match &t.tok {
            Tok::Word(_) | Tok::Number(_) => true,
            Tok::Sym(s) => is_name_symbol(s),
            _ => false,
        }
    }

    fn parse_args(&mut self) -> PResult<Args> {
        if self.at_sym(")") {
            return Ok(Args::Positional(Vec::new()));
        }
        if self.looks_like_named_arg() {
            let mut v = Vec::new();
            loop {
                let mut toks = Vec::new();
                while !self.at_sym(":") {
                    if matches!(self.peek().tok, Tok::Eof) {
                        return Err(self.err_expected("':'"));
                    }
                    toks.push(self.bump());
                }
                self.bump();
                let (name, span) = self.name_from_tokens(&toks);
                let e = self.parse_expr()?;
                v.push((name, span, e));
                if !self.eat_sym(",") {
                    break;
                }
                if !self.looks_like_named_arg() {
                    return Err(Diagnostic::error(
                        "FEEL-S012",
                        "cannot mix named and positional arguments",
                        self.peek().span,
                    ));
                }
            }
            Ok(Args::Named(v))
        } else {
            let mut v = vec![self.parse_expr()?];
            while self.eat_sym(",") {
                v.push(self.parse_expr()?);
            }
            Ok(Args::Positional(v))
        }
    }

    fn looks_like_named_arg(&self) -> bool {
        let mut j = self.pos;
        let mut n = 0;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if t.is_sym(":") {
                return n > 0;
            }
            if !Self::tok_can_be_in_name(t) {
                return false;
            }
            n += 1;
            j += 1;
        }
        false
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Number(n) => {
                self.bump();
                Ok(self.mk(ExprKind::Number(n.clone()), t.span))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(self.mk(ExprKind::String(s.clone()), t.span))
            }
            Tok::Sym("@") => {
                self.bump();
                let st = self.peek().clone();
                if let Tok::Str(s) = &st.tok {
                    self.bump();
                    Ok(self.mk(ExprKind::At(s.clone()), t.span.to(st.span)))
                } else {
                    Err(self.err_expected("a string literal after '@'"))
                }
            }
            Tok::Sym("(") => {
                self.bump();
                let saved = self.no_bracket_postfix;
                self.no_bracket_postfix = 0;
                let first = self.parse_expr();
                self.no_bracket_postfix = saved;
                let first = first?;
                if self.at_sym("..") {
                    return self.finish_interval(t.span, false, first);
                }
                self.expect_sym(")")?;
                // keep the parenthesized span
                let mut e = first;
                e.span = t.span.to(self.last_span());
                Ok(e)
            }
            Tok::Sym("]") => {
                self.bump();
                let first = self.parse_expr()?;
                if !self.at_sym("..") {
                    return Err(self.err_expected("'..'"));
                }
                self.finish_interval(t.span, false, first)
            }
            Tok::Sym("[") => {
                self.bump();
                if self.at_sym("]") {
                    let end = self.bump().span;
                    return Ok(self.mk(ExprKind::List(Vec::new()), t.span.to(end)));
                }
                let saved = self.no_bracket_postfix;
                self.no_bracket_postfix = 0;
                let first = self.parse_expr();
                self.no_bracket_postfix = saved;
                let first = first?;
                if self.at_sym("..") {
                    return self.finish_interval(t.span, true, first);
                }
                let mut items = vec![first];
                while self.eat_sym(",") {
                    let saved = self.no_bracket_postfix;
                    self.no_bracket_postfix = 0;
                    let e = self.parse_expr();
                    self.no_bracket_postfix = saved;
                    items.push(e?);
                }
                let end = self.expect_sym("]")?;
                Ok(self.mk(ExprKind::List(items), t.span.to(end)))
            }
            Tok::Sym("{") => self.parse_context(),
            Tok::Sym("<")
            | Tok::Sym("<=")
            | Tok::Sym(">")
            | Tok::Sym(">=")
            | Tok::Sym("=")
            | Tok::Sym("!=") => {
                let op = match &t.tok {
                    Tok::Sym("<") => CmpOp::Lt,
                    Tok::Sym("<=") => CmpOp::Le,
                    Tok::Sym(">") => CmpOp::Gt,
                    Tok::Sym("=") => CmpOp::Eq,
                    Tok::Sym("!=") => CmpOp::Ne,
                    _ => CmpOp::Ge,
                };
                self.bump();
                let e = self.parse_endpoint()?;
                let span = t.span.to(e.span);
                Ok(self.mk(ExprKind::UnaryRange(op, Box::new(e)), span))
            }
            Tok::Word(w) => {
                match w.as_str() {
                    "null" => {
                        self.bump();
                        return Ok(self.mk(ExprKind::Null, t.span));
                    }
                    "true" => {
                        self.bump();
                        return Ok(self.mk(ExprKind::Bool(true), t.span));
                    }
                    "false" => {
                        self.bump();
                        return Ok(self.mk(ExprKind::Bool(false), t.span));
                    }
                    "if" | "for" | "some" | "every" | "function" if !self.name_starts_here() => {
                        return self.parse_expr();
                    }
                    _ => {}
                }
                let (name, span) = self.parse_name()?;
                Ok(self.mk(ExprKind::Name(name), span))
            }
            _ => Err(self.err_expected("an expression")),
        }
    }

    fn finish_interval(
        &mut self,
        start_span: Span,
        start_incl: bool,
        first: Expr,
    ) -> PResult<Expr> {
        self.expect_sym("..")?;
        self.no_bracket_postfix += 1;
        let second = self.parse_expr();
        self.no_bracket_postfix -= 1;
        let second = second?;
        let end_incl = if self.at_sym("]") {
            true
        } else if self.at_sym(")") || self.at_sym("[") {
            false
        } else {
            return Err(self.err_expected("']', ')' or '[' to close the interval"));
        };
        let end = self.bump().span;
        let span = start_span.to(end);
        let iv = Interval {
            start: first,
            end: second,
            start_incl,
            end_incl,
        };
        Ok(self.mk(ExprKind::Interval(Box::new(iv)), span))
    }

    fn parse_context(&mut self) -> PResult<Expr> {
        let start = self.bump().span;
        let mut entries: Vec<ContextEntry> = Vec::new();
        self.push_scope();
        let saved = self.no_bracket_postfix;
        self.no_bracket_postfix = 0;
        let r = (|| -> PResult<()> {
            if self.at_sym("}") {
                return Ok(());
            }
            loop {
                let kt = self.peek().clone();
                let (key, key_span, key_name) = if let Tok::Str(s) = &kt.tok {
                    self.bump();
                    (
                        s.clone(),
                        kt.span,
                        Name {
                            text: s.clone(),
                            tokens: name_tokens(s),
                        },
                    )
                } else {
                    let mut toks = Vec::new();
                    while !self.at_sym(":") {
                        if matches!(self.peek().tok, Tok::Eof)
                            || self.at_sym("}")
                            || self.at_sym(",")
                        {
                            return Err(self.err_expected("':' after context key"));
                        }
                        toks.push(self.bump());
                    }
                    if toks.is_empty() {
                        return Err(self.err_expected("a context key"));
                    }
                    let (n, s) = self.name_from_tokens(&toks);
                    (n.text.clone(), s, n)
                };
                self.expect_sym(":")?;
                let value = self.parse_expr()?;
                if entries.iter().any(|e| e.key == key) {
                    self.diagnostics.push(Diagnostic::error(
                        "FEEL-S020",
                        format!("duplicate context key '{key}'"),
                        key_span,
                    ));
                }
                self.declare(&key_name);
                entries.push(ContextEntry {
                    key,
                    key_span,
                    value,
                });
                if !self.eat_sym(",") {
                    break;
                }
            }
            Ok(())
        })();
        self.no_bracket_postfix = saved;
        self.pop_scope();
        r?;
        let end = self.expect_sym("}")?;
        Ok(self.mk(ExprKind::Context(entries), start.to(end)))
    }

    /// Name recognition (10.3.1.6): longest match against names in scope,
    /// otherwise a run of plain (non-keyword) word tokens.
    fn parse_name(&mut self) -> PResult<(Name, Span)> {
        let start = self.pos;
        let mut best: Option<usize> = None;
        let mut toks = Vec::new();
        let mut j = self.pos;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if !Self::tok_can_be_in_name(t) {
                break;
            }
            toks.push(t.name_text());
            if !self.known_prefix(&toks) {
                break;
            }
            if self.known_full(&toks) {
                best = Some(j);
            }
            j += 1;
        }
        let end = match best {
            Some(b) => b + 1,
            None => {
                let mut k = start;
                while k < self.toks.len() {
                    match &self.toks[k].tok {
                        Tok::Word(w) if !is_keyword(w) => k += 1,
                        Tok::Number(n) if k > start && n.chars().all(|c| c.is_ascii_digit()) => {
                            k += 1
                        }
                        _ => break,
                    }
                }
                k
            }
        };
        if end == start {
            return Err(self.err_expected("a name"));
        }
        if let Tok::Word(w) = &self.toks[start].tok {
            if is_keyword(w) && best.is_none() {
                return Err(self.err_expected("an expression"));
            }
        }
        let toks: Vec<Token> = self.toks[start..end].to_vec();
        self.pos = end;
        Ok(self.name_from_tokens(&toks))
    }

    // ---------------------------------------------------------------- types

    pub fn parse_type(&mut self) -> PResult<TypeExpr> {
        let t = self.peek().clone();
        if let Tok::Word(w) = &t.tok {
            let lower = w.as_str();
            if (lower == "list" || lower == "range") && self.peek_at(1).is_sym("<") {
                self.bump();
                self.bump();
                let inner = self.parse_type()?;
                self.expect_sym(">")?;
                return Ok(if lower == "list" {
                    TypeExpr::List(Box::new(inner))
                } else {
                    TypeExpr::Range(Box::new(inner))
                });
            }
            if lower == "context" && self.peek_at(1).is_sym("<") {
                self.bump();
                self.bump();
                let mut entries = Vec::new();
                if !self.at_sym(">") {
                    loop {
                        let mut toks = Vec::new();
                        while !self.at_sym(":") {
                            if matches!(self.peek().tok, Tok::Eof) || self.at_sym(">") {
                                return Err(self.err_expected("':' in context type"));
                            }
                            toks.push(self.bump());
                        }
                        self.bump();
                        let key = if toks.len() == 1 {
                            if let Tok::Str(s) = &toks[0].tok {
                                s.clone()
                            } else {
                                self.name_from_tokens(&toks).0.text
                            }
                        } else {
                            self.name_from_tokens(&toks).0.text
                        };
                        let ty = self.parse_type()?;
                        entries.push((key, ty));
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                self.expect_sym(">")?;
                return Ok(TypeExpr::Context(entries));
            }
            if lower == "function" && self.peek_at(1).is_sym("<") {
                self.bump();
                self.bump();
                let mut params = Vec::new();
                if !self.at_sym(">") {
                    loop {
                        params.push(self.parse_type()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                self.expect_sym(">")?;
                // "->"
                if !self.eat_sym("->") {
                    return Err(self.err_expected("'->' in function type"));
                }
                let ret = self.parse_type()?;
                return Ok(TypeExpr::Function(params, Box::new(ret)));
            }
        }
        // qualified type name: longest known type name, else a dotted word sequence
        let builtin_types = builtin_type_names();
        let start = self.pos;
        let mut best: Option<usize> = None;
        let mut toks = Vec::new();
        let mut j = self.pos;
        while j < self.toks.len() {
            let t = &self.toks[j];
            if !matches!(t.tok, Tok::Word(_)) && !t.is_sym(".") && !t.is_sym("-") {
                break;
            }
            toks.push(t.name_text());
            let pre = builtin_types.has_prefix(&toks) || self.global.type_names.has_prefix(&toks);
            if !pre {
                break;
            }
            if builtin_types.contains(&toks) || self.global.type_names.contains(&toks) {
                best = Some(j);
            }
            j += 1;
        }
        let end = match best {
            Some(b) => b + 1,
            None => {
                // a (possibly dotted) qualified name of single words
                let mut k = start;
                if matches!(self.toks[k].tok, Tok::Word(_)) {
                    k += 1;
                    while self.toks[k].is_sym(".") && matches!(self.toks[k + 1].tok, Tok::Word(_)) {
                        k += 2;
                    }
                }
                k
            }
        };
        if end == start {
            return Err(self.err_expected("a type"));
        }
        let toks: Vec<Token> = self.toks[start..end].to_vec();
        self.pos = end;
        let (n, span) = self.name_from_tokens(&toks);
        Ok(TypeExpr::Named(n.text, span))
    }

    pub fn source(&self) -> &str {
        self.src
    }
}

/// Grammar rule 30 additional name symbols, plus ASCII apostrophe.
pub fn is_name_symbol(s: &str) -> bool {
    matches!(s, "." | "/" | "-" | "\u{2019}" | "'" | "+" | "*")
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if crate::lexer::is_feel_whitespace(c) {
            in_ws = true;
        } else {
            if in_ws && !out.is_empty() {
                out.push(' ');
            }
            in_ws = false;
            out.push(c);
        }
    }
    out
}
