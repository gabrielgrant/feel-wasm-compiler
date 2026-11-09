//! Semantics-preserving IR optimizations (selectable; see `CompileOptions::optimize`).
//!
//! * **Constant folding** evaluates operator/builtin applications whose
//!   operands are constants *with the shared runtime itself* (the very code
//!   the generated module would call), one operation at a time in source
//!   order — so rounding order is unchanged. A fold is only applied when the
//!   operation produced no evaluation diagnostic and a scalar result, so
//!   diagnostics and null behaviour are preserved.
//! * **Filter/projection fusion** rewrites `list[pred].key` into a single
//!   loop (`FilterPath`) that selects `key` from each kept item, avoiding the
//!   intermediate list.
//!
//! Differential tests (`tests/differential.rs`) compare optimized and
//! unoptimized modules on the semantic vectors.

use crate::ast::{BinOp, CmpOp};
use crate::ir::*;
use crate::lower::ConstPool;
use feel_runtime::{builtins, json, ops, NoCallbacks, Runtime, Value, H, NULL};

pub struct Folder<'a> {
    pub consts: &'a mut ConstPool,
    rt: Runtime,
    pub folded: u32,
    pub fused: u32,
}

enum Folded {
    Null,
    True,
    False,
    Const(ConstId),
}

impl<'a> Folder<'a> {
    pub fn new(consts: &'a mut ConstPool) -> Self {
        Folder {
            consts,
            rt: Runtime::new(),
            folded: 0,
            fused: 0,
        }
    }

    fn load(&mut self, ir: &Ir) -> Option<H> {
        match ir {
            Ir::Null => Some(NULL),
            Ir::True => Some(feel_runtime::TRUE),
            Ir::False => Some(feel_runtime::FALSE),
            Ir::Const(c) => json::decode(&mut self.rt, &self.consts.items[*c as usize]).ok(),
            _ => None,
        }
    }

    fn result(&mut self, h: H) -> Option<Folded> {
        if !self.rt.diags.is_empty() || self.rt.aborted() {
            return None;
        }
        match self.rt.get(h) {
            Value::Null => Some(Folded::Null),
            Value::Bool(true) => Some(Folded::True),
            Value::Bool(false) => Some(Folded::False),
            Value::Num(_)
            | Value::Str(_)
            | Value::Date(_)
            | Value::Time(_)
            | Value::DateTime(_)
            | Value::Dtd(_)
            | Value::Ymd(_) => {
                let j = json::encode(&self.rt, h);
                let j = match self.rt.get(h) {
                    // keep the scale of numbers exactly (plain notation loses negative scales)
                    Value::Num(d) if d.scale() < 0 => {
                        let mut s = String::from("{\"@number\":");
                        json::write_json_string(&mut s, &d.to_scientific_string());
                        s.push('}');
                        s
                    }
                    _ => j,
                };
                Some(Folded::Const(self.consts.add_json(j)))
            }
            _ => None,
        }
    }

    fn tri_result(&mut self, t: i32) -> Option<Folded> {
        if !self.rt.diags.is_empty() || self.rt.aborted() {
            return None;
        }
        Some(match t {
            1 => Folded::True,
            0 => Folded::False,
            _ => Folded::Null,
        })
    }

    fn to_ir(f: Folded) -> Ir {
        match f {
            Folded::Null => Ir::Null,
            Folded::True => Ir::True,
            Folded::False => Ir::False,
            Folded::Const(c) => Ir::Const(c),
        }
    }

    fn is_const(ir: &Ir) -> bool {
        matches!(ir, Ir::Null | Ir::True | Ir::False | Ir::Const(_))
    }

    fn try_fold(&mut self, ir: &Ir) -> Option<Ir> {
        self.rt.begin_eval();
        let r = match ir {
            Ir::Bin(op, a, b, _) if Self::is_const(a) && Self::is_const(b) => {
                let (x, y) = (self.load(a)?, self.load(b)?);
                let h = match op {
                    BinOp::Add => ops::add(&mut self.rt, x, y, 0),
                    BinOp::Sub => ops::sub(&mut self.rt, x, y, 0),
                    BinOp::Mul => ops::mul(&mut self.rt, x, y, 0),
                    BinOp::Div => ops::div(&mut self.rt, x, y, 0),
                    BinOp::Pow => ops::pow(&mut self.rt, x, y, 0),
                };
                self.result(h)?
            }
            Ir::Neg(a, _) if Self::is_const(a) => {
                let x = self.load(a)?;
                let h = ops::neg(&mut self.rt, x, 0);
                self.result(h)?
            }
            Ir::Cmp(op, a, b, _) if Self::is_const(a) && Self::is_const(b) => {
                let (x, y) = (self.load(a)?, self.load(b)?);
                let t = match op {
                    CmpOp::Eq => ops::eq_tri(&mut self.rt, x, y, 0),
                    CmpOp::Ne => ops::tri_not(ops::eq_tri(&mut self.rt, x, y, 0)),
                    CmpOp::Lt => ops::lt(&mut self.rt, x, y, 0),
                    CmpOp::Le => ops::le(&mut self.rt, x, y, 0),
                    CmpOp::Gt => ops::gt(&mut self.rt, x, y, 0),
                    CmpOp::Ge => ops::ge(&mut self.rt, x, y, 0),
                };
                self.tri_result(t)?
            }
            Ir::IsNull(a, neg) if Self::is_const(a) => {
                let is_null = matches!(**a, Ir::Null);
                if is_null != *neg {
                    Folded::True
                } else {
                    Folded::False
                }
            }
            Ir::Path(a, k, _) if Self::is_const(a) && !matches!(**a, Ir::Null) => {
                let x = self.load(a)?;
                let key = self.load(&Ir::Const(*k))?;
                let h = ops::path(&mut self.rt, x, key, 0);
                self.result(h)?
            }
            Ir::CallBuiltin { id, args, .. } if args.iter().all(Self::is_const) => {
                let b = &builtins::BUILTINS[*id as usize];
                if !b.deterministic || b.extension != 0 {
                    return None;
                }
                let mut hs = Vec::new();
                for a in args {
                    hs.push(self.load(a)?);
                }
                let h = builtins::call(&mut NoCallbacks(&mut self.rt), *id, &hs, 0);
                self.result(h)?
            }
            Ir::Not(a, _) if matches!(**a, Ir::True | Ir::False) => {
                if matches!(**a, Ir::True) {
                    Folded::False
                } else {
                    Folded::True
                }
            }
            _ => return None,
        };
        self.folded += 1;
        Some(Self::to_ir(r))
    }

    /// Rewrite bottom-up.
    pub fn run(&mut self, ir: &mut Ir) {
        self.children(ir);
        if let Some(n) = self.try_fold(ir) {
            *ir = n;
            return;
        }
        // control flow with constant conditions
        match ir {
            Ir::If(c, t, f) => match **c {
                Ir::True => *ir = (**t).clone(),
                Ir::False | Ir::Null => *ir = (**f).clone(),
                _ => {}
            },
            Ir::And(a, b) => {
                if matches!(**a, Ir::False) {
                    *ir = Ir::False;
                } else if matches!(**a, Ir::True) && b.is_tri() {
                    *ir = (**b).clone();
                }
            }
            Ir::Or(a, b) => {
                if matches!(**a, Ir::True) {
                    *ir = Ir::True;
                } else if matches!(**a, Ir::False) && b.is_tri() {
                    *ir = (**b).clone();
                }
            }
            Ir::Path(base, key, site) => {
                if let Ir::Filter { .. } = **base {
                    if let Ir::Filter {
                        list, item, pred, ..
                    } = (**base).clone()
                    {
                        *ir = Ir::FilterPath {
                            list,
                            item,
                            pred,
                            key: *key,
                            site: *site,
                        };
                        self.fused += 1;
                    }
                }
            }
            _ => {}
        }
    }

    fn children(&mut self, ir: &mut Ir) {
        match ir {
            Ir::Null
            | Ir::True
            | Ir::False
            | Ir::Const(_)
            | Ir::Local(_)
            | Ir::Slot(_)
            | Ir::BuiltinFn(_)
            | Ir::HostFn { .. }
            | Ir::Partial(_) => {}
            Ir::ItemField { fallback, .. } => self.run(fallback),
            Ir::Path(a, _, _)
            | Ir::Neg(a, _)
            | Ir::IsNull(a, _)
            | Ir::Not(a, _)
            | Ir::InstanceOf(a, _)
            | Ir::Coerce(a, _, _)
            | Ir::PointRange { value: a, .. } => self.run(a),
            Ir::ExternalFn { mapping, .. } => self.run(mapping),
            Ir::Bin(_, a, b, _)
            | Ir::Cmp(_, a, b, _)
            | Ir::And(a, b)
            | Ir::Or(a, b)
            | Ir::Index(a, b, _) => {
                self.run(a);
                self.run(b);
            }
            Ir::If(a, b, c) => {
                self.run(a);
                self.run(b);
                self.run(c);
            }
            Ir::Let(_, v, b) => {
                self.run(v);
                self.run(b);
            }
            Ir::List(items) => items.iter_mut().for_each(|x| self.run(x)),
            Ir::Context(es) => es.iter_mut().for_each(|(_, _, x)| self.run(x)),
            Ir::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.run(s);
                }
                if let Some(e) = end {
                    self.run(e);
                }
            }
            Ir::In { tests, .. } => {
                for t in tests {
                    match t {
                        TestIr::Cmp(_, e, _) => self.run(e),
                        TestIr::Interval { start, end, .. } => {
                            self.run(start);
                            self.run(end);
                        }
                        TestIr::Value { value, .. } => self.run(value),
                    }
                }
            }
            Ir::Filter { list, pred, .. } | Ir::FilterPath { list, pred, .. } => {
                self.run(list);
                self.run(pred);
            }
            Ir::For { iters, body, .. } => {
                for (_, src) in iters.iter_mut() {
                    match src {
                        IterIr::Expr(e) => self.run(e),
                        IterIr::Range(a, b) => {
                            self.run(a);
                            self.run(b);
                        }
                    }
                }
                self.run(body);
            }
            Ir::Quant { iters, body, .. } => {
                for (_, e) in iters.iter_mut() {
                    self.run(e);
                }
                self.run(body);
            }
            Ir::CallBuiltin { args, .. } | Ir::CallValue { args, .. } => {
                args.iter_mut().for_each(|x| self.run(x));
                if let Ir::CallValue { f, .. } = ir {
                    self.run(f);
                }
            }
            Ir::CallBuiltinNamed { args, .. } => args.iter_mut().for_each(|(_, x)| self.run(x)),
            Ir::CallValueNamed { f, args, .. } => {
                self.run(f);
                args.iter_mut().for_each(|(_, x)| self.run(x));
            }
            Ir::Closure { captures, .. } => captures.iter_mut().for_each(|x| self.run(x)),
        }
    }
}

pub fn optimize(prog: &mut ProgramIr, consts: &mut ConstPool) -> (u32, u32) {
    let mut f = Folder::new(consts);
    f.run(&mut prog.main);
    for func in prog.funcs.iter_mut() {
        f.run(&mut func.body);
    }
    (f.folded, f.fused)
}
