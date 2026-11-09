//! DMN → FEEL lowering (the DMN integration layer).
//!
//! A DMN model (with its imports) is lowered to one FEEL context expression
//! whose entries are, in dependency order, the imported models, the input
//! data (bound to compile-time *input slots* and converted to their typeRef),
//! the business knowledge models (FEEL functions), the decision services
//! (functions) and the decisions (each converted to its typeRef, 10.3.2.9.4).
//! The context is compiled once to a Wasm module and evaluated per test case.
//!
//! Decision tables are lowered here, *not* in the FEEL core: input
//! expressions are bound once, each input entry becomes a unary-tests check of
//! the corresponding input value (`ExprKind::Tests`), matched rule outputs
//! are collected in rule order and the hit policy (Unique, Any, Priority,
//! First, Rule order, Output order, Collect with + # < > aggregation) is
//! applied with ordinary FEEL builtins (`count`, `distinct values`, `sort`,
//! `sum`, `min`, `max`).

use crate::model::*;
use feel_compiler::ast::*;
use feel_compiler::diag::Span;
use feel_compiler::parser::name_tokens;
use feel_compiler::{CompileOptions, Entry};

pub struct ModelSet {
    pub main: Definitions,
    /// models directly imported by the main model, by import name
    pub imports: Vec<(String, Definitions)>,
    /// every model reachable through imports (looked up by namespace)
    pub all: Vec<Definitions>,
}

impl ModelSet {
    pub fn by_namespace(&self, ns: &str) -> Option<&Definitions> {
        if self.main.namespace == ns {
            return Some(&self.main);
        }
        self.all.iter().find(|d| d.namespace == ns)
    }
}

/// Input slot name for an input data element of a non-main model.
pub fn namespaced_input(ns: &str, name: &str) -> String {
    format!("{ns}::{name}")
}

#[derive(Default)]
pub struct LowerReport {
    /// expressions that failed to parse (element, message)
    pub errors: Vec<(String, String)>,
    pub unsupported: Vec<String>,
}

pub struct Lowered {
    pub program: Expr,
    pub options: CompileOptions,
    pub report: LowerReport,
    pub input_names: Vec<String>,
}

pub fn mk(kind: ExprKind) -> Expr {
    Expr {
        id: 0,
        span: Span::default(),
        kind,
    }
}
pub fn name(s: &str) -> Expr {
    mk(ExprKind::Name(Name {
        text: s.to_string(),
        tokens: name_tokens(s),
    }))
}
pub fn path(e: Expr, k: &str) -> Expr {
    mk(ExprKind::Path(
        Box::new(e),
        Name {
            text: k.to_string(),
            tokens: name_tokens(k),
        },
    ))
}
pub fn call(f: &str, args: Vec<Expr>) -> Expr {
    mk(ExprKind::Call(Box::new(name(f)), Args::Positional(args)))
}
pub fn ctx(entries: Vec<(String, Expr)>) -> Expr {
    mk(ExprKind::Context(
        entries
            .into_iter()
            .map(|(key, value)| ContextEntry {
                key,
                key_span: Span::default(),
                value,
            })
            .collect(),
    ))
}
pub fn num(n: &str) -> Expr {
    mk(ExprKind::Number(n.to_string()))
}
pub fn null() -> Expr {
    mk(ExprKind::Null)
}
pub fn cmp(op: CmpOp, a: Expr, b: Expr) -> Expr {
    mk(ExprKind::Compare(op, Box::new(a), Box::new(b)))
}
pub fn iff(c: Expr, t: Expr, f: Expr) -> Expr {
    mk(ExprKind::If(Box::new(c), Box::new(t), Box::new(f)))
}
pub fn index(l: Expr, i: i64) -> Expr {
    mk(ExprKind::Filter(Box::new(l), Box::new(num(&i.to_string()))))
}
pub fn instance_of(e: Expr, t: &str) -> Expr {
    mk(ExprKind::InstanceOf(Box::new(e), ty(t)))
}
/// `not(e instance of boolean or e = null)`
fn not_boolean(e: Expr) -> Expr {
    call(
        "not",
        vec![mk(ExprKind::Or(
            Box::new(instance_of(e.clone(), "boolean")),
            Box::new(cmp(CmpOp::Eq, e, null())),
        ))],
    )
}
fn ty(t: &str) -> TypeExpr {
    TypeExpr::Named(t.to_string(), Span::default())
}

pub struct Lowerer<'a> {
    set: &'a ModelSet,
    pub report: LowerReport,
    base_names: Vec<String>,
    fields: Vec<String>,
    aliases: Vec<(String, String)>,
    /// item definition → (allowed values, applies to collection elements)
    allowed: Vec<(String, String, bool)>,
    tmp: u32,
}

fn builtin_type(t: &str) -> Option<&'static str> {
    let t = t.strip_prefix("feel:").unwrap_or(t);
    Some(match t {
        "number" | "decimal" | "integer" | "double" | "long" | "int" => "number",
        "string" => "string",
        "boolean" => "boolean",
        "date" => "date",
        "time" => "time",
        "date and time" | "dateTime" => "date and time",
        "days and time duration" | "dayTimeDuration" | "duration" => "days and time duration",
        "years and months duration" | "yearMonthDuration" => "years and months duration",
        "Any" | "any" => "Any",
        "context" => "context",
        "list" => "list",
        "function" => "function",
        _ => return None,
    })
}

impl<'a> Lowerer<'a> {
    pub fn new(set: &'a ModelSet) -> Self {
        let mut l = Lowerer {
            set,
            report: LowerReport::default(),
            base_names: Vec::new(),
            fields: Vec::new(),
            aliases: Vec::new(),
            allowed: Vec::new(),
            tmp: 0,
        };
        l.collect_types();
        l
    }

    fn collect_types(&mut self) {
        let mut models: Vec<(String, &Definitions)> =
            std::iter::once((String::new(), &self.set.main))
                .chain(self.set.imports.iter().map(|(n, d)| (n.clone(), d)))
                .collect();
        for d in &self.set.all {
            if !models.iter().any(|(_, m)| m.namespace == d.namespace) {
                models.push(("\u{0}".to_string(), d));
            }
        }
        for (prefix, m) in &models {
            for idf in &m.item_defs {
                let name = if prefix.is_empty() {
                    idf.name.clone()
                } else {
                    format!("{prefix}.{}", idf.name)
                };
                let t = self.item_type(idf, prefix, m);
                self.aliases.push((name.clone(), t));
                if let Some(av) = &idf.allowed_values {
                    self.allowed.push((name, av.clone(), idf.is_collection));
                }
                collect_component_names(idf, &mut self.fields);
            }
            for d in &m.decisions {
                self.fields.push(d.name.clone());
            }
            for b in &m.bkms {
                self.fields.push(b.name.clone());
            }
            for s in &m.services {
                self.fields.push(s.name.clone());
            }
            for i in &m.inputs {
                self.fields.push(i.name.clone());
            }
            if prefix == "\u{0}" {
                continue;
            }
            if prefix.is_empty() {
                for x in m
                    .decisions
                    .iter()
                    .map(|d| &d.name)
                    .chain(m.bkms.iter().map(|b| &b.name))
                    .chain(m.services.iter().map(|s| &s.name))
                    .chain(m.inputs.iter().map(|i| &i.name))
                {
                    self.base_names.push(x.clone());
                }
            } else {
                self.base_names.push(prefix.clone());
            }
        }
    }

    /// Structural FEEL type string for an item definition.
    fn item_type(&self, idf: &ItemDef, prefix: &str, m: &Definitions) -> String {
        let base = if let Some(fi) = &idf.function_item {
            let ps: Vec<String> = fi
                .params
                .iter()
                .map(|(_, t)| self.type_string(t.as_deref(), prefix, m))
                .collect();
            format!(
                "function<{}>->{}",
                ps.join(", "),
                self.type_string(fi.output_type_ref.as_deref(), prefix, m)
            )
        } else if !idf.components.is_empty() {
            let parts: Vec<String> = idf
                .components
                .iter()
                .map(|c| format!("\"{}\": {}", c.name, self.item_type(c, prefix, m)))
                .collect();
            format!("context<{}>", parts.join(", "))
        } else {
            self.type_string(idf.type_ref.as_deref(), prefix, m)
        };
        if idf.is_collection {
            format!("list<{base}>")
        } else {
            base
        }
    }

    fn type_string(&self, t: Option<&str>, prefix: &str, m: &Definitions) -> String {
        let Some(t) = t else { return "Any".into() };
        if let Some(b) = builtin_type(t) {
            return b.to_string();
        }
        // item definition in the same model, possibly prefixed
        if let Some(idf) = m.item_defs.iter().find(|i| i.name == t) {
            return self.item_type(idf, prefix, m);
        }
        if let Some((idf, im)) = self.find_item_def(t, m) {
            return self.item_type(idf, prefix, im);
        }
        "Any".into()
    }

    /// Resolve an item definition name relative to model `m` (possibly
    /// qualified by an import name).
    fn find_item_def<'b>(
        &'b self,
        t: &str,
        m: &'b Definitions,
    ) -> Option<(&'b ItemDef, &'b Definitions)> {
        if let Some(idf) = m.item_defs.iter().find(|i| i.name == t) {
            return Some((idf, m));
        }
        if let Some((p, rest)) = t.split_once('.') {
            for imp in m.imports.iter().filter(|i| i.name == p) {
                if let Some(im) = self.set.by_namespace(&imp.namespace) {
                    if let Some(idf) = im.item_defs.iter().find(|i| i.name == rest) {
                        return Some((idf, im));
                    }
                }
            }
        }
        // unnamed imports make their item definitions visible unqualified
        for imp in m.imports.iter().filter(|i| i.name.is_empty()) {
            if let Some(im) = self.set.by_namespace(&imp.namespace) {
                if let Some(idf) = im.item_defs.iter().find(|i| i.name == t) {
                    return Some((idf, im));
                }
            }
        }
        None
    }

    fn resolve_typeref(&self, t: Option<&str>, prefix: &str, m: &Definitions) -> Option<String> {
        let t = t?;
        let s = self.type_string(Some(t), prefix, m);
        if s == "Any" {
            None
        } else {
            Some(s)
        }
    }

    fn allowed_for(&self, t: Option<&str>, m: &Definitions) -> Option<(String, bool)> {
        let (idf, _) = self.find_item_def(t?, m)?;
        idf.allowed_values.clone().map(|a| (a, idf.is_collection))
    }

    /// Return type of a function-typed typeRef (BKM / decision service
    /// variables), used to convert the invocation result.
    fn fn_return_type(&self, t: Option<&str>, prefix: &str, m: &Definitions) -> Option<String> {
        let s = self.resolve_typeref(t, prefix, m)?;
        match feel_runtime::types::parse_type(&s) {
            feel_runtime::types::RtType::Function(_, r) => match *r {
                feel_runtime::types::RtType::Any => None,
                r => Some(r.to_string()),
            },
            _ => None,
        }
    }

    fn fresh(&mut self, base: &str) -> String {
        self.tmp += 1;
        format!("__{base}{}", self.tmp)
    }

    pub fn options(&self, scope: &[String], entry: Entry) -> CompileOptions {
        let mut o = CompileOptions::default();
        o.entry = entry;
        o.check = feel_compiler::check::CheckMode::Off;
        o.known_names = self
            .base_names
            .iter()
            .chain(scope.iter())
            .cloned()
            .collect();
        o.known_fields = self.fields.clone();
        o.schema.types = self.aliases.clone();
        o
    }

    fn parse_expr(&mut self, text: &str, scope: &[String], what: &str) -> Expr {
        let o = self.options(scope, Entry::Expression);
        let p = feel_compiler::parse(text, &o);
        match p.program {
            Some(Program::Expression(e)) => {
                if let Some(bad) = invalid_at_literal(&e) {
                    self.report.errors.push((
                        what.to_string(),
                        format!("@\"{bad}\" is not a valid temporal literal"),
                    ));
                    return null();
                }
                e
            }
            _ => {
                let msg = p
                    .diagnostics
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ");
                self.report
                    .errors
                    .push((what.to_string(), format!("{msg} in `{}`", text.trim())));
                null()
            }
        }
    }

    fn parse_tests(&mut self, text: &str, scope: &[String], what: &str) -> UnaryTests {
        let o = self.options(scope, Entry::UnaryTests);
        let p = feel_compiler::parse(text, &o);
        match p.program {
            Some(Program::UnaryTests(t, _)) => t,
            _ => {
                let msg = p
                    .diagnostics
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ");
                self.report.errors.push((
                    what.to_string(),
                    format!("{msg} in unary tests `{}`", text.trim()),
                ));
                UnaryTests::Positive(vec![UnaryTest::Expr(mk(ExprKind::Bool(false)), false)])
            }
        }
    }

    /// Typed binding: conversion to `typeRef` and allowed-values check.
    fn bind(&mut self, e: Expr, type_ref: Option<&str>, prefix: &str, m: &Definitions) -> Expr {
        let mut e = e;
        if let Some(t) = self.resolve_typeref(type_ref, prefix, m) {
            e = mk(ExprKind::Coerce(Box::new(e), ty(&t)));
        }
        if let Some((av, coll)) = self.allowed_for(type_ref, m) {
            let v = self.fresh("v");
            let r = self.fresh("r");
            let tests = self.parse_tests(&av, &[], "allowed values");
            let check = if coll {
                let x = self.fresh("x");
                let t = mk(ExprKind::Tests(Box::new(name(&x)), Box::new(tests)));
                mk(ExprKind::Quantified(
                    Quantifier::Every,
                    vec![(
                        Name {
                            text: x.clone(),
                            tokens: name_tokens(&x),
                        },
                        Span::default(),
                        name(&v),
                    )],
                    Box::new(t),
                ))
            } else {
                mk(ExprKind::Tests(Box::new(name(&v)), Box::new(tests)))
            };
            let body = iff(
                cmp(CmpOp::Eq, name(&v), null()),
                null(),
                iff(check, name(&v), null()),
            );
            e = path(ctx(vec![(v, e), (r.clone(), body)]), &r);
        }
        e
    }

    // ------------------------------------------------------------------ boxed expressions

    pub fn boxed(
        &mut self,
        b: &Boxed,
        scope: &mut Vec<String>,
        what: &str,
        prefix: &str,
        m: &Definitions,
    ) -> Expr {
        match b {
            Boxed::Literal { text, .. } => {
                if text.trim().is_empty() {
                    return null();
                }
                self.parse_expr(text, scope, what)
            }
            Boxed::Context { entries } => {
                let n0 = scope.len();
                let mut out = Vec::new();
                let mut result = None;
                for (n, t, e) in entries {
                    match n {
                        Some(n) => {
                            let is_fn = matches!(e, Boxed::Function(_));
                            if is_fn {
                                scope.push(n.clone());
                            }
                            let v = self.boxed(e, scope, what, prefix, m);
                            let v = self.bind(v, t.as_deref(), prefix, m);
                            if !is_fn {
                                scope.push(n.clone());
                            }
                            out.push((n.clone(), v));
                        }
                        None => result = Some(self.boxed(e, scope, what, prefix, m)),
                    }
                }
                scope.truncate(n0);
                match result {
                    Some(r) => {
                        let k = self.fresh("result");
                        out.push((k.clone(), r));
                        path(ctx(out), &k)
                    }
                    None => ctx(out),
                }
            }
            Boxed::List(items) => mk(ExprKind::List(
                items
                    .iter()
                    .map(|x| self.boxed(x, scope, what, prefix, m))
                    .collect(),
            )),
            Boxed::Relation { columns, rows } => mk(ExprKind::List(
                rows.iter()
                    .map(|r| {
                        ctx(columns
                            .iter()
                            .cloned()
                            .zip(r.iter().map(|c| self.boxed(c, scope, what, prefix, m)))
                            .collect())
                    })
                    .collect(),
            )),
            Boxed::Invocation { callee, bindings } => {
                let f = self.boxed(callee, scope, what, prefix, m);
                let args = bindings
                    .iter()
                    .map(|(p, e)| {
                        let v = match e {
                            Some(e) => self.boxed(e, scope, what, prefix, m),
                            None => null(),
                        };
                        (
                            Name {
                                text: p.clone(),
                                tokens: name_tokens(p),
                            },
                            Span::default(),
                            v,
                        )
                    })
                    .collect();
                mk(ExprKind::Call(Box::new(f), Args::Named(args)))
            }
            Boxed::Function(fd) => self.function(fd, scope, what, prefix, m),
            // Boxed conditional / filter / iterators (10.2.1.8-10): the "if",
            // "match" and "satisfies" parts MUST resolve to booleans and the
            // "in" parts to collections; otherwise the result is null.
            Boxed::Conditional { cond, then, els } => {
                let c = self.boxed(cond, scope, what, prefix, m);
                let t = self.boxed(then, scope, what, prefix, m);
                let e = self.boxed(els, scope, what, prefix, m);
                let cv = self.fresh("cond");
                let r = self.fresh("r");
                let body = iff(
                    instance_of(name(&cv), "boolean"),
                    iff(name(&cv), t, e.clone()),
                    iff(cmp(CmpOp::Eq, name(&cv), null()), e, null()),
                );
                path(ctx(vec![(cv, c), (r.clone(), body)]), &r)
            }
            Boxed::Filter { input, matcher } => {
                let l = self.boxed(input, scope, what, prefix, m);
                scope.push("item".into());
                let p = self.boxed(matcher, scope, what, prefix, m);
                scope.pop();
                let lv = self.fresh("list");
                let r = self.fresh("r");
                let bad = mk(ExprKind::Filter(
                    Box::new(name(&lv)),
                    Box::new(not_boolean(p.clone())),
                ));
                let body = iff(
                    instance_of(name(&lv), "list"),
                    iff(
                        cmp(CmpOp::Gt, call("count", vec![bad]), num("0")),
                        null(),
                        mk(ExprKind::Filter(Box::new(name(&lv)), Box::new(p))),
                    ),
                    null(),
                );
                path(ctx(vec![(lv, l), (r.clone(), body)]), &r)
            }
            Boxed::For { var, input, ret } => {
                let l = self.boxed(input, scope, what, prefix, m);
                scope.push(var.clone());
                scope.push("partial".into());
                let rv = self.boxed(ret, scope, what, prefix, m);
                scope.pop();
                scope.pop();
                let lv = self.fresh("list");
                let r = self.fresh("r");
                let f = mk(ExprKind::For(
                    vec![(
                        Name {
                            text: var.clone(),
                            tokens: name_tokens(var),
                        },
                        Span::default(),
                        IterCtx::Expr(name(&lv)),
                    )],
                    Box::new(rv),
                ));
                let body = iff(instance_of(name(&lv), "list"), f, null());
                path(ctx(vec![(lv, l), (r.clone(), body)]), &r)
            }
            Boxed::Quantified {
                every,
                var,
                input,
                satisfies,
            } => {
                let l = self.boxed(input, scope, what, prefix, m);
                scope.push(var.clone());
                let sv = self.boxed(satisfies, scope, what, prefix, m);
                scope.pop();
                let lv = self.fresh("list");
                let r = self.fresh("r");
                let q = |kind: Quantifier, body: Expr| {
                    mk(ExprKind::Quantified(
                        kind,
                        vec![(
                            Name {
                                text: var.clone(),
                                tokens: name_tokens(var),
                            },
                            Span::default(),
                            name(&lv),
                        )],
                        Box::new(body),
                    ))
                };
                let check = q(Quantifier::Some, not_boolean(sv.clone()));
                let main = q(
                    if *every {
                        Quantifier::Every
                    } else {
                        Quantifier::Some
                    },
                    sv,
                );
                let body = iff(
                    instance_of(name(&lv), "list"),
                    iff(check, null(), main),
                    null(),
                );
                path(ctx(vec![(lv, l), (r.clone(), body)]), &r)
            }
            Boxed::DecisionTable(dt) => self.decision_table(dt, scope, what),
            Boxed::Unsupported(n) => {
                self.report.unsupported.push(format!("{what}: <{n}>"));
                null()
            }
        }
    }

    fn function(
        &mut self,
        fd: &FunctionDefinition,
        scope: &mut Vec<String>,
        what: &str,
        prefix: &str,
        m: &Definitions,
    ) -> Expr {
        let params: Vec<Param> = fd
            .params
            .iter()
            .map(|(n, t)| Param {
                name: Name {
                    text: n.clone(),
                    tokens: name_tokens(n),
                },
                span: Span::default(),
                ty: self
                    .resolve_typeref(t.as_deref(), prefix, m)
                    .map(|s| ty(&s)),
            })
            .collect();
        let n0 = scope.len();
        scope.extend(fd.params.iter().map(|p| p.0.clone()));
        let kind = fd.kind.to_ascii_uppercase();
        let (body, external) = if kind == "JAVA" || kind == "PMML" {
            // body is a context with class/method signature (or document/model)
            let inner = match fd.body.as_deref() {
                Some(b) => self.boxed(b, scope, what, prefix, m),
                None => ctx(vec![]),
            };
            let key = if kind == "JAVA" { "java" } else { "pmml" };
            // normalise DMN's `class`/`method signature` context into the FEEL mapping form
            (ctx(vec![(key.to_string(), inner)]), true)
        } else {
            let b = match fd.body.as_deref() {
                Some(b) => {
                    let e = self.boxed(b, scope, what, prefix, m);
                    // the body expression's own typeRef converts its result
                    match b {
                        Boxed::Literal {
                            type_ref: Some(t), ..
                        } => self.bind(e, Some(t), prefix, m),
                        _ => e,
                    }
                }
                None => null(),
            };
            (b, false)
        };
        scope.truncate(n0);
        mk(ExprKind::FunctionDef(FunctionDef {
            params,
            body: Box::new(body),
            external,
        }))
    }

    fn decision_table(&mut self, dt: &DecisionTable, scope: &[String], what: &str) -> Expr {
        let mut entries: Vec<(String, Expr)> = Vec::new();
        let mut in_names = Vec::new();
        for (k, inp) in dt.inputs.iter().enumerate() {
            let n = self.fresh(&format!("in{k}_"));
            let e = self.parse_expr(&inp.expr, scope, &format!("{what} input {}", k + 1));
            entries.push((n.clone(), e));
            in_names.push(n);
        }
        let single = dt.outputs.len() <= 1;
        let out_names: Vec<String> = dt
            .outputs
            .iter()
            .enumerate()
            .map(|(i, o)| o.name.clone().unwrap_or_else(|| format!("output{}", i + 1)))
            .collect();
        let mut rule_exprs = Vec::new();
        for (r, rule) in dt.rules.iter().enumerate() {
            let mut cond: Option<Expr> = None;
            for (k, t) in rule.inputs.iter().enumerate() {
                let Some(iname) = in_names.get(k) else {
                    continue;
                };
                let tests = if t.trim() == "-" || t.trim().is_empty() {
                    match dt.inputs.get(k).and_then(|i| i.input_values.as_ref()) {
                        Some(v) if !v.trim().is_empty() => {
                            self.parse_tests(v, scope, &format!("{what} input values {}", k + 1))
                        }
                        _ => continue,
                    }
                } else {
                    self.parse_tests(t, scope, &format!("{what} rule {} input {}", r + 1, k + 1))
                };
                let test = mk(ExprKind::Tests(Box::new(name(iname)), Box::new(tests)));
                cond = Some(match cond {
                    None => test,
                    Some(c) => mk(ExprKind::And(Box::new(c), Box::new(test))),
                });
            }
            // input names must be visible to output entries? (no: outputs use the outer scope)
            let outs: Vec<Expr> = rule
                .outputs
                .iter()
                .enumerate()
                .map(|(j, o)| {
                    self.parse_expr(o, scope, &format!("{what} rule {} output {}", r + 1, j + 1))
                })
                .collect();
            let out = if single {
                outs.into_iter().next().unwrap_or_else(null)
            } else {
                ctx(out_names.iter().cloned().zip(outs).collect())
            };
            let item = mk(ExprKind::List(vec![out]));
            rule_exprs.push(match cond {
                None => item,
                Some(c) => iff(c, item, mk(ExprKind::List(vec![]))),
            });
        }
        let m = self.fresh("matches");
        let matches = if rule_exprs.is_empty() {
            mk(ExprKind::List(vec![]))
        } else {
            call("concatenate", rule_exprs)
        };
        entries.push((m.clone(), matches));
        // default output
        let has_default = dt.outputs.iter().any(|o| o.default.is_some());
        let default = if !has_default {
            null()
        } else if single {
            let d = dt.outputs[0].default.clone().unwrap_or_default();
            self.parse_expr(&d, scope, &format!("{what} default output"))
        } else {
            let vals: Vec<(String, Expr)> = dt
                .outputs
                .iter()
                .zip(out_names.iter())
                .map(|(o, n)| {
                    let v = match &o.default {
                        Some(d) => self.parse_expr(d, scope, &format!("{what} default output")),
                        None => null(),
                    };
                    (n.clone(), v)
                })
                .collect();
            ctx(vals)
        };
        let mv = || name(&m);
        let none = cmp(CmpOp::Eq, call("count", vec![mv()]), num("0"));
        let hp = dt.hit_policy.to_ascii_uppercase();
        let agg = dt.aggregation.clone().map(|a| a.to_ascii_uppercase());
        let body = match hp.as_str() {
            "UNIQUE" | "U" => iff(
                none,
                default,
                iff(
                    cmp(CmpOp::Eq, call("count", vec![mv()]), num("1")),
                    index(mv(), 1),
                    null(),
                ),
            ),
            "ANY" | "A" => iff(
                none,
                default,
                iff(
                    cmp(
                        CmpOp::Eq,
                        call("count", vec![call("distinct values", vec![mv()])]),
                        num("1"),
                    ),
                    index(mv(), 1),
                    null(),
                ),
            ),
            "FIRST" | "F" => iff(none, default, index(mv(), 1)),
            "PRIORITY" | "P" => {
                let sorted = self.priority_sort(dt, &out_names, mv(), single, scope, what);
                iff(none, default, index(sorted, 1))
            }
            "OUTPUT ORDER" | "O" => {
                let sorted = self.priority_sort(dt, &out_names, mv(), single, scope, what);
                iff(none, default, sorted)
            }
            "RULE ORDER" | "R" => iff(none, default, mv()),
            "COLLECT" | "C" => match agg.as_deref() {
                Some("SUM") => iff(none, default, call("sum", vec![mv()])),
                Some("COUNT") => call("count", vec![mv()]),
                Some("MIN") => iff(none, default, call("min", vec![mv()])),
                Some("MAX") => iff(none, default, call("max", vec![mv()])),
                _ => iff(none, default, mv()),
            },
            _ => {
                self.report
                    .unsupported
                    .push(format!("{what}: hit policy {hp}"));
                null()
            }
        };
        let r = self.fresh("dt");
        entries.push((r.clone(), body));
        path(ctx(entries), &r)
    }

    /// Sort matched outputs by the order of their values in the output
    /// values lists (Priority / Output order).
    fn priority_sort(
        &mut self,
        dt: &DecisionTable,
        out_names: &[String],
        list: Expr,
        single: bool,
        scope: &[String],
        what: &str,
    ) -> Expr {
        // per output: list of priority values
        let mut keys: Vec<(Option<String>, Expr)> = Vec::new();
        for (i, o) in dt.outputs.iter().enumerate() {
            if let Some(v) = &o.output_values {
                let lst =
                    self.parse_expr(&format!("[{v}]"), scope, &format!("{what} output values"));
                keys.push((
                    if single {
                        None
                    } else {
                        Some(out_names[i].clone())
                    },
                    lst,
                ));
            }
        }
        if keys.is_empty() {
            return list;
        }
        let a = "__pa".to_string();
        let b = "__pb".to_string();
        let prio = |x: &str, k: &Option<String>, l: &Expr| -> Expr {
            let v = match k {
                Some(k) => path(name(x), k),
                None => name(x),
            };
            index(call("index of", vec![l.clone(), v]), 1)
        };
        // lexicographic comparison over outputs with values
        let mut cond: Option<Expr> = None;
        for (k, l) in keys.iter().rev() {
            let lt = cmp(CmpOp::Lt, prio(&a, k, l), prio(&b, k, l));
            cond = Some(match cond {
                None => lt,
                Some(rest) => mk(ExprKind::Or(
                    Box::new(lt),
                    Box::new(mk(ExprKind::And(
                        Box::new(cmp(CmpOp::Eq, prio(&a, k, l), prio(&b, k, l))),
                        Box::new(rest),
                    ))),
                )),
            });
        }
        let f = mk(ExprKind::FunctionDef(FunctionDef {
            params: vec![
                Param {
                    name: Name::simple(&a),
                    span: Span::default(),
                    ty: None,
                },
                Param {
                    name: Name::simple(&b),
                    span: Span::default(),
                    ty: None,
                },
            ],
            body: Box::new(cond.unwrap()),
            external: false,
        }));
        call("sort", vec![list, f])
    }

    // ------------------------------------------------------------------ DRG

    fn find_name(m: &Definitions, id: &str) -> Option<String> {
        m.decisions
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| m.inputs.iter().find(|i| i.id == id).map(|i| i.name.clone()))
            .or_else(|| m.bkms.iter().find(|b| b.id == id).map(|b| b.name.clone()))
            .or_else(|| {
                m.services
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.clone())
            })
    }

    /// Context entries for a model: inputs, BKMs, services, decisions.
    /// `input_source(name)` gives the expression providing an input value.
    fn model_entries(
        &mut self,
        m: &Definitions,
        prefix: &str,
        input_source: &dyn Fn(&str) -> Expr,
        depth: u32,
    ) -> Vec<(String, Expr)> {
        let mut entries = Vec::new();
        let mut scope: Vec<String> = Vec::new();
        // imported models: a context per named import; unnamed imports are
        // merged into this model's scope (DMN 1.5 6.3.3). Input data of an
        // imported model are bound once per namespace (DMN16-50).
        if depth < 8 {
            for imp in m.imports.clone() {
                let Some(im) = self.set.by_namespace(&imp.namespace).cloned() else {
                    continue;
                };
                let ns = im.namespace.clone();
                let src = move |n: &str| name(&namespaced_input(&ns, n));
                let sub = self.model_entries(&im, &imp.name, &src, depth + 1);
                if imp.name.is_empty() {
                    for (k, v) in sub {
                        scope.push(k.clone());
                        entries.push((k, v));
                    }
                } else {
                    scope.push(imp.name.clone());
                    entries.push((imp.name.clone(), ctx(sub)));
                }
            }
        }
        for i in &m.inputs {
            let e = input_source(&i.name);
            let e = self.bind(e, i.type_ref.as_deref(), prefix, m);
            entries.push((i.name.clone(), e));
        }
        // BKMs in knowledge-requirement order
        let order = topo(
            &m.bkms.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
            |id| {
                m.bkms
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| b.required_knowledge.clone())
                    .unwrap_or_default()
            },
        );
        for id in order {
            let b = m.bkms.iter().find(|b| b.id == id).unwrap().clone();
            let what = format!("bkm '{}'", b.name);
            scope.push(b.name.clone());
            let mut e = match &b.logic {
                Some(fd) => self.function(fd, &mut scope, &what, prefix, m),
                None => null(),
            };
            if let Some(rt) = self.fn_return_type(b.type_ref.as_deref(), prefix, m) {
                if let ExprKind::FunctionDef(fd) = &mut e.kind {
                    if !fd.external {
                        let body = std::mem::replace(&mut *fd.body, null());
                        *fd.body = mk(ExprKind::Coerce(Box::new(body), ty(&rt)));
                    }
                }
            }
            entries.push((b.name.clone(), e));
        }
        // decisions in requirement order
        let ids: Vec<String> = m.decisions.iter().map(|d| d.id.clone()).collect();
        let order = topo(&ids, |id| {
            m.decisions
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.required_decisions.clone())
                .unwrap_or_default()
        });
        // decision services (they reference decisions by evaluating them in
        // their own scope; they need BKMs, which are defined above)
        for s in &m.services {
            let e = self.service(s, m, prefix);
            entries.push((s.name.clone(), e));
        }
        for id in order {
            let d = m.decisions.iter().find(|d| d.id == id).unwrap().clone();
            let e = self.decision_logic(&d, &mut scope, prefix, m);
            entries.push((d.name.clone(), e));
            scope.push(d.name.clone());
        }
        entries
    }

    fn decision_logic(
        &mut self,
        d: &Decision,
        scope: &mut Vec<String>,
        prefix: &str,
        m: &Definitions,
    ) -> Expr {
        let what = format!("decision '{}'", d.name);
        let e = match &d.expr {
            Some(b) => self.boxed(b, scope, &what, prefix, m),
            None => null(),
        };
        self.bind(e, d.type_ref.as_deref(), prefix, m)
    }

    fn service(&mut self, s: &DecisionService, m: &Definitions, prefix: &str) -> Expr {
        let mut params = Vec::new();
        for id in s.input_data.iter().chain(s.input_decisions.iter()) {
            if let Some(n) = Self::find_name(m, id) {
                let t = m
                    .inputs
                    .iter()
                    .find(|i| i.name == n)
                    .and_then(|i| i.type_ref.clone())
                    .or_else(|| {
                        m.decisions
                            .iter()
                            .find(|d| d.name == n)
                            .and_then(|d| d.type_ref.clone())
                    });
                params.push((n, t));
            }
        }
        // encapsulated + output decisions in requirement order
        let mut ids: Vec<String> = s.encapsulated_decisions.clone();
        for o in &s.output_decisions {
            if !ids.contains(o) {
                ids.push(o.clone());
            }
        }
        let order = topo(&ids, |id| {
            m.decisions
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.required_decisions.clone())
                .unwrap_or_default()
        });
        let mut scope: Vec<String> = params.iter().map(|p| p.0.clone()).collect();
        let mut entries = Vec::new();
        for id in order {
            if let Some(d) = m.decisions.iter().find(|d| d.id == id).cloned() {
                let e = self.decision_logic(&d, &mut scope, prefix, m);
                entries.push((d.name.clone(), e));
                scope.push(d.name.clone());
            }
        }
        let outs: Vec<String> = s
            .output_decisions
            .iter()
            .filter_map(|id| Self::find_name(m, id))
            .collect();
        let r = self.fresh("ds");
        let result = if outs.len() == 1 {
            name(&outs[0])
        } else {
            ctx(outs.iter().map(|o| (o.clone(), name(o))).collect())
        };
        entries.push((r.clone(), result));
        let mut body = path(ctx(entries), &r);
        if let Some(rt) = self.fn_return_type(s.type_ref.as_deref(), prefix, m) {
            body = mk(ExprKind::Coerce(Box::new(body), ty(&rt)));
        }
        let ps = params
            .iter()
            .map(|(n, t)| Param {
                name: Name {
                    text: n.clone(),
                    tokens: name_tokens(n),
                },
                span: Span::default(),
                ty: self
                    .resolve_typeref(t.as_deref(), prefix, m)
                    .map(|s| ty(&s)),
            })
            .collect();
        mk(ExprKind::FunctionDef(FunctionDef {
            params: ps,
            body: Box::new(body),
            external: false,
        }))
    }

    /// The whole-model program: a context of all DRG elements. Input data
    /// values come from input slots of the same name; inputs of an imported
    /// model come from the fields of an input named after the import.
    pub fn lower_model(&mut self) -> Expr {
        let main = self.set.main.clone();
        let src = |n: &str| name(n);
        // the main model's inputs are input slots of the same name; imported
        // models' inputs are slots named "<namespace>::<name>"
        ctx(self.model_entries(&main, "", &src, 0))
    }

    pub fn input_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .set
            .main
            .inputs
            .iter()
            .map(|i| i.name.clone())
            .collect();
        v.extend(self.set.imports.iter().map(|(n, _)| n.clone()));
        v
    }
}

/// The payload of the first invalid `@"..."` literal (a static error that
/// makes only the containing DRG element fail).
fn invalid_at_literal(e: &Expr) -> Option<String> {
    use feel_runtime::temporal as tm;
    if let ExprKind::At(s) = &e.kind {
        let ok = tm::Date::parse(s).is_some()
            || tm::DateTime::parse(s).is_some()
            || tm::Time::parse(s).is_some()
            || tm::parse_duration(s).is_some();
        if !ok {
            return Some(s.clone());
        }
    }
    e.children().into_iter().find_map(invalid_at_literal)
}

fn collect_component_names(idf: &ItemDef, out: &mut Vec<String>) {
    for c in &idf.components {
        out.push(c.name.clone());
        collect_component_names(c, out);
    }
}

/// Topological order (dependencies first), stable w.r.t. the input order.
pub fn topo(ids: &[String], deps: impl Fn(&str) -> Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut visiting: Vec<String> = Vec::new();
    fn visit(
        id: &str,
        ids: &[String],
        deps: &dyn Fn(&str) -> Vec<String>,
        out: &mut Vec<String>,
        visiting: &mut Vec<String>,
    ) {
        if out.iter().any(|x| x == id)
            || visiting.iter().any(|x| x == id)
            || !ids.iter().any(|x| x == id)
        {
            return;
        }
        visiting.push(id.to_string());
        for d in deps(id) {
            visit(&d, ids, deps, out, visiting);
        }
        visiting.pop();
        out.push(id.to_string());
    }
    for id in ids {
        visit(id, ids, &deps, &mut out, &mut visiting);
    }
    out
}
