//! AST → IR lowering: name resolution (10.3.2.11 scope and context stack),
//! closure conversion, constant pool and source-site table construction.

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::ir::*;
use crate::Env;
use feel_runtime::builtins;
use feel_runtime::Decimal;
use std::collections::HashMap;

/// Constant pool: FEEL-JSON texts; constant `i` gets runtime handle `3 + i`.
#[derive(Default, Clone, Debug)]
pub struct ConstPool {
    pub items: Vec<String>,
    index: HashMap<String, u32>,
}

impl ConstPool {
    pub fn add_json(&mut self, json: String) -> ConstId {
        if let Some(i) = self.index.get(&json) {
            return *i;
        }
        let i = self.items.len() as u32;
        self.index.insert(json.clone(), i);
        self.items.push(json);
        i
    }
    /// Reserve a slot that is filled later (never shared via deduplication).
    pub fn reserve(&mut self) -> ConstId {
        self.items.push("null".to_string());
        (self.items.len() - 1) as u32
    }
    pub fn str(&mut self, s: &str) -> ConstId {
        let mut j = String::new();
        feel_runtime::json::write_json_string(&mut j, s);
        self.add_json(j)
    }
    pub fn number(&mut self, d: &Decimal) -> ConstId {
        if d.scale() < 0 {
            let mut j = String::from("{\"@number\":");
            feel_runtime::json::write_json_string(&mut j, &d.to_scientific_string());
            j.push('}');
            self.add_json(j)
        } else {
            self.add_json(d.to_string())
        }
    }
    pub fn tagged(&mut self, tag: &str, s: &str) -> ConstId {
        let mut j = format!("{{\"{tag}\":");
        feel_runtime::json::write_json_string(&mut j, s);
        j.push('}');
        self.add_json(j)
    }
    pub fn to_json(&self) -> String {
        format!("[{}]", self.items.join(","))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SiteInfo {
    pub start: usize,
    pub end: usize,
    pub what: String,
}

#[derive(Clone, Debug)]
enum Binding {
    Local(Local),
    Partial(Local),
}

#[derive(Clone, Debug)]
enum Frame {
    Lex {
        level: usize,
        names: Vec<(Vec<String>, Binding)>,
    },
    Item {
        level: usize,
        item: Local,
    },
}

#[derive(Default, Debug)]
struct FnCtx {
    next_local: u32,
    /// (source level, source local) → local in this function
    capture_map: Vec<((usize, Local), Local)>,
    /// captured locals of this function, in env order, with the local in the
    /// enclosing function that provides the value
    captures: Vec<(Local, Local)>,
}

pub struct Lowerer<'a> {
    env: &'a Env,
    pub consts: ConstPool,
    pub sites: Vec<SiteInfo>,
    pub slots: Vec<String>,
    pub slot_types: Vec<Option<String>>,
    /// slots only referenced as fallbacks of implicit filter names (a missing
    /// input is not reported for them)
    pub slot_quiet: Vec<bool>,
    in_item_fallback: u32,
    declared_inputs: usize,
    pub funcs: Vec<Option<FuncIr>>,
    fstack: Vec<FnCtx>,
    frames: Vec<Frame>,
    pub diags: Vec<Diagnostic>,
}

pub fn tokens_of(s: &str) -> Vec<String> {
    crate::parser::name_tokens(s)
}

impl<'a> Lowerer<'a> {
    pub fn new(env: &'a Env) -> Self {
        let mut l = Lowerer {
            env,
            consts: ConstPool::default(),
            sites: vec![SiteInfo {
                start: 0,
                end: 0,
                what: "module".into(),
            }],
            slots: Vec::new(),
            slot_types: Vec::new(),
            slot_quiet: Vec::new(),
            in_item_fallback: 0,
            declared_inputs: 0,
            funcs: Vec::new(),
            fstack: vec![FnCtx::default()],
            frames: Vec::new(),
            diags: Vec::new(),
        };
        // constants 0 and 1 are reserved for the slot-name and slot-type lists
        l.consts.reserve();
        l.consts.reserve();
        l.consts.reserve();
        // pre-declare all schema inputs so slot order is stable
        for i in &env.inputs {
            l.slot_index(&i.name, i.ty.clone());
        }
        // declared inputs are reported when missing only if referenced
        for q in l.slot_quiet.iter_mut() {
            *q = true;
        }
        l.declared_inputs = l.slots.len();
        l
    }

    fn level(&self) -> usize {
        self.fstack.len() - 1
    }

    fn new_local(&mut self) -> Local {
        let f = self.fstack.last_mut().unwrap();
        let l = f.next_local;
        f.next_local += 1;
        l
    }

    pub fn site(&mut self, span: Span, what: &str) -> Site {
        self.sites.push(SiteInfo {
            start: span.start,
            end: span.end,
            what: what.to_string(),
        });
        (self.sites.len() - 1) as Site
    }

    fn slot_index(&mut self, name: &str, ty: Option<String>) -> u32 {
        let quiet = self.in_item_fallback > 0;
        if let Some(i) = self.slots.iter().position(|s| s == name) {
            if !quiet {
                self.slot_quiet[i] = false;
            }
            return i as u32;
        }
        self.slots.push(name.to_string());
        self.slot_types.push(ty);
        self.slot_quiet.push(quiet);
        (self.slots.len() - 1) as u32
    }

    /// Finish: write slot names/types into constants 0 and 1.
    pub fn finish_consts(&mut self) {
        let mut names = String::from("[");
        let mut types = String::from("[");
        let mut quiet = String::from("[");
        for (i, q) in self.slot_quiet.iter().enumerate() {
            if i > 0 {
                quiet.push(',');
            }
            quiet.push_str(if *q { "true" } else { "false" });
        }
        quiet.push(']');
        for (i, n) in self.slots.iter().enumerate() {
            if i > 0 {
                names.push(',');
                types.push(',');
            }
            feel_runtime::json::write_json_string(&mut names, n);
            match &self.slot_types[i] {
                Some(t) => feel_runtime::json::write_json_string(&mut types, t),
                None => types.push_str("null"),
            }
        }
        names.push(']');
        types.push(']');
        self.consts.items[0] = names;
        self.consts.items[1] = types;
        self.consts.items[2] = quiet;
    }

    fn push_lex(&mut self) {
        let level = self.level();
        self.frames.push(Frame::Lex {
            level,
            names: Vec::new(),
        });
    }
    fn pop_frame(&mut self) {
        self.frames.pop();
    }
    fn declare(&mut self, toks: Vec<String>, b: Binding) {
        if let Some(Frame::Lex { names, .. }) = self.frames.last_mut() {
            names.push((toks, b));
        }
    }

    /// Make `src_local` (defined at function level `src_level`) available at
    /// function level `level`, adding captures along the way.
    fn local_at_level(&mut self, level: usize, src_level: usize, src_local: Local) -> Local {
        if level == src_level {
            return src_local;
        }
        let key = (src_level, src_local);
        if let Some((_, l)) = self.fstack[level]
            .capture_map
            .iter()
            .find(|(k, _)| *k == key)
        {
            return *l;
        }
        let outer = self.local_at_level(level - 1, src_level, src_local);
        let f = &mut self.fstack[level];
        let l = f.next_local;
        f.next_local += 1;
        f.capture_map.push((key, l));
        f.captures.push((l, outer));
        l
    }

    fn resolve_from(&mut self, toks: &[String], text: &str, upto: usize, span: Span) -> Ir {
        let cur = self.level();
        let mut i = upto;
        while i > 0 {
            i -= 1;
            match self.frames[i].clone() {
                Frame::Lex { level, names } => {
                    if let Some((_, b)) = names.iter().rev().find(|(n, _)| n.as_slice() == toks) {
                        return match b {
                            Binding::Local(l) => Ir::Local(self.local_at_level(cur, level, *l)),
                            Binding::Partial(l) => Ir::Partial(self.local_at_level(cur, level, *l)),
                        };
                    }
                }
                Frame::Item { level, item } => {
                    let item_l = self.local_at_level(cur, level, item);
                    let key = self.consts.str(text);
                    let fallback = if toks.len() == 1 && toks[0] == "item" {
                        Ir::Local(item_l)
                    } else {
                        self.in_item_fallback += 1;
                        let r = self.resolve_from(toks, text, i, span);
                        self.in_item_fallback -= 1;
                        r
                    };
                    return Ir::ItemField {
                        item: item_l,
                        key,
                        fallback: Box::new(fallback),
                    };
                }
            }
        }
        self.resolve_global(toks, text, span)
    }

    fn resolve_global(&mut self, toks: &[String], text: &str, span: Span) -> Ir {
        if let Some(inp) = self.env.inputs.iter().find(|i| tokens_of(&i.name) == toks) {
            let name = inp.name.clone();
            let ty = inp.ty.clone();
            return Ir::Slot(self.slot_index(&name, ty));
        }
        if let Some(h) = self
            .env
            .host_functions
            .iter()
            .find(|h| tokens_of(&h.name) == toks)
        {
            let h = h.clone();
            let name = self.consts.str(&h.name);
            let sig = self.sig_const(
                &h.params
                    .iter()
                    .map(|(n, t)| (n.clone(), t.clone()))
                    .collect::<Vec<_>>(),
            );
            return Ir::HostFn { name, sig };
        }
        let joined = toks.join(" ");
        if let Some(id) = self.builtin_id(&joined) {
            return Ir::BuiltinFn(id);
        }
        if self.env.auto_inputs {
            return Ir::Slot(self.slot_index(text, None));
        }
        if self.in_item_fallback > 0 {
            // an implicit filter name that is not an input: only the current
            // item can supply it
            return Ir::Null;
        }
        self.diags.push(Diagnostic::error(
            "FEEL-C101",
            format!("unknown name '{text}'"),
            span,
        ));
        Ir::Null
    }

    fn builtin_id(&self, joined: &str) -> Option<u16> {
        let id = builtins::lookup(joined)?;
        let b = &builtins::BUILTINS[id as usize];
        if b.extension != 0 && self.env.extensions & b.extension == 0 {
            return None;
        }
        Some(id)
    }

    /// Is `toks` bound lexically (not to a builtin)?
    fn lexically_bound(&self, toks: &[String]) -> bool {
        self.frames.iter().any(|f| match f {
            Frame::Lex { names, .. } => names.iter().any(|(n, _)| n.as_slice() == toks),
            Frame::Item { .. } => false,
        }) || self.env.inputs.iter().any(|i| tokens_of(&i.name) == toks)
            || self
                .env
                .host_functions
                .iter()
                .any(|h| tokens_of(&h.name) == toks)
    }

    fn sig_const(&mut self, params: &[(String, Option<String>)]) -> ConstId {
        let mut j = String::from("[");
        for (i, (n, t)) in params.iter().enumerate() {
            if i > 0 {
                j.push(',');
            }
            j.push('[');
            feel_runtime::json::write_json_string(&mut j, n);
            j.push(',');
            match t {
                Some(t) => feel_runtime::json::write_json_string(&mut j, t),
                None => j.push_str("null"),
            }
            j.push(']');
        }
        j.push(']');
        self.consts.add_json(j)
    }

    // ------------------------------------------------------------------ programs

    pub fn lower_program(&mut self, p: &Program) -> ProgramIr {
        let main = match p {
            Program::Expression(e) => self.lower(e),
            Program::UnaryTests(t, span) => {
                let q = self.slot_index("?", self.env.candidate_type.clone());
                let cand = self.new_local();
                self.push_lex();
                self.declare(vec!["?".into()], Binding::Local(cand));
                let body = self.lower_unary_tests(cand, t, *span);
                self.pop_frame();
                Ir::Let(cand, Box::new(Ir::Slot(q)), Box::new(body))
            }
        };
        let main_locals = self.fstack[0].next_local;
        self.finish_consts();
        ProgramIr {
            main,
            main_locals,
            funcs: self
                .funcs
                .iter()
                .map(|f| f.clone().expect("function lowered"))
                .collect(),
        }
    }

    fn lower_unary_tests(&mut self, cand: Local, t: &UnaryTests, _span: Span) -> Ir {
        match t {
            UnaryTests::Any => Ir::True,
            UnaryTests::Positive(ts) => Ir::In {
                cand,
                tests: self.lower_tests(ts),
                negated: false,
            },
            UnaryTests::Negated(ts) => Ir::In {
                cand,
                tests: self.lower_tests(ts),
                negated: true,
            },
        }
    }

    fn lower_tests(&mut self, ts: &[UnaryTest]) -> Vec<TestIr> {
        ts.iter()
            .map(|t| match t {
                UnaryTest::Compare(op, e, span) => {
                    let site = self.site(*span, op.as_str());
                    TestIr::Cmp(*op, self.lower(e), site)
                }
                UnaryTest::Interval(iv, span) => {
                    let site = self.site(*span, "interval");
                    TestIr::Interval {
                        start: self.lower(&iv.start),
                        end: self.lower(&iv.end),
                        start_incl: iv.start_incl,
                        end_incl: iv.end_incl,
                        site,
                    }
                }
                UnaryTest::Expr(e, q) => {
                    let site = self.site(e.span, "unary test");
                    TestIr::Value {
                        value: self.lower(e),
                        uses_q: *q,
                        site,
                    }
                }
            })
            .collect()
    }

    // ------------------------------------------------------------------ expressions

    pub fn lower(&mut self, e: &Expr) -> Ir {
        match &e.kind {
            ExprKind::Null => Ir::Null,
            ExprKind::Bool(true) => Ir::True,
            ExprKind::Bool(false) => Ir::False,
            ExprKind::Number(t) => match Decimal::parse(t) {
                Some(d) => Ir::Const(self.consts.number(&d)),
                None => {
                    self.diags.push(Diagnostic::error(
                        "FEEL-S030",
                        format!("numeric literal {t} is outside the FEEL number range"),
                        e.span,
                    ));
                    Ir::Null
                }
            },
            ExprKind::String(s) => Ir::Const(self.consts.str(s)),
            ExprKind::At(s) => self.lower_at(s, e.span),
            ExprKind::Name(n) => self.resolve_from(&n.tokens, &n.text, self.frames.len(), e.span),
            ExprKind::Path(base, n) => {
                let b = self.lower(base);
                let key = self.consts.str(&n.text);
                let site = self.site(e.span, "path");
                Ir::Path(Box::new(b), key, site)
            }
            ExprKind::Filter(l, p) => self.lower_filter(l, p, e.span),
            ExprKind::Call(f, args) => self.lower_call(f, args, e.span),
            ExprKind::Neg(x) => {
                let site = self.site(e.span, "-");
                Ir::Neg(Box::new(self.lower(x)), site)
            }
            ExprKind::Binary(op, a, b) => {
                let what = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Pow => "**",
                };
                let site = self.site(e.span, what);
                let (a, b) = (self.lower(a), self.lower(b));
                Ir::Bin(*op, Box::new(a), Box::new(b), site)
            }
            ExprKind::Compare(op, a, b) => {
                let a_null = matches!(a.kind, ExprKind::Null);
                let b_null = matches!(b.kind, ExprKind::Null);
                if matches!(op, CmpOp::Eq | CmpOp::Ne) && (a_null || b_null) {
                    let other = if a_null { b } else { a };
                    return Ir::IsNull(Box::new(self.lower(other)), *op == CmpOp::Ne);
                }
                let site = self.site(e.span, op.as_str());
                let (a, b) = (self.lower(a), self.lower(b));
                Ir::Cmp(*op, Box::new(a), Box::new(b), site)
            }
            ExprKind::And(a, b) => Ir::And(Box::new(self.lower(a)), Box::new(self.lower(b))),
            ExprKind::Or(a, b) => Ir::Or(Box::new(self.lower(a)), Box::new(self.lower(b))),
            ExprKind::Between(x, lo, hi) => {
                let l = self.new_local();
                let xv = self.lower(x);
                let s1 = self.site(e.span, ">=");
                let s2 = self.site(e.span, "<=");
                let lo = self.lower(lo);
                let hi = self.lower(hi);
                Ir::Let(
                    l,
                    Box::new(xv),
                    Box::new(Ir::And(
                        Box::new(Ir::Cmp(CmpOp::Ge, Box::new(Ir::Local(l)), Box::new(lo), s1)),
                        Box::new(Ir::Cmp(CmpOp::Le, Box::new(Ir::Local(l)), Box::new(hi), s2)),
                    )),
                )
            }
            ExprKind::In(x, tests) => {
                let cand = self.new_local();
                let xv = self.lower(x);
                self.push_lex();
                self.declare(vec!["?".into()], Binding::Local(cand));
                let ts = self.lower_tests(tests);
                self.pop_frame();
                Ir::Let(
                    cand,
                    Box::new(xv),
                    Box::new(Ir::In {
                        cand,
                        tests: ts,
                        negated: false,
                    }),
                )
            }
            ExprKind::Tests(x, tests) => {
                let cand = self.new_local();
                let xv = self.lower(x);
                self.push_lex();
                self.declare(vec!["?".into()], Binding::Local(cand));
                let body = self.lower_unary_tests(cand, tests, e.span);
                self.pop_frame();
                Ir::Let(cand, Box::new(xv), Box::new(body))
            }
            ExprKind::InstanceOf(x, ty) => {
                let xv = self.lower(x);
                let t = self.type_const(ty);
                Ir::InstanceOf(Box::new(xv), t)
            }
            ExprKind::Coerce(x, ty) => {
                let xv = self.lower(x);
                let t = self.type_const(ty);
                let site = self.site(e.span, "binding");
                Ir::Coerce(Box::new(xv), t, site)
            }
            ExprKind::If(c, t, f) => Ir::If(
                Box::new(self.lower(c)),
                Box::new(self.lower(t)),
                Box::new(self.lower(f)),
            ),
            ExprKind::For(its, body) => {
                let site = self.site(e.span, "for");
                self.push_lex();
                let mut iters = Vec::new();
                for (n, _, ic) in its {
                    let src = match ic {
                        IterCtx::Expr(x) => IterIr::Expr(self.lower(x)),
                        IterCtx::Range(a, b) => IterIr::Range(self.lower(a), self.lower(b)),
                    };
                    let l = self.new_local();
                    self.declare(n.tokens.clone(), Binding::Local(l));
                    iters.push((l, src));
                }
                let partial = self.new_local();
                self.declare(vec!["partial".into()], Binding::Partial(partial));
                let b = self.lower(body);
                self.pop_frame();
                Ir::For {
                    iters,
                    partial,
                    body: Box::new(b),
                    site,
                }
            }
            ExprKind::Quantified(q, its, body) => {
                let site = self.site(
                    e.span,
                    if *q == Quantifier::Some {
                        "some"
                    } else {
                        "every"
                    },
                );
                self.push_lex();
                let mut iters = Vec::new();
                for (n, _, x) in its {
                    let src = self.lower(x);
                    let l = self.new_local();
                    self.declare(n.tokens.clone(), Binding::Local(l));
                    iters.push((l, src));
                }
                let b = self.lower(body);
                self.pop_frame();
                Ir::Quant {
                    every: *q == Quantifier::Every,
                    iters,
                    body: Box::new(b),
                    site,
                }
            }
            ExprKind::List(items) => Ir::List(items.iter().map(|x| self.lower(x)).collect()),
            ExprKind::Context(entries) => {
                self.push_lex();
                let mut out = Vec::new();
                for en in entries {
                    let key = self.consts.str(&en.key);
                    let l = self.new_local();
                    let toks = tokens_of(&en.key);
                    let v = if let ExprKind::FunctionDef(fd) = &en.value.kind {
                        // allow (recursive) self reference of function entries
                        self.declare(toks, Binding::Local(l));
                        self.lower_function(fd, en.value.span, Some(l))
                    } else {
                        let v = self.lower(&en.value);
                        self.declare(toks, Binding::Local(l));
                        v
                    };
                    out.push((key, l, v));
                }
                self.pop_frame();
                Ir::Context(out)
            }
            ExprKind::FunctionDef(fd) => self.lower_function(fd, e.span, None),
            ExprKind::Interval(iv) => {
                let site = self.site(e.span, "range");
                Ir::Range {
                    start: Some(Box::new(self.lower(&iv.start))),
                    end: Some(Box::new(self.lower(&iv.end))),
                    start_incl: iv.start_incl,
                    end_incl: iv.end_incl,
                    site,
                }
            }
            ExprKind::UnaryRange(op, x) => {
                let site = self.site(e.span, "range");
                let b = Box::new(self.lower(x));
                let v = Some(b.clone());
                match op {
                    CmpOp::Eq | CmpOp::Ne => Ir::PointRange {
                        value: b,
                        ne: *op == CmpOp::Ne,
                        site,
                    },
                    CmpOp::Lt => Ir::Range {
                        start: None,
                        end: v,
                        start_incl: false,
                        end_incl: false,
                        site,
                    },
                    CmpOp::Le => Ir::Range {
                        start: None,
                        end: v,
                        start_incl: false,
                        end_incl: true,
                        site,
                    },
                    CmpOp::Gt => Ir::Range {
                        start: v,
                        end: None,
                        start_incl: false,
                        end_incl: false,
                        site,
                    },
                    _ => Ir::Range {
                        start: v,
                        end: None,
                        start_incl: true,
                        end_incl: false,
                        site,
                    },
                }
            }
        }
    }

    pub fn type_const(&mut self, ty: &TypeExpr) -> ConstId {
        let s = self.env.resolve_type_name(&ty.to_string());
        self.consts.str(&s)
    }

    fn lower_at(&mut self, s: &str, span: Span) -> Ir {
        use feel_runtime::temporal as tm;
        let c = if let Some(d) = tm::Date::parse(s) {
            Some(self.consts.tagged("@date", &d.to_string()))
        } else if let Some(d) = tm::DateTime::parse(s) {
            Some(self.consts.tagged("@dateTime", &d.to_string()))
        } else if let Some(t) = tm::Time::parse(s) {
            Some(self.consts.tagged("@time", &t.to_string()))
        } else {
            match tm::parse_duration(s) {
                Some(tm::Duration::DayTime(d)) => {
                    Some(self.consts.tagged("@duration", &d.to_string()))
                }
                Some(tm::Duration::YearMonth(d)) => {
                    Some(self.consts.tagged("@duration", &d.to_string()))
                }
                None => None,
            }
        };
        match c {
            Some(c) => Ir::Const(c),
            None => {
                self.diags.push(Diagnostic::error(
                    "FEEL-S031",
                    format!(
                        "@\"{s}\" is not a valid date, time, date and time or duration literal"
                    ),
                    span,
                ));
                Ir::Null
            }
        }
    }

    fn lower_filter(&mut self, l: &Expr, p: &Expr, span: Span) -> Ir {
        let lv = self.lower(l);
        if let ExprKind::Number(t) = &p.kind {
            if let Some(d) = Decimal::parse(t) {
                let c = self.consts.number(&d);
                let site = self.site(span, "index");
                let base = match lv {
                    Ir::Partial(pl) => Ir::Local(pl),
                    other => other,
                };
                return Ir::Index(Box::new(base), Box::new(Ir::Const(c)), site);
            }
        }
        let site = self.site(span, "filter");
        let item = self.new_local();
        let level = self.level();
        self.frames.push(Frame::Item { level, item });
        let pv = self.lower(p);
        self.pop_frame();
        Ir::Filter {
            list: Box::new(lv),
            item,
            pred: Box::new(pv),
            site,
        }
    }

    fn lower_call(&mut self, f: &Expr, args: &Args, span: Span) -> Ir {
        let site = self.site(span, "call");
        if let ExprKind::Name(n) = &f.kind {
            if !self.lexically_bound(&n.tokens) {
                if let Some(id) = self.builtin_id(&n.tokens.join(" ")) {
                    if n.tokens.len() == 1 && n.tokens[0] == "not" {
                        if let Args::Positional(a) = args {
                            if a.len() == 1 {
                                return Ir::Not(Box::new(self.lower(&a[0])), site);
                            }
                        }
                    }
                    return match args {
                        Args::Positional(a) => Ir::CallBuiltin {
                            id,
                            args: a.iter().map(|x| self.lower(x)).collect(),
                            site,
                        },
                        Args::Named(a) => {
                            let mut v = Vec::new();
                            for (pn, _, x) in a {
                                let k = self.consts.str(&pn.text);
                                v.push((k, self.lower(x)));
                            }
                            Ir::CallBuiltinNamed { id, args: v, site }
                        }
                    };
                }
            }
        }
        let fv = self.lower(f);
        match args {
            Args::Positional(a) => Ir::CallValue {
                f: Box::new(fv),
                args: a.iter().map(|x| self.lower(x)).collect(),
                site,
            },
            Args::Named(a) => {
                let mut v = Vec::new();
                for (pn, _, x) in a {
                    let k = self.consts.str(&pn.text);
                    v.push((k, self.lower(x)));
                }
                Ir::CallValueNamed {
                    f: Box::new(fv),
                    args: v,
                    site,
                }
            }
        }
    }

    fn lower_function(&mut self, fd: &FunctionDef, _span: Span, self_local: Option<Local>) -> Ir {
        let params: Vec<(String, Option<String>)> = fd
            .params
            .iter()
            .map(|p| {
                (
                    p.name.text.clone(),
                    p.ty.as_ref()
                        .map(|t| self.env.resolve_type_name(&t.to_string())),
                )
            })
            .collect();
        let sig = self.sig_const(&params);
        if fd.external {
            let m = self.lower(&fd.body);
            return Ir::ExternalFn {
                mapping: Box::new(m),
                sig,
            };
        }
        let idx = self.funcs.len() as u32;
        self.funcs.push(None);
        // new function context: locals 0 and 1 are the wasm params (env, args)
        self.fstack.push(FnCtx {
            next_local: 2,
            ..Default::default()
        });
        self.push_lex();
        let mut plocals = Vec::new();
        for p in &fd.params {
            let l = self.new_local();
            self.declare(p.name.tokens.clone(), Binding::Local(l));
            plocals.push(l);
        }
        let body = self.lower(&fd.body);
        self.pop_frame();
        let ctx = self.fstack.pop().unwrap();
        let captures: Vec<Local> = ctx.captures.iter().map(|(l, _)| *l).collect();
        let outer: Vec<Ir> = ctx.captures.iter().map(|(_, o)| Ir::Local(*o)).collect();
        self.funcs[idx as usize] = Some(FuncIr {
            name: format!("fn{idx}"),
            params: plocals,
            captures,
            body,
            nlocals: ctx.next_local,
        });
        Ir::Closure {
            func: idx,
            captures: outer,
            sig,
            self_local,
        }
    }
}
