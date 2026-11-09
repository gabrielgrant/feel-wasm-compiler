//! IR → WebAssembly lowering.
//!
//! Module layout (see docs/ABI.md):
//! * imports: only the `feel_rt_v1` runtime helpers the expression uses;
//! * defined functions: `evaluate`, `__feel_apply`, `__feel_invoke`,
//!   `__feel_set_limits`, then one function per FEEL function literal;
//! * table 0: funcref table of the FEEL function bodies (closure dispatch);
//! * globals: fuel (i64), depth (i32), max fuel (i64), max depth (i32);
//! * custom sections `feel.meta` (JSON metadata) and `feel.consts`
//!   (FEEL-JSON constant pool). The module has no linear memory.

use crate::ast::{BinOp, CmpOp};
use crate::ir::*;
use feel_runtime::abi::{import_name, ABI_FUNCS, ABI_MODULE};
use std::borrow::Cow;
use std::collections::BTreeMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, ElementSection, Elements, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction as I, Module, RefType, TableSection,
    TableType, TypeSection, ValType,
};

const G_FUEL: u32 = 0;
const G_DEPTH: u32 = 1;
const G_MAX_FUEL: u32 = 2;
const G_MAX_DEPTH: u32 = 3;
const ABSENT: i32 = -1;
const RESERVED: i32 = 3;

/// Defined function slots (relative to the first defined function).
const F_EVALUATE: u32 = 0;
const F_APPLY: u32 = 1;
const F_INVOKE: u32 = 2;
const F_SET_LIMITS: u32 = 3;
const F_USER0: u32 = 4;

pub struct EmitInput<'a> {
    pub prog: &'a ProgramIr,
    pub consts_json: &'a str,
    pub meta_json: &'a str,
}

pub struct Emitted {
    pub bytes: Vec<u8>,
    pub imports: Vec<String>,
}

struct Imports {
    /// ABI function name → import index
    used: BTreeMap<&'static str, u32>,
    order: Vec<&'static str>,
    frozen: bool,
}

impl Imports {
    fn idx(&mut self, name: &'static str) -> u32 {
        if let Some(i) = self.used.get(name) {
            return *i;
        }
        assert!(!self.frozen, "import {name} discovered after freezing");
        let i = self.order.len() as u32;
        self.used.insert(name, i);
        self.order.push(name);
        i
    }
}

struct FnBuilder<'a> {
    code: Vec<I<'static>>,
    next_local: u32,
    nparams: u32,
    labels: Vec<u32>,
    next_label: u32,
    imports: &'a mut Imports,
    n_imports: u32,
    /// main function: slot → cached local
    slot_cache: Option<BTreeMap<u32, u32>>,
}

impl<'a> FnBuilder<'a> {
    fn new(nparams: u32, nlocals: u32, imports: &'a mut Imports, n_imports: u32, main: bool) -> Self {
        FnBuilder {
            code: Vec::new(),
            next_local: nlocals.max(nparams),
            nparams,
            labels: Vec::new(),
            next_label: 0,
            imports,
            n_imports,
            slot_cache: if main { Some(BTreeMap::new()) } else { None },
        }
    }
    fn tmp(&mut self) -> u32 {
        let l = self.next_local;
        self.next_local += 1;
        l
    }
    fn i(&mut self, ins: I<'static>) {
        self.code.push(ins);
    }
    fn c(&mut self, x: i32) {
        self.code.push(I::I32Const(x));
    }
    fn get(&mut self, l: u32) {
        self.code.push(I::LocalGet(l));
    }
    fn set(&mut self, l: u32) {
        self.code.push(I::LocalSet(l));
    }
    fn tee(&mut self, l: u32) {
        self.code.push(I::LocalTee(l));
    }
    fn rt(&mut self, name: &'static str) {
        let idx = self.imports.idx(name);
        self.code.push(I::Call(idx));
    }
    fn call_defined(&mut self, k: u32) {
        self.code.push(I::Call(self.n_imports + k));
    }
    fn open(&mut self, ins: I<'static>) -> u32 {
        let id = self.next_label;
        self.next_label += 1;
        self.labels.push(id);
        self.code.push(ins);
        id
    }
    fn end(&mut self) {
        self.labels.pop();
        self.code.push(I::End);
    }
    fn else_(&mut self) {
        self.code.push(I::Else);
    }
    fn depth_of(&self, label: u32) -> u32 {
        let pos = self.labels.iter().rposition(|l| *l == label).expect("label in scope");
        (self.labels.len() - 1 - pos) as u32
    }
    fn br(&mut self, label: u32) {
        let d = self.depth_of(label);
        self.code.push(I::Br(d));
    }
    fn br_if(&mut self, label: u32) {
        let d = self.depth_of(label);
        self.code.push(I::BrIf(d));
    }
    fn fuel_tick(&mut self) {
        self.i(I::GlobalGet(G_FUEL));
        self.i(I::I64Const(1));
        self.i(I::I64Sub);
        self.i(I::GlobalSet(G_FUEL));
        self.i(I::GlobalGet(G_FUEL));
        self.i(I::I64Const(0));
        self.i(I::I64LtS);
        self.open(I::If(BlockType::Empty));
        self.rt("fuel_exhausted");
        self.i(I::Unreachable);
        self.end();
    }

    // -------------------------------------------------------------- handles

    fn emit_h(&mut self, ir: &Ir) {
        match ir {
            Ir::Null => self.c(0),
            Ir::True => self.c(1),
            Ir::False => self.c(2),
            Ir::Const(c) => self.c(RESERVED + *c as i32),
            Ir::Local(l) => self.get(*l),
            Ir::Slot(i) => {
                if let Some(cache) = &mut self.slot_cache {
                    let l = match cache.get(i) {
                        Some(l) => *l,
                        None => {
                            let l = self.next_local;
                            self.next_local += 1;
                            self.slot_cache.as_mut().unwrap().insert(*i, l);
                            l
                        }
                    };
                    self.get(l);
                } else {
                    self.c(*i as i32);
                    self.rt("slot");
                }
            }
            Ir::ItemField { item, key, fallback } => {
                let t = self.tmp();
                self.get(*item);
                self.c(RESERVED + *key as i32);
                self.rt("item_field");
                self.tee(t);
                self.c(ABSENT);
                self.i(I::I32Ne);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.get(t);
                self.else_();
                self.emit_h(fallback);
                self.end();
            }
            Ir::Path(b, key, site) => {
                self.emit_h(b);
                self.c(RESERVED + *key as i32);
                self.c(*site as i32);
                self.rt("path");
            }
            Ir::Bin(op, a, b, site) => {
                self.emit_h(a);
                self.emit_h(b);
                self.c(*site as i32);
                self.rt(match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    BinOp::Div => "div",
                    BinOp::Pow => "pow",
                });
            }
            Ir::Neg(a, site) => {
                self.emit_h(a);
                self.c(*site as i32);
                self.rt("neg");
            }
            Ir::Cmp(..)
            | Ir::IsNull(..)
            | Ir::And(..)
            | Ir::Or(..)
            | Ir::Not(..)
            | Ir::In { .. }
            | Ir::InstanceOf(..)
            | Ir::Quant { .. } => {
                // tri → handle: 0→2 (false), 1→1 (true), 2→0 (null) = 2 - t
                self.c(2);
                self.emit_t(ir);
                self.i(I::I32Sub);
            }
            Ir::If(c, t, f) => {
                self.emit_t(c);
                self.c(1);
                self.i(I::I32Eq);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.emit_h(t);
                self.else_();
                self.emit_h(f);
                self.end();
            }
            Ir::Let(l, v, body) => {
                self.emit_h(v);
                self.set(*l);
                self.emit_h(body);
            }
            Ir::List(items) => {
                let t = self.tmp();
                self.c(items.len() as i32);
                self.rt("list_new");
                self.set(t);
                for it in items {
                    self.get(t);
                    self.emit_h(it);
                    self.rt("list_push");
                }
                self.get(t);
            }
            Ir::Context(entries) => {
                for (_, l, v) in entries {
                    if let Ir::Closure { self_local: Some(_), .. } = v {
                        self.emit_h(v);
                        self.i(I::Drop);
                    } else {
                        self.emit_h(v);
                        self.set(*l);
                    }
                }
                let t = self.tmp();
                self.c(entries.len() as i32);
                self.rt("ctx_new");
                self.set(t);
                for (k, l, _) in entries {
                    self.get(t);
                    self.c(RESERVED + *k as i32);
                    self.get(*l);
                    self.rt("ctx_put");
                }
                self.get(t);
            }
            Ir::Range { start, end, start_incl, end_incl, site } => {
                let mut flags = 0;
                match start {
                    Some(s) => self.emit_h(s),
                    None => {
                        self.c(0);
                        flags |= 4;
                    }
                }
                match end {
                    Some(e) => self.emit_h(e),
                    None => {
                        self.c(0);
                        flags |= 8;
                    }
                }
                if *start_incl {
                    flags |= 1;
                }
                if *end_incl {
                    flags |= 2;
                }
                self.c(flags);
                self.c(*site as i32);
                self.rt("range");
            }
            Ir::Filter { list, item, pred, site } => self.emit_filter(list, *item, pred, None, *site),
            Ir::FilterPath { list, item, pred, key, site } => self.emit_filter(list, *item, pred, Some(*key), *site),
            Ir::Index(l, n, site) => {
                self.emit_h(l);
                self.emit_h(n);
                self.c(*site as i32);
                self.rt("index");
            }
            Ir::For { iters, partial, body, site } => self.emit_for(iters, *partial, body, *site),
            Ir::CallBuiltin { id, args, site } => {
                self.c(*id as i32);
                self.emit_list(args);
                self.c(*site as i32);
                self.rt("call_builtin");
            }
            Ir::CallBuiltinNamed { id, args, site } => {
                self.c(*id as i32);
                self.emit_named(args);
                self.c(*site as i32);
                self.rt("call_builtin_named");
            }
            Ir::CallValue { f, args, site } => {
                self.emit_h(f);
                self.emit_list(args);
                self.c(0);
                self.c(*site as i32);
                self.call_defined(F_INVOKE);
            }
            Ir::CallValueNamed { f, args, site } => {
                self.emit_h(f);
                self.emit_named(args);
                self.c(1);
                self.c(*site as i32);
                self.call_defined(F_INVOKE);
            }
            Ir::Closure { func, captures, sig, self_local } => {
                let env = self.tmp();
                let fv = self.tmp();
                self.c(captures.len() as i32);
                self.rt("list_new");
                self.set(env);
                self.c(*func as i32);
                self.get(env);
                self.c(RESERVED + *sig as i32);
                self.rt("make_closure");
                self.set(fv);
                if let Some(sl) = self_local {
                    self.get(fv);
                    self.set(*sl);
                }
                for c in captures {
                    self.get(env);
                    self.emit_h(c);
                    self.rt("list_push");
                }
                self.get(fv);
            }
            Ir::BuiltinFn(id) => {
                self.c(*id as i32);
                self.rt("builtin_fn");
            }
            Ir::HostFn { name, sig } => {
                self.c(RESERVED + *name as i32);
                self.c(RESERVED + *sig as i32);
                self.rt("host_fn");
            }
            Ir::ExternalFn { mapping, sig } => {
                self.emit_h(mapping);
                self.c(RESERVED + *sig as i32);
                self.rt("external_fn");
            }
            Ir::Coerce(x, ty, site) => {
                self.emit_h(x);
                self.c(RESERVED + *ty as i32);
                self.c(*site as i32);
                self.rt("coerce");
            }
            Ir::Partial(l) => {
                self.get(*l);
                self.rt("list_snapshot");
            }
        }
    }

    fn emit_list(&mut self, args: &[Ir]) {
        let t = self.tmp();
        self.c(args.len() as i32);
        self.rt("list_new");
        self.set(t);
        for a in args {
            self.get(t);
            self.emit_h(a);
            self.rt("list_push");
        }
        self.get(t);
    }

    fn emit_named(&mut self, args: &[(ConstId, Ir)]) {
        let t = self.tmp();
        self.c(args.len() as i32);
        self.rt("ctx_new");
        self.set(t);
        for (k, a) in args {
            self.get(t);
            self.c(RESERVED + *k as i32);
            self.emit_h(a);
            self.rt("ctx_put");
        }
        self.get(t);
    }

    // -------------------------------------------------------------- tri-state

    fn emit_t(&mut self, ir: &Ir) {
        match ir {
            Ir::True => self.c(1),
            Ir::False => self.c(0),
            Ir::Null => self.c(2),
            Ir::Cmp(op, a, b, site) => {
                self.emit_h(a);
                self.emit_h(b);
                self.c(*site as i32);
                self.rt(match op {
                    CmpOp::Eq => "eq",
                    CmpOp::Ne => "ne",
                    CmpOp::Lt => "lt",
                    CmpOp::Le => "le",
                    CmpOp::Gt => "gt",
                    CmpOp::Ge => "ge",
                });
            }
            Ir::IsNull(x, negate) => {
                self.emit_h(x);
                self.c(0);
                self.i(if *negate { I::I32Ne } else { I::I32Eq });
            }
            Ir::And(a, b) => {
                let ta = self.tmp();
                let tb = self.tmp();
                self.emit_t(a);
                self.tee(ta);
                self.i(I::I32Eqz);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.c(0);
                self.else_();
                self.emit_t(b);
                self.tee(tb);
                self.i(I::I32Eqz);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.c(0);
                self.else_();
                self.c(1);
                self.c(2);
                self.get(ta);
                self.get(tb);
                self.i(I::I32And);
                self.c(1);
                self.i(I::I32Eq);
                self.i(I::Select);
                self.end();
                self.end();
            }
            Ir::Or(a, b) => {
                let ta = self.tmp();
                let tb = self.tmp();
                self.emit_t(a);
                self.tee(ta);
                self.c(1);
                self.i(I::I32Eq);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.c(1);
                self.else_();
                self.emit_t(b);
                self.tee(tb);
                self.c(1);
                self.i(I::I32Eq);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.c(1);
                self.else_();
                self.get(ta);
                self.get(tb);
                self.i(I::I32Or);
                self.end();
                self.end();
            }
            Ir::Not(x, site) => {
                if x.is_tri() || matches!(**x, Ir::True | Ir::False | Ir::Null) {
                    let t = self.tmp();
                    self.emit_t(x);
                    self.set(t);
                    // t==2 ? 2 : 1-t
                    self.c(2);
                    self.c(1);
                    self.get(t);
                    self.i(I::I32Sub);
                    self.get(t);
                    self.c(2);
                    self.i(I::I32Eq);
                    self.i(I::Select);
                } else {
                    self.emit_h(x);
                    self.c(*site as i32);
                    self.rt("not_bool");
                }
            }
            Ir::In { cand, tests, negated } => {
                let acc = self.tmp();
                let tt = self.tmp();
                self.c(0);
                self.set(acc);
                for t in tests {
                    // skip remaining tests once true
                    self.get(acc);
                    self.c(1);
                    self.i(I::I32Ne);
                    self.open(I::If(BlockType::Empty));
                    self.emit_test(*cand, t);
                    self.tee(tt);
                    // new = tt==1 ? 1 : acc|tt
                    self.c(1);
                    self.i(I::I32Eq);
                    self.open(I::If(BlockType::Result(ValType::I32)));
                    self.c(1);
                    self.else_();
                    self.get(acc);
                    self.get(tt);
                    self.i(I::I32Or);
                    self.end();
                    self.set(acc);
                    self.end();
                }
                if *negated {
                    self.c(2);
                    self.c(1);
                    self.get(acc);
                    self.i(I::I32Sub);
                    self.get(acc);
                    self.c(2);
                    self.i(I::I32Eq);
                    self.i(I::Select);
                } else {
                    self.get(acc);
                }
            }
            Ir::InstanceOf(x, ty) => {
                self.emit_h(x);
                self.c(RESERVED + *ty as i32);
                self.rt("instance_of");
            }
            Ir::Quant { every, iters, body, site } => self.emit_quant(*every, iters, body, *site),
            Ir::If(c, t, f) => {
                self.emit_t(c);
                self.c(1);
                self.i(I::I32Eq);
                self.open(I::If(BlockType::Result(ValType::I32)));
                self.emit_t(t);
                self.else_();
                self.emit_t(f);
                self.end();
            }
            Ir::Let(l, v, body) => {
                self.emit_h(v);
                self.set(*l);
                self.emit_t(body);
            }
            other => {
                // handle → tri inline: h==1 → 1, h==2 → 0, else 2
                let t = self.tmp();
                self.emit_h(other);
                self.set(t);
                self.c(1);
                self.c(0);
                self.c(2);
                self.get(t);
                self.c(2);
                self.i(I::I32Eq);
                self.i(I::Select);
                self.get(t);
                self.c(1);
                self.i(I::I32Eq);
                self.i(I::Select);
            }
        }
    }

    fn emit_test(&mut self, cand: Local, t: &TestIr) {
        match t {
            TestIr::Cmp(op, e, site) => {
                if matches!(e, Ir::Null) && matches!(op, CmpOp::Eq | CmpOp::Ne) {
                    self.get(cand);
                    self.c(0);
                    self.i(if *op == CmpOp::Eq { I::I32Eq } else { I::I32Ne });
                    return;
                }
                self.get(cand);
                self.emit_h(e);
                self.c(*site as i32);
                self.rt(match op {
                    CmpOp::Eq => "eq",
                    CmpOp::Ne => "ne",
                    CmpOp::Lt => "lt",
                    CmpOp::Le => "le",
                    CmpOp::Gt => "gt",
                    CmpOp::Ge => "ge",
                });
            }
            TestIr::Interval { start, end, start_incl, end_incl, site } => {
                let a = Ir::Cmp(if *start_incl { CmpOp::Ge } else { CmpOp::Gt }, Box::new(Ir::Local(cand)), Box::new(start.clone()), *site);
                let b = Ir::Cmp(if *end_incl { CmpOp::Le } else { CmpOp::Lt }, Box::new(Ir::Local(cand)), Box::new(end.clone()), *site);
                self.emit_t(&Ir::And(Box::new(a), Box::new(b)));
            }
            TestIr::Value { value, uses_q, site } => {
                match value {
                    // `null` test: candidate is null
                    Ir::Null => {
                        self.get(cand);
                        self.c(0);
                        self.i(I::I32Eq);
                    }
                    // scalar constants (strings, numbers, temporal literals) can only
                    // be compared with `=`
                    Ir::Const(_) if !*uses_q && !value_may_be_collection(value) => {
                        self.get(cand);
                        self.emit_h(value);
                        self.c(*site as i32);
                        self.rt("eq");
                    }
                    _ => {
                        self.get(cand);
                        self.emit_h(value);
                        self.c(*uses_q as i32);
                        self.c(*site as i32);
                        self.rt("in_value");
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------- loops

    fn emit_filter(&mut self, list: &Ir, item: Local, pred: &Ir, key: Option<ConstId>, site: Site) {
        let lst = self.tmp();
        let n = self.tmp();
        let i = self.tmp();
        let out = self.tmp();
        let res = self.tmp();
        let v = self.tmp();
        self.emit_h(list);
        self.rt("filter_list");
        self.tee(lst);
        self.c(ABSENT);
        self.i(I::I32Eq);
        self.open(I::If(BlockType::Empty));
        if let Some(k) = key {
            self.c(0);
            self.c(RESERVED + k as i32);
            self.c(site as i32);
            self.rt("path");
        } else {
            self.c(0);
        }
        self.set(res);
        self.else_();
        self.get(lst);
        self.rt("list_len");
        self.set(n);
        self.c(0);
        self.rt("list_new");
        self.tee(out);
        self.set(res);
        self.c(0);
        self.set(i);
        let exit = self.open(I::Block(BlockType::Empty));
        let lp = self.open(I::Loop(BlockType::Empty));
        self.get(i);
        self.get(n);
        self.i(I::I32GeS);
        self.br_if(exit);
        self.fuel_tick();
        self.get(lst);
        self.get(i);
        self.rt("list_get");
        self.set(item);
        let pred_is_tri = pred.is_tri();
        if pred_is_tri {
            self.emit_t(pred);
            self.c(1);
            self.i(I::I32Eq);
        } else {
            self.emit_h(pred);
            self.set(v);
            // a numeric "filter" is an index (Table 68)
            self.get(i);
            self.i(I::I32Eqz);
            self.open(I::If(BlockType::Empty));
            self.get(v);
            self.rt("is_number");
            self.open(I::If(BlockType::Empty));
            self.get(lst);
            self.get(v);
            self.c(site as i32);
            self.rt("index");
            if let Some(k) = key {
                self.c(RESERVED + k as i32);
                self.c(site as i32);
                self.rt("path");
            }
            self.set(res);
            self.br(exit);
            self.end();
            self.end();
            self.get(v);
            self.c(1);
            self.i(I::I32Eq);
        }
        self.open(I::If(BlockType::Empty));
        self.get(out);
        self.get(item);
        if let Some(k) = key {
            self.c(RESERVED + k as i32);
            self.rt("select_or_null");
        }
        self.rt("list_push");
        self.end();
        self.get(i);
        self.c(1);
        self.i(I::I32Add);
        self.set(i);
        self.br(lp);
        self.end(); // loop
        self.end(); // block
        self.end(); // if
        self.get(res);
    }

    /// Emit nested loops over `iters`, running `inner` for each combination.
    /// `exit` is the label to jump to when an iteration source is null (the
    /// caller must have set the result beforehand).
    fn emit_nested<F>(&mut self, iters: &[(Local, IterIr)], exit: u32, null_result: Option<(u32, i32)>, site: Site, inner: &mut F)
    where
        F: FnMut(&mut Self),
    {
        if iters.is_empty() {
            inner(self);
            return;
        }
        let (var, src) = &iters[0];
        let lst = self.tmp();
        let n = self.tmp();
        let i = self.tmp();
        match src {
            IterIr::Expr(e) => {
                self.emit_h(e);
                self.c(site as i32);
                self.rt("as_iter");
            }
            IterIr::Range(a, b) => {
                self.emit_h(a);
                self.emit_h(b);
                self.c(site as i32);
                self.rt("iter_range");
            }
        }
        self.tee(lst);
        self.c(ABSENT);
        self.i(I::I32Eq);
        self.open(I::If(BlockType::Empty));
        if let Some((res, v)) = null_result {
            self.c(v);
            self.set(res);
        }
        self.br(exit);
        self.end();
        self.get(lst);
        self.rt("list_len");
        self.set(n);
        self.c(0);
        self.set(i);
        let brk = self.open(I::Block(BlockType::Empty));
        let lp = self.open(I::Loop(BlockType::Empty));
        self.get(i);
        self.get(n);
        self.i(I::I32GeS);
        self.br_if(brk);
        self.fuel_tick();
        self.get(lst);
        self.get(i);
        self.rt("list_get");
        self.set(*var);
        self.emit_nested(&iters[1..], exit, null_result, site, inner);
        self.get(i);
        self.c(1);
        self.i(I::I32Add);
        self.set(i);
        self.br(lp);
        self.end();
        self.end();
    }

    fn emit_for(&mut self, iters: &[(Local, IterIr)], partial: Local, body: &Ir, site: Site) {
        let res = self.tmp();
        self.c(0);
        self.rt("list_new");
        self.tee(partial);
        self.set(res);
        let exit = self.open(I::Block(BlockType::Empty));
        let body = body.clone();
        self.emit_nested(iters, exit, Some((res, 0)), site, &mut |s: &mut Self| {
            s.get(partial);
            s.emit_h(&body);
            s.rt("list_push");
        });
        self.end();
        self.get(res);
    }

    fn emit_quant(&mut self, every: bool, iters: &[(Local, Ir)], body: &Ir, site: Site) {
        let acc = self.tmp();
        let t = self.tmp();
        self.c(if every { 1 } else { 0 });
        self.set(acc);
        let exit = self.open(I::Block(BlockType::Empty));
        let its: Vec<(Local, IterIr)> = iters.iter().map(|(l, e)| (*l, IterIr::Expr(e.clone()))).collect();
        let body = body.clone();
        self.emit_nested(&its, exit, Some((acc, 2)), site, &mut |s: &mut Self| {
            s.emit_t(&body);
            s.set(t);
            if every {
                // false decides; null taints
                s.get(t);
                s.i(I::I32Eqz);
                s.open(I::If(BlockType::Empty));
                s.c(0);
                s.set(acc);
                s.br(exit);
                s.end();
                s.get(t);
                s.c(2);
                s.i(I::I32Eq);
                s.open(I::If(BlockType::Empty));
                s.c(2);
                s.set(acc);
                s.end();
            } else {
                s.get(t);
                s.c(1);
                s.i(I::I32Eq);
                s.open(I::If(BlockType::Empty));
                s.c(1);
                s.set(acc);
                s.br(exit);
                s.end();
                s.get(t);
                s.c(2);
                s.i(I::I32Eq);
                s.open(I::If(BlockType::Empty));
                s.c(2);
                s.set(acc);
                s.end();
            }
        });
        self.end();
        self.get(acc);
    }

    fn finish(self) -> (Function, BTreeMap<u32, u32>) {
        let nlocals = self.next_local - self.nparams;
        let mut f = Function::new([(nlocals, ValType::I32)]);
        let cache = self.slot_cache.clone().unwrap_or_default();
        // prologue for the main function: load cached input slots
        for (slot, l) in &cache {
            f.instruction(&I::I32Const(*slot as i32));
            f.instruction(&I::Call(*self.imports.used.get("slot").expect("slot import")));
            f.instruction(&I::LocalSet(*l));
        }
        for ins in &self.code {
            f.instruction(ins);
        }
        f.instruction(&I::End);
        (f, cache)
    }
}

fn value_may_be_collection(_v: &Ir) -> bool {
    // constants are created by the compiler only for scalars (numbers,
    // strings, temporal literals)
    false
}

struct Bodies {
    funcs: Vec<Function>,
}

fn build_bodies(prog: &ProgramIr, imports: &mut Imports, n_imports: u32) -> Bodies {
    let mut funcs = Vec::new();
    // evaluate()
    {
        // ensure the slot import exists if we may need it in the prologue
        let mut b = FnBuilder::new(0, prog.main_locals, imports, n_imports, true);
        b.i(I::GlobalGet(G_MAX_FUEL));
        b.i(I::GlobalSet(G_FUEL));
        b.c(0);
        b.i(I::GlobalSet(G_DEPTH));
        b.emit_h(&prog.main);
        if !b.slot_cache.as_ref().unwrap().is_empty() {
            b.imports.idx("slot");
        }
        funcs.push(b.finish().0);
    }
    // __feel_apply(f, args)
    {
        let mut b = FnBuilder::new(2, 2, imports, n_imports, false);
        b.get(0);
        b.rt("fn_env");
        b.get(1);
        b.get(0);
        b.rt("fn_index");
        b.i(I::CallIndirect { type_index: 0, table_index: 0 });
        funcs.push(b.finish().0);
    }
    // __feel_invoke(f, args, named, site)
    {
        let mut b = FnBuilder::new(4, 4, imports, n_imports, false);
        let bound = b.tmp();
        b.get(0);
        b.rt("fn_kind");
        b.c(2);
        b.i(I::I32Eq);
        b.open(I::If(BlockType::Result(ValType::I32)));
        b.get(0);
        b.get(1);
        b.get(2);
        b.get(3);
        b.rt("bind_args");
        b.tee(bound);
        b.c(ABSENT);
        b.i(I::I32Eq);
        b.open(I::If(BlockType::Result(ValType::I32)));
        b.c(0);
        b.else_();
        b.get(0);
        b.rt("fn_env");
        b.get(bound);
        b.get(0);
        b.rt("fn_index");
        b.i(I::CallIndirect { type_index: 0, table_index: 0 });
        b.end();
        b.else_();
        b.get(0);
        b.get(1);
        b.get(2);
        b.get(3);
        b.rt("call_value");
        b.end();
        funcs.push(b.finish().0);
    }
    // __feel_set_limits(max_fuel: i64, max_depth: i32)
    {
        let mut f = Function::new([]);
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::GlobalSet(G_MAX_FUEL));
        f.instruction(&I::LocalGet(1));
        f.instruction(&I::GlobalSet(G_MAX_DEPTH));
        f.instruction(&I::End);
        funcs.push(f);
    }
    // user functions (env, args) -> result
    for fi in &prog.funcs {
        let mut b = FnBuilder::new(2, fi.nlocals, imports, n_imports, false);
        let ret = b.tmp();
        b.fuel_tick();
        b.i(I::GlobalGet(G_DEPTH));
        b.c(1);
        b.i(I::I32Add);
        b.i(I::GlobalSet(G_DEPTH));
        b.i(I::GlobalGet(G_DEPTH));
        b.i(I::GlobalGet(G_MAX_DEPTH));
        b.i(I::I32GtS);
        b.open(I::If(BlockType::Empty));
        b.rt("depth_exceeded");
        b.i(I::Unreachable);
        b.end();
        for (k, p) in fi.params.iter().enumerate() {
            b.get(1);
            b.c(k as i32);
            b.rt("list_get");
            b.set(*p);
        }
        for (k, c) in fi.captures.iter().enumerate() {
            b.get(0);
            b.c(k as i32);
            b.rt("list_get");
            b.set(*c);
        }
        b.emit_h(&fi.body);
        b.set(ret);
        b.i(I::GlobalGet(G_DEPTH));
        b.c(1);
        b.i(I::I32Sub);
        b.i(I::GlobalSet(G_DEPTH));
        b.get(ret);
        funcs.push(b.finish().0);
    }
    Bodies { funcs }
}

pub fn emit_module(input: &EmitInput) -> Emitted {
    // pass 1: discover the imports actually used
    let mut probe = Imports { used: BTreeMap::new(), order: Vec::new(), frozen: false };
    let _ = build_bodies(input.prog, &mut probe, 0);
    // deterministic import order: ABI declaration order
    let mut order: Vec<&'static str> = probe.order.clone();
    order.sort_by_key(|n| ABI_FUNCS.iter().position(|f| f.0 == *n).unwrap_or(usize::MAX));
    let mut imports = Imports { used: BTreeMap::new(), order: Vec::new(), frozen: false };
    for n in &order {
        imports.idx(n);
    }
    imports.frozen = true;
    let n_imports = imports.order.len() as u32;
    // pass 2: real bodies with final indices
    let bodies = build_bodies(input.prog, &mut imports, n_imports);

    let mut module = Module::new();
    // types: 0 = (i32,i32)->i32 (user functions, apply); then import shapes
    let mut types = TypeSection::new();
    let mut type_ids: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    types.ty().function([ValType::I32, ValType::I32], [ValType::I32]);
    type_ids.insert((2, 1), 0);
    let mut next_type = 1u32;
    let mut shape = |n: usize, r: usize, types: &mut TypeSection| -> u32 {
        if let Some(t) = type_ids.get(&(n, r)) {
            return *t;
        }
        types.ty().function(vec![ValType::I32; n], vec![ValType::I32; r]);
        let t = next_type;
        next_type += 1;
        type_ids.insert((n, r), t);
        t
    };
    let mut import_sec = ImportSection::new();
    for name in &imports.order {
        let (_, n, r) = ABI_FUNCS.iter().find(|f| f.0 == *name).expect("abi function");
        let t = shape(*n, *r, &mut types);
        import_sec.import(ABI_MODULE, &import_name(name), EntityType::Function(t));
    }
    let t_eval = shape(0, 1, &mut types);
    let t_invoke = shape(4, 1, &mut types);
    let t_limits = next_type;
    types.ty().function([ValType::I64, ValType::I32], []);

    let mut fsec = FunctionSection::new();
    fsec.function(t_eval);
    fsec.function(0);
    fsec.function(t_invoke);
    fsec.function(t_limits);
    for _ in &input.prog.funcs {
        fsec.function(0);
    }

    let nfuncs = input.prog.funcs.len() as u64;
    let mut tables = TableSection::new();
    tables.table(TableType { element_type: RefType::FUNCREF, table64: false, minimum: nfuncs, maximum: Some(nfuncs), shared: false });

    let mut globals = GlobalSection::new();
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(0));
    globals.global(GlobalType { val_type: ValType::I32, mutable: true, shared: false }, &ConstExpr::i32_const(0));
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(50_000_000));
    globals.global(GlobalType { val_type: ValType::I32, mutable: true, shared: false }, &ConstExpr::i32_const(512));

    let mut exports = ExportSection::new();
    exports.export("evaluate", ExportKind::Func, n_imports + F_EVALUATE);
    exports.export("__feel_apply", ExportKind::Func, n_imports + F_APPLY);
    exports.export("__feel_set_limits", ExportKind::Func, n_imports + F_SET_LIMITS);

    let mut elems = ElementSection::new();
    if nfuncs > 0 {
        let idxs: Vec<u32> = (0..nfuncs as u32).map(|k| n_imports + F_USER0 + k).collect();
        elems.active(Some(0), &ConstExpr::i32_const(0), Elements::Functions(Cow::Owned(idxs)));
    }

    let mut code = CodeSection::new();
    for f in &bodies.funcs {
        code.function(f);
    }

    module.section(&types);
    module.section(&import_sec);
    module.section(&fsec);
    module.section(&tables);
    module.section(&globals);
    module.section(&exports);
    if nfuncs > 0 {
        module.section(&elems);
    }
    module.section(&code);
    module.section(&CustomSection { name: Cow::Borrowed("feel.meta"), data: Cow::Borrowed(input.meta_json.as_bytes()) });
    module.section(&CustomSection { name: Cow::Borrowed("feel.consts"), data: Cow::Borrowed(input.consts_json.as_bytes()) });
    Emitted { bytes: module.finish(), imports: imports.order.iter().map(|s| s.to_string()).collect() }
}
