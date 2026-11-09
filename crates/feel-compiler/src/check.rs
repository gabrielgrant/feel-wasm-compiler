//! Optional, schema-aware type checker (CEL-style parse → check → evaluate).
//!
//! The checker never changes the semantics of compiled code: the code
//! generator does not consume its results except to decide whether
//! compilation is allowed (strict policy). Findings carry stable codes:
//!
//! | code | finding |
//! |------|---------|
//! | FEEL-C101 | unknown name |
//! | FEEL-C102 | unknown context field / property |
//! | FEEL-C103 | arithmetic operator applied to incompatible types (always null) |
//! | FEEL-C104 | wrong number of arguments / unknown named parameter |
//! | FEEL-C105 | argument type mismatch |
//! | FEEL-C106 | non-boolean condition / logical operand |
//! | FEEL-C107 | comparison of incompatible types (always null) |
//! | FEEL-C108 | result does not conform to the expected type |
//! | FEEL-C109 | unary test incompatible with the candidate type |
//! | FEEL-C110 | invocation of a non-function |
//! | FEEL-C111 | unknown type name |
//! | FEEL-W201 | `-a ** b` precedence (from the parser) |
//! | FEEL-W210 | possibly-null operand (strict policy only) |
//! | FEEL-W211 | filter on a value that is not a list |

use crate::ast::*;
use crate::diag::{Diagnostic, Severity, Span};
use crate::types::{named, parse_schema_type, Field, Ty};
use feel_runtime::builtins;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CheckMode {
    Off,
    #[default]
    Permissive,
    Strict,
}

pub struct CheckEnv<'a> {
    pub inputs: Vec<(Vec<String>, String, Ty)>,
    pub functions: Vec<(Vec<String>, String, Ty, Vec<String>)>,
    pub aliases: &'a [(String, String)],
    pub closed: bool,
    pub extensions: u32,
    pub candidate: Ty,
    pub expected: Option<Ty>,
    pub mode: CheckMode,
}

pub struct CheckOutput {
    pub result: Ty,
    pub diagnostics: Vec<Diagnostic>,
    pub types: HashMap<NodeId, Ty>,
}

struct Checker<'a> {
    env: &'a CheckEnv<'a>,
    scopes: Vec<Vec<(Vec<String>, String, Ty)>>,
    /// narrowed (non-null) names / paths
    narrowed: Vec<Vec<String>>,
    diags: Vec<Diagnostic>,
    types: HashMap<NodeId, Ty>,
}

fn toks(s: &str) -> Vec<String> {
    crate::parser::name_tokens(s)
}

pub fn check_program(p: &Program, env: &CheckEnv) -> CheckOutput {
    let mut c = Checker {
        env,
        scopes: vec![Vec::new()],
        narrowed: Vec::new(),
        diags: Vec::new(),
        types: HashMap::new(),
    };
    let result = match p {
        Program::Expression(e) => c.expr(e),
        Program::UnaryTests(t, span) => {
            c.scopes
                .push(vec![(vec!["?".into()], "?".into(), env.candidate.clone())]);
            c.unary_tests(&env.candidate, t, *span);
            c.scopes.pop();
            Ty::Boolean
        }
    };
    if let (Some(exp), Program::Expression(e)) = (&env.expected, p) {
        if !result.compatible(exp) {
            c.err(
                "FEEL-C108",
                format!("result type {result} does not conform to the expected type {exp}"),
                e.span,
            );
        }
    }
    CheckOutput {
        result,
        diagnostics: c.diags,
        types: c.types,
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) {
                0
            } else {
                1
            };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

pub fn suggest<'s>(name: &str, candidates: impl Iterator<Item = &'s str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = levenshtein(name, c);
        let lim = (name.chars().count().max(3)) / 3 + 1;
        if d <= lim && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c.to_string())
}

fn spec_ty(s: &str) -> Ty {
    parse_schema_type(s, &[]).unwrap_or(Ty::Any)
}

impl Checker<'_> {
    fn err(&mut self, code: &str, msg: impl Into<String>, span: Span) {
        let sev = if self.env.mode == CheckMode::Strict {
            Severity::Error
        } else {
            Severity::Warning
        };
        let mut d = Diagnostic::error(code, msg, span);
        d.severity = sev;
        self.diags.push(d);
    }
    fn err_s(&mut self, code: &str, msg: impl Into<String>, span: Span, sugg: Option<String>) {
        self.err(code, msg, span);
        if let Some(s) = sugg {
            self.diags.last_mut().unwrap().suggestion = Some(s);
        }
    }
    fn warn(&mut self, code: &str, msg: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic::warning(code, msg, span));
    }

    fn lookup(&self, t: &[String]) -> Option<Ty> {
        for s in self.scopes.iter().rev() {
            if let Some((_, _, ty)) = s.iter().rev().find(|(n, _, _)| n.as_slice() == t) {
                return Some(ty.clone());
            }
        }
        if let Some((_, _, ty)) = self.env.inputs.iter().find(|(n, _, _)| n.as_slice() == t) {
            return Some(ty.clone());
        }
        if let Some((_, _, ty, _)) = self
            .env
            .functions
            .iter()
            .find(|(n, _, _, _)| n.as_slice() == t)
        {
            return Some(ty.clone());
        }
        None
    }

    fn all_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in &self.scopes {
            v.extend(s.iter().map(|x| x.1.clone()));
        }
        v.extend(self.env.inputs.iter().map(|x| x.1.clone()));
        v.extend(self.env.functions.iter().map(|x| x.1.clone()));
        v.extend(
            builtins::BUILTINS
                .iter()
                .filter(|b| b.extension == 0 || self.env.extensions & b.extension != 0)
                .map(|b| b.name.to_string()),
        );
        v
    }

    fn is_narrowed(&self, key: &[String]) -> bool {
        self.narrowed.iter().any(|n| n.as_slice() == key)
    }

    /// The name/path key of an expression for narrowing (`a`, `a.b`).
    fn narrow_key(e: &Expr) -> Option<Vec<String>> {
        match &e.kind {
            ExprKind::Name(n) => Some(vec![n.text.clone()]),
            ExprKind::Path(b, n) => {
                let mut k = Self::narrow_key(b)?;
                k.push(n.text.clone());
                Some(k)
            }
            _ => None,
        }
    }

    /// Facts implied by `cond` being true (`positive`) or false.
    fn facts(e: &Expr, positive: bool, out: &mut Vec<Vec<String>>) {
        match &e.kind {
            ExprKind::Compare(op, a, b) => {
                let (other, is_null_test) = match (&a.kind, &b.kind) {
                    (_, ExprKind::Null) => (a, true),
                    (ExprKind::Null, _) => (b, true),
                    _ => (a, false),
                };
                if is_null_test {
                    if (*op == CmpOp::Ne && positive) || (*op == CmpOp::Eq && !positive) {
                        if let Some(k) = Self::narrow_key(other) {
                            out.push(k);
                        }
                    }
                } else if positive {
                    // a comparison that is true implies both operands are non-null
                    for x in [a, b] {
                        if let Some(k) = Self::narrow_key(x) {
                            out.push(k);
                        }
                    }
                }
            }
            ExprKind::And(a, b) if positive => {
                Self::facts(a, true, out);
                Self::facts(b, true, out);
            }
            ExprKind::Or(a, b) if !positive => {
                Self::facts(a, false, out);
                Self::facts(b, false, out);
            }
            _ => {}
        }
    }

    fn with_facts<T>(&mut self, facts: Vec<Vec<String>>, f: impl FnOnce(&mut Self) -> T) -> T {
        let n = self.narrowed.len();
        self.narrowed.extend(facts);
        let r = f(self);
        self.narrowed.truncate(n);
        r
    }

    fn expr(&mut self, e: &Expr) -> Ty {
        let t = self.expr_inner(e);
        self.types.insert(e.id, t.clone());
        t
    }

    fn possibly_null(&mut self, t: &Ty, e: &Expr, what: &str) {
        if self.env.mode == CheckMode::Strict && matches!(t, Ty::Nullable(_)) {
            let key = Self::narrow_key(e);
            let n = key.map(|k| k.join(".")).unwrap_or_else(|| "operand".into());
            self.warn(
                "FEEL-W210",
                format!("{n} may be null here; {what} then yields null"),
                e.span,
            );
            if let Some(d) = self.diags.last_mut() {
                d.suggestion = Some(format!("guard with `{n} != null`"));
            }
        }
    }

    fn expr_inner(&mut self, e: &Expr) -> Ty {
        match &e.kind {
            ExprKind::Null => Ty::Null,
            ExprKind::Bool(_) => Ty::Boolean,
            ExprKind::Number(_) => Ty::Number,
            ExprKind::String(_) => Ty::String,
            ExprKind::At(s) => {
                use feel_runtime::temporal as tm;
                if tm::Date::parse(s).is_some() {
                    Ty::Date
                } else if tm::DateTime::parse(s).is_some() {
                    Ty::DateTime
                } else if tm::Time::parse(s).is_some() {
                    Ty::Time
                } else {
                    match tm::parse_duration(s) {
                        Some(tm::Duration::DayTime(_)) => Ty::Dtd,
                        Some(tm::Duration::YearMonth(_)) => Ty::Ymd,
                        None => Ty::Null,
                    }
                }
            }
            ExprKind::Name(n) => match self.lookup(&n.tokens) {
                Some(t) => {
                    if self.is_narrowed(std::slice::from_ref(&n.text)) {
                        t.non_null()
                    } else {
                        t
                    }
                }
                None => {
                    if let Some(id) = builtins::lookup(&n.tokens.join(" ")) {
                        let ov = &builtins::overloads(id)[0];
                        return Ty::Function(
                            ov.params.iter().map(|p| spec_ty(&p.ty)).collect(),
                            Box::new(spec_ty(&ov.ret)),
                        );
                    }
                    if self.env.closed {
                        let names = self.all_names();
                        let s = suggest(&n.text, names.iter().map(|s| s.as_str()));
                        self.err_s(
                            "FEEL-C101",
                            format!("unknown name '{}'", n.text),
                            e.span,
                            s.map(|s| format!("did you mean '{s}'?")),
                        );
                    }
                    Ty::Any
                }
            },
            ExprKind::Path(b, n) => {
                let bt = self.expr(b);
                let key = Self::narrow_key(e);
                let t = self.select(&bt, &n.text, e.span);
                if key.map(|k| self.is_narrowed(&k)).unwrap_or(false) {
                    t.non_null()
                } else {
                    t
                }
            }
            ExprKind::Filter(l, p) => {
                let lt = self.expr(l);
                let elem = match lt.non_null() {
                    Ty::List(t) => *t,
                    Ty::Any => Ty::Any,
                    other => {
                        if lt.known().is_some() {
                            self.warn(
                                "FEEL-W211",
                                format!(
                                    "filtering a {} (treated as a singleton list)",
                                    other.kind()
                                ),
                                l.span,
                            );
                        }
                        other
                    }
                };
                let mut frame = vec![(vec!["item".to_string()], "item".to_string(), elem.clone())];
                if let Ty::Context(fs, _) = elem.non_null() {
                    for f in fs {
                        let ft = if f.optional {
                            f.ty.clone().nullable()
                        } else {
                            f.ty.clone()
                        };
                        frame.push((toks(&f.name), f.name.clone(), ft));
                    }
                }
                self.scopes.push(frame);
                let pt = self.expr(p);
                self.scopes.pop();
                match pt.non_null() {
                    Ty::Number => elem.nullable(),
                    Ty::Boolean | Ty::Any => Ty::List(Box::new(elem)),
                    other => {
                        self.err(
                            "FEEL-C106",
                            format!(
                                "filter expression is {}, not boolean or number",
                                other.kind()
                            ),
                            p.span,
                        );
                        Ty::List(Box::new(elem))
                    }
                }
            }
            ExprKind::Call(f, args) => self.call(f, args, e.span),
            ExprKind::Neg(x) => {
                let t = self.expr(x);
                self.possibly_null(&t, x, "negation");
                match t.non_null() {
                    Ty::Number | Ty::Dtd | Ty::Ymd => t,
                    Ty::Any | Ty::Null => Ty::Any,
                    other => {
                        self.err(
                            "FEEL-C103",
                            format!("negation of {} is always null", other.kind()),
                            e.span,
                        );
                        Ty::Null
                    }
                }
            }
            ExprKind::Binary(op, a, b) => {
                let ta = self.expr(a);
                let tb = self.expr(b);
                self.possibly_null(&ta, a, "the operation");
                self.possibly_null(&tb, b, "the operation");
                let r = arith_type(*op, &ta.non_null(), &tb.non_null());
                match r {
                    Some(t) => {
                        if ta.is_nullable() || tb.is_nullable() {
                            t.nullable()
                        } else {
                            t
                        }
                    }
                    None => {
                        let sym = match op {
                            BinOp::Add => "+",
                            BinOp::Sub => "-",
                            BinOp::Mul => "*",
                            BinOp::Div => "/",
                            BinOp::Pow => "**",
                        };
                        self.err(
                            "FEEL-C103",
                            format!(
                                "'{sym}' is not defined for {} and {}; the result is always null",
                                ta.kind(),
                                tb.kind()
                            ),
                            e.span,
                        );
                        Ty::Null
                    }
                }
            }
            ExprKind::Compare(op, a, b) => {
                let ta = self.expr(a);
                let tb = self.expr(b);
                let null_test =
                    matches!(a.kind, ExprKind::Null) || matches!(b.kind, ExprKind::Null);
                if !null_test {
                    let (x, y) = (ta.non_null(), tb.non_null());
                    let ok = match op {
                        CmpOp::Eq | CmpOp::Ne => comparable_eq(&x, &y),
                        _ => comparable_ord(&x, &y),
                    };
                    if !ok {
                        self.err(
                            "FEEL-C107",
                            format!(
                                "cannot compare {} with {} using '{}'; the result is always null",
                                ta.kind(),
                                tb.kind(),
                                op.as_str()
                            ),
                            e.span,
                        );
                    }
                }
                Ty::Boolean
            }
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                let ta = self.expr(a);
                let is_and = matches!(e.kind, ExprKind::And(..));
                let mut fs = Vec::new();
                Self::facts(a, is_and, &mut fs);
                let tb = self.with_facts(fs, |c| c.expr(b));
                for (t, x) in [(ta, a), (tb, b)] {
                    if !t.compatible(&Ty::Boolean) {
                        self.err(
                            "FEEL-C106",
                            format!("logical operand is {}, not boolean", t.kind()),
                            x.span,
                        );
                    }
                }
                Ty::Boolean.nullable()
            }
            ExprKind::Between(x, lo, hi) => {
                let t = self.expr(x);
                let a = self.expr(lo);
                let b = self.expr(hi);
                for o in [a, b] {
                    if !comparable_ord(&t.non_null(), &o.non_null()) {
                        self.err(
                            "FEEL-C107",
                            format!("cannot compare {} with {} in 'between'", t.kind(), o.kind()),
                            e.span,
                        );
                    }
                }
                Ty::Boolean
            }
            ExprKind::In(x, tests) => {
                let t = self.expr(x);
                self.scopes
                    .push(vec![(vec!["?".into()], "?".into(), t.clone())]);
                for ts in tests {
                    self.unary_test(&t, ts);
                }
                self.scopes.pop();
                Ty::Boolean
            }
            ExprKind::Tests(x, tests) => {
                let t = self.expr(x);
                self.scopes
                    .push(vec![(vec!["?".into()], "?".into(), t.clone())]);
                self.unary_tests(&t, tests, e.span);
                self.scopes.pop();
                Ty::Boolean
            }
            ExprKind::InstanceOf(x, ty) => {
                self.expr(x);
                self.type_expr(ty);
                Ty::Boolean
            }
            ExprKind::Coerce(x, ty) => {
                self.expr(x);
                self.type_expr(ty).nullable()
            }
            ExprKind::If(c, t, f) => {
                let tc = self.expr(c);
                if !tc.compatible(&Ty::Boolean) {
                    self.err(
                        "FEEL-C106",
                        format!("if condition is {}, not boolean", tc.kind()),
                        c.span,
                    );
                }
                let mut pos = Vec::new();
                Self::facts(c, true, &mut pos);
                let mut neg = Vec::new();
                Self::facts(c, false, &mut neg);
                let tt = self.with_facts(pos, |s| s.expr(t));
                let tf = self.with_facts(neg, |s| s.expr(f));
                tt.join(&tf)
            }
            ExprKind::For(its, body) => {
                let mut frame = Vec::new();
                for (n, _, ic) in its {
                    self.scopes.push(frame.clone());
                    let et = match ic {
                        IterCtx::Expr(x) => {
                            let t = self.expr(x);
                            match t.non_null() {
                                Ty::List(el) => *el,
                                Ty::Range(el) => *el,
                                Ty::Any => Ty::Any,
                                other => other,
                            }
                        }
                        IterCtx::Range(a, b) => {
                            let ta = self.expr(a);
                            let tb = self.expr(b);
                            for (t, x) in [(&ta, a), (&tb, b)] {
                                if !matches!(
                                    t.non_null(),
                                    Ty::Number | Ty::Date | Ty::Any | Ty::Null
                                ) {
                                    self.err(
                                        "FEEL-C105",
                                        format!(
                                            "iteration bound must be a number or date, not {}",
                                            t.kind()
                                        ),
                                        x.span,
                                    );
                                }
                            }
                            ta.non_null()
                        }
                    };
                    self.scopes.pop();
                    frame.push((n.tokens.clone(), n.text.clone(), et));
                }
                frame.push((
                    vec!["partial".into()],
                    "partial".into(),
                    Ty::List(Box::new(Ty::Any)),
                ));
                self.scopes.push(frame);
                let bt = self.expr(body);
                self.scopes.pop();
                Ty::List(Box::new(bt))
            }
            ExprKind::Quantified(_, its, body) => {
                let mut frame = Vec::new();
                for (n, _, x) in its {
                    self.scopes.push(frame.clone());
                    let t = self.expr(x);
                    self.scopes.pop();
                    let et = match t.non_null() {
                        Ty::List(el) => *el,
                        _ => Ty::Any,
                    };
                    frame.push((n.tokens.clone(), n.text.clone(), et));
                }
                self.scopes.push(frame);
                let bt = self.expr(body);
                self.scopes.pop();
                if !bt.compatible(&Ty::Boolean) {
                    self.err(
                        "FEEL-C106",
                        format!("'satisfies' expression is {}, not boolean", bt.kind()),
                        body.span,
                    );
                }
                Ty::Boolean.nullable()
            }
            ExprKind::List(items) => {
                let mut t: Option<Ty> = None;
                for it in items {
                    let x = self.expr(it);
                    t = Some(match t {
                        None => x,
                        Some(p) => p.join(&x),
                    });
                }
                Ty::List(Box::new(t.unwrap_or(Ty::Any)))
            }
            ExprKind::Context(entries) => {
                self.scopes.push(Vec::new());
                let mut fs = Vec::new();
                for en in entries {
                    if let ExprKind::FunctionDef(fd) = &en.value.kind {
                        let pre = Ty::Function(
                            fd.params.iter().map(|_| Ty::Any).collect(),
                            Box::new(Ty::Any),
                        );
                        self.scopes
                            .last_mut()
                            .unwrap()
                            .push((toks(&en.key), en.key.clone(), pre));
                    }
                    let t = self.expr(&en.value);
                    self.scopes.last_mut().unwrap().push((
                        toks(&en.key),
                        en.key.clone(),
                        t.clone(),
                    ));
                    fs.push(Field {
                        name: en.key.clone(),
                        ty: t,
                        optional: false,
                    });
                }
                self.scopes.pop();
                Ty::Context(fs, false)
            }
            ExprKind::FunctionDef(fd) => {
                let mut frame = Vec::new();
                let mut ps = Vec::new();
                for p in &fd.params {
                    let t = p.ty.as_ref().map(|t| self.type_expr(t)).unwrap_or(Ty::Any);
                    ps.push(t.clone());
                    frame.push((p.name.tokens.clone(), p.name.text.clone(), t));
                }
                self.scopes.push(frame);
                let bt = if fd.external {
                    self.expr(&fd.body);
                    Ty::Any
                } else {
                    self.expr(&fd.body)
                };
                self.scopes.pop();
                Ty::Function(ps, Box::new(bt))
            }
            ExprKind::Interval(iv) => {
                let a = self.expr(&iv.start);
                let b = self.expr(&iv.end);
                if !comparable_ord(&a.non_null(), &b.non_null()) {
                    self.err(
                        "FEEL-C107",
                        format!(
                            "range endpoints {} and {} are not comparable",
                            a.kind(),
                            b.kind()
                        ),
                        e.span,
                    );
                }
                Ty::Range(Box::new(a.non_null().join(&b.non_null())))
            }
            ExprKind::UnaryRange(_, x) => {
                let t = self.expr(x);
                Ty::Range(Box::new(t.non_null()))
            }
        }
    }

    fn type_expr(&mut self, t: &TypeExpr) -> Ty {
        let s = t.to_string();
        match parse_schema_type(&s, self.env.aliases) {
            Ok(t) => t,
            Err(m) => {
                let span = match t {
                    TypeExpr::Named(_, sp) => *sp,
                    _ => Span::default(),
                };
                self.err("FEEL-C111", m, span);
                Ty::Any
            }
        }
    }

    fn select(&mut self, bt: &Ty, key: &str, span: Span) -> Ty {
        let base = bt.non_null();
        let r = match &base {
            Ty::Any | Ty::Null => Ty::Any,
            Ty::Context(fs, open) => match fs.iter().find(|f| f.name == key) {
                Some(f) => {
                    if f.optional {
                        f.ty.clone().nullable()
                    } else {
                        f.ty.clone()
                    }
                }
                None => {
                    if !*open {
                        let s = suggest(key, fs.iter().map(|f| f.name.as_str()));
                        self.err_s(
                            "FEEL-C102",
                            format!("context of type {bt} has no field '{key}'; selecting it yields null"),
                            span,
                            s.map(|s| format!("did you mean '{s}'?")),
                        );
                        return Ty::Null;
                    }
                    Ty::Any
                }
            },
            Ty::List(el) => Ty::List(Box::new(self.select(el, key, span))),
            Ty::Date => prop(key, &["year", "month", "day", "weekday"], Ty::Number),
            Ty::DateTime => match key {
                "time offset" => Ty::Dtd.nullable(),
                "timezone" => Ty::String.nullable(),
                "time" => Ty::Time,
                "date" => Ty::Date,
                _ => prop(
                    key,
                    &[
                        "year", "month", "day", "weekday", "hour", "minute", "second",
                    ],
                    Ty::Number,
                ),
            },
            Ty::Time => match key {
                "time offset" => Ty::Dtd.nullable(),
                "timezone" => Ty::String.nullable(),
                _ => prop(key, &["hour", "minute", "second"], Ty::Number),
            },
            Ty::Ymd => prop(key, &["years", "months"], Ty::Number),
            Ty::Dtd => prop(key, &["days", "hours", "minutes", "seconds"], Ty::Number),
            Ty::Range(t) => match key {
                "start" | "end" => (**t).clone().nullable(),
                "start included" | "end included" => Ty::Boolean,
                _ => Ty::Null,
            },
            _ => Ty::Null,
        };
        if r == Ty::Null && !matches!(base, Ty::Context(..)) {
            self.err(
                "FEEL-C102",
                format!(
                    "{} has no property '{key}'; selecting it yields null",
                    bt.kind()
                ),
                span,
            );
        }
        if bt.is_nullable() {
            r.nullable()
        } else {
            r
        }
    }

    fn call(&mut self, f: &Expr, args: &Args, span: Span) -> Ty {
        let arg_types: Vec<(Option<String>, Ty, Span)> = match args {
            Args::Positional(a) => a.iter().map(|x| (None, self.expr(x), x.span)).collect(),
            Args::Named(a) => a
                .iter()
                .map(|(n, _, x)| (Some(n.text.clone()), self.expr(x), x.span))
                .collect(),
        };
        if let ExprKind::Name(n) = &f.kind {
            if self.lookup(&n.tokens).is_none() {
                let joined = n.tokens.join(" ");
                if let Some(id) = builtins::lookup(&joined) {
                    let b = &builtins::BUILTINS[id as usize];
                    if b.extension == 0 || self.env.extensions & b.extension != 0 {
                        self.types.insert(f.id, Ty::Any);
                        return self.builtin_call(id, &arg_types, span);
                    }
                }
            }
        }
        let ft = self.expr(f);
        match ft.non_null() {
            Ty::Function(ps, r) => {
                if !ps.is_empty()
                    && matches!(args, Args::Positional(_))
                    && ps.len() != arg_types.len()
                {
                    self.err(
                        "FEEL-C104",
                        format!(
                            "function expects {} arguments but {} were given",
                            ps.len(),
                            arg_types.len()
                        ),
                        span,
                    );
                }
                *r
            }
            Ty::Any | Ty::Null => Ty::Any,
            other => {
                self.err(
                    "FEEL-C110",
                    format!(
                        "a {} is not a function; invoking it yields null",
                        other.kind()
                    ),
                    f.span,
                );
                Ty::Null
            }
        }
    }

    fn builtin_call(&mut self, id: u16, args: &[(Option<String>, Ty, Span)], span: Span) -> Ty {
        let name = builtins::name_of(id);
        let ovs = builtins::overloads(id);
        let named = args.first().map(|a| a.0.is_some()).unwrap_or(false);
        let n = args.len();
        let mut chosen = None;
        for o in &ovs {
            let ok_arity = if named {
                args.iter()
                    .all(|a| o.params.iter().any(|p| Some(&p.name) == a.0.as_ref()))
            } else {
                n >= o.min_args() && o.max_args().map(|m| n <= m).unwrap_or(true)
            };
            if !ok_arity {
                continue;
            }
            // type compatibility
            let mut ok = true;
            for (i, (an, at, _)) in args.iter().enumerate() {
                let p = if named {
                    o.params.iter().find(|p| Some(&p.name) == an.as_ref())
                } else {
                    o.params
                        .get(i)
                        .or_else(|| o.params.last().filter(|p| p.variadic))
                };
                if let Some(p) = p {
                    let pt = spec_ty(&p.ty);
                    let compatible = at.compatible(&pt)
                        || matches!(at.non_null(), Ty::List(ref el) if el.compatible(&pt))
                        || matches!(pt, Ty::List(ref el) if at.compatible(el));
                    if !compatible {
                        ok = false;
                    }
                }
            }
            if ok {
                chosen = Some(o.clone());
                break;
            }
            if chosen.is_none() {
                chosen = Some(o.clone());
            }
        }
        let Some(o) = chosen else {
            if named {
                let given: Vec<String> = args.iter().filter_map(|a| a.0.clone()).collect();
                let all: Vec<String> = ovs
                    .iter()
                    .flat_map(|o| o.params.iter().map(|p| p.name.clone()))
                    .collect();
                let bad = given
                    .iter()
                    .find(|g| !all.contains(g))
                    .cloned()
                    .unwrap_or_default();
                let s = suggest(&bad, all.iter().map(|s| s.as_str()));
                self.err_s(
                    "FEEL-C104",
                    format!("'{name}' has no parameter named '{bad}'; the call yields null"),
                    span,
                    s.map(|s| format!("did you mean '{s}'?")),
                );
            } else {
                let arities: Vec<String> = ovs
                    .iter()
                    .map(|o| match o.max_args() {
                        Some(m) if m == o.min_args() => m.to_string(),
                        Some(m) => format!("{}..{}", o.min_args(), m),
                        None => format!("{}+", o.min_args()),
                    })
                    .collect();
                self.err(
                    "FEEL-C104",
                    format!(
                        "'{name}' expects {} arguments but {n} were given; the call yields null",
                        arities.join(" or ")
                    ),
                    span,
                );
            }
            return Ty::Null;
        };
        // report definite argument mismatches against the chosen overload
        for (i, (an, at, aspan)) in args.iter().enumerate() {
            let p = if named {
                o.params.iter().find(|p| Some(&p.name) == an.as_ref())
            } else {
                o.params
                    .get(i)
                    .or_else(|| o.params.last().filter(|p| p.variadic))
            };
            if let Some(p) = p {
                let pt = spec_ty(&p.ty);
                let compatible = at.compatible(&pt)
                    || matches!(at.non_null(), Ty::List(ref el) if el.compatible(&pt))
                    || matches!(pt, Ty::List(ref el) if at.compatible(el));
                if !compatible {
                    self.err(
                        "FEEL-C105",
                        format!(
                            "argument '{}' of '{name}' expects {} but got {}; the call yields null",
                            p.name, pt, at
                        ),
                        *aspan,
                    );
                }
            }
        }
        // result type
        let first = args.first().map(|a| a.1.non_null()).unwrap_or(Ty::Any);
        match name {
            "min" | "max" => match (&first, n) {
                (Ty::List(el), 1) => (**el).clone().nullable(),
                (t, _) if n > 1 => t.clone(),
                _ => Ty::Any,
            },
            "abs" => first,
            "sublist" | "append" | "concatenate" | "insert before" | "remove" | "reverse"
            | "union" | "distinct values" | "sort" | "list replace" => {
                if let Ty::List(_) = first {
                    first
                } else {
                    Ty::List(Box::new(Ty::Any))
                }
            }
            "get value" => Ty::Any,
            "duration" => Ty::Any,
            _ => {
                let r = spec_ty(&o.ret);
                // conversion builtins and most others return null on bad input
                match r {
                    Ty::Any => Ty::Any,
                    r => r.nullable(),
                }
            }
        }
    }

    fn unary_tests(&mut self, cand: &Ty, t: &UnaryTests, _span: Span) {
        match t {
            UnaryTests::Any => {}
            UnaryTests::Positive(ts) | UnaryTests::Negated(ts) => {
                for x in ts {
                    self.unary_test(cand, x);
                }
            }
        }
    }

    fn unary_test(&mut self, cand: &Ty, t: &UnaryTest) {
        match t {
            UnaryTest::Compare(op, e, span) => {
                let et = self.expr(e);
                if matches!(e.kind, ExprKind::Null) {
                    return;
                }
                let ok = match op {
                    CmpOp::Eq | CmpOp::Ne => comparable_eq(&cand.non_null(), &et.non_null()),
                    _ => comparable_ord(&cand.non_null(), &et.non_null()),
                };
                if !ok {
                    self.err(
                        "FEEL-C109",
                        format!(
                            "test '{} {}' cannot match a candidate of type {}",
                            op.as_str(),
                            et.kind(),
                            cand
                        ),
                        *span,
                    );
                }
            }
            UnaryTest::Interval(iv, span) => {
                let a = self.expr(&iv.start);
                let b = self.expr(&iv.end);
                for x in [a, b] {
                    if !comparable_ord(&cand.non_null(), &x.non_null()) {
                        self.err(
                            "FEEL-C109",
                            format!(
                                "interval of {} cannot contain a candidate of type {}",
                                x.kind(),
                                cand
                            ),
                            *span,
                        );
                    }
                }
            }
            UnaryTest::Expr(e, uses_q) => {
                let et = self.expr(e);
                if !*uses_q {
                    let x = et.non_null();
                    let ok = match &x {
                        Ty::List(el) => {
                            comparable_eq(&cand.non_null(), &el.non_null())
                                || matches!(el.non_null(), Ty::Range(_))
                        }
                        Ty::Range(el) => comparable_ord(&cand.non_null(), &el.non_null()),
                        Ty::Boolean => true,
                        other => comparable_eq(&cand.non_null(), other),
                    };
                    if !ok {
                        self.err(
                            "FEEL-C109",
                            format!(
                                "test of type {} cannot match a candidate of type {}",
                                et, cand
                            ),
                            e.span,
                        );
                    }
                } else if !et.compatible(&Ty::Boolean)
                    && !matches!(et.non_null(), Ty::Range(_) | Ty::List(_))
                {
                    self.err(
                        "FEEL-C106",
                        format!("unary test using '?' is {}, not boolean", et.kind()),
                        e.span,
                    );
                }
            }
        }
    }
}

fn prop(key: &str, valid: &[&str], t: Ty) -> Ty {
    if valid.contains(&key) {
        t
    } else {
        Ty::Null
    }
}

fn temporal(t: &Ty) -> bool {
    matches!(t, Ty::Date | Ty::Time | Ty::DateTime)
}

pub fn comparable_eq(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Any, _) | (_, Ty::Any) | (Ty::Null, _) | (_, Ty::Null) => true,
        (Ty::Date, Ty::DateTime) | (Ty::DateTime, Ty::Date) => true,
        (Ty::List(_), Ty::List(_))
        | (Ty::Context(..), Ty::Context(..))
        | (Ty::Range(_), Ty::Range(_)) => true,
        (Ty::Function(..), Ty::Function(..)) => true,
        (x, y) => x == y,
    }
}

pub fn comparable_ord(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Any, _) | (_, Ty::Any) | (Ty::Null, _) | (_, Ty::Null) => true,
        (Ty::Date, Ty::DateTime) | (Ty::DateTime, Ty::Date) => true,
        (x, y) if x == y => matches!(
            x,
            Ty::Number | Ty::String | Ty::Date | Ty::Time | Ty::DateTime | Ty::Dtd | Ty::Ymd
        ),
        _ => false,
    }
}

/// Result type of an arithmetic operator (Tables 57, 59, 60), `None` if the
/// combination is undefined. `Any` operands give `Any`.
pub fn arith_type(op: BinOp, a: &Ty, b: &Ty) -> Option<Ty> {
    use Ty::*;
    if matches!(a, Any) || matches!(b, Any) {
        return Some(Any);
    }
    if matches!(a, Null) || matches!(b, Null) {
        return Some(Null);
    }
    Some(match op {
        BinOp::Add => match (a, b) {
            (Number, Number) => Number,
            (String, String) => String,
            (Ymd, Ymd) => Ymd,
            (Dtd, Dtd) => Dtd,
            (DateTime, Ymd | Dtd) | (Ymd | Dtd, DateTime) => DateTime,
            (Time, Dtd) | (Dtd, Time) => Time,
            (Date, Ymd | Dtd) | (Ymd | Dtd, Date) => Date,
            _ => return None,
        },
        BinOp::Sub => match (a, b) {
            (Number, Number) => Number,
            (Ymd, Ymd) => Ymd,
            (Dtd, Dtd) => Dtd,
            (x, y)
                if temporal(x)
                    && temporal(y)
                    && (x == y || matches!((x, y), (Date, DateTime) | (DateTime, Date))) =>
            {
                Dtd
            }
            (DateTime, Ymd | Dtd) => DateTime,
            (Time, Dtd) => Time,
            (Date, Ymd | Dtd) => Date,
            _ => return None,
        },
        BinOp::Mul => match (a, b) {
            (Number, Number) => Number,
            (Ymd, Number) | (Number, Ymd) => Ymd,
            (Dtd, Number) | (Number, Dtd) => Dtd,
            _ => return None,
        },
        BinOp::Div => match (a, b) {
            (Number, Number) => Number,
            (Ymd, Number) => Ymd,
            (Dtd, Number) => Dtd,
            (Ymd, Ymd) | (Dtd, Dtd) => Number,
            _ => return None,
        },
        BinOp::Pow => match (a, b) {
            (Number, Number) => Number,
            _ => return None,
        },
    })
}

/// Build a checker environment from a schema.
pub fn env_from_schema<'a>(
    schema: &'a crate::types::Schema,
    mode: CheckMode,
    extensions: u32,
) -> Result<CheckEnv<'a>, Diagnostic> {
    let bad = |m: String| Diagnostic::error("FEEL-B501", m, Span::default());
    let mut inputs = Vec::new();
    for i in &schema.inputs {
        let t = match &i.ty {
            Some(t) => parse_schema_type(t, &schema.types)
                .map_err(|e| bad(format!("input '{}': {e}", i.name)))?,
            None => Ty::Any,
        };
        inputs.push((toks(&i.name), i.name.clone(), t));
    }
    let mut functions = Vec::new();
    for f in &schema.functions {
        let mut ps = Vec::new();
        let mut names = Vec::new();
        for p in &f.params {
            ps.push(match &p.ty {
                Some(t) => parse_schema_type(t, &schema.types)
                    .map_err(|e| bad(format!("function '{}': {e}", f.name)))?,
                None => Ty::Any,
            });
            names.push(p.name.clone());
        }
        let r = match &f.returns {
            Some(t) => parse_schema_type(t, &schema.types)
                .map_err(|e| bad(format!("function '{}': {e}", f.name)))?,
            None => Ty::Any,
        };
        functions.push((
            toks(&f.name),
            f.name.clone(),
            Ty::Function(ps, Box::new(r)),
            names,
        ));
    }
    let candidate = match &schema.candidate {
        Some(t) => {
            parse_schema_type(t, &schema.types).map_err(|e| bad(format!("candidate: {e}")))?
        }
        None => Ty::Any,
    };
    let expected = match &schema.expected {
        Some(t) => {
            Some(parse_schema_type(t, &schema.types).map_err(|e| bad(format!("expected: {e}")))?)
        }
        None => None,
    };
    let _ = named;
    Ok(CheckEnv {
        inputs,
        functions,
        aliases: &schema.types,
        closed: schema.closed,
        extensions,
        candidate,
        expected,
        mode,
    })
}
