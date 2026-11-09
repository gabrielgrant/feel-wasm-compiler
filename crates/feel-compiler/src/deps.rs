//! Static dependency extraction for incremental recalculation.
//!
//! Reports which inputs (and which statically-known field paths of them) an
//! expression reads, whether it reads the clock, which host functions it may
//! call, and *why* the analysis may be incomplete (dynamic lookups, dynamic
//! calls, implicit filter names). A host that wants to be safe invalidates a
//! formula whenever any listed input changes; `exact == false` tells it that
//! field-level invalidation is not sound for that formula.

use crate::ir::*;
use feel_runtime::builtins;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dependencies {
    /// input name → statically known field paths read (empty path list with
    /// `whole: true` means the whole value is used)
    pub inputs: Vec<InputDependency>,
    pub clock: bool,
    pub host_functions: Vec<String>,
    /// true when all reads are fully described by `inputs` paths
    pub exact: bool,
    /// reasons the analysis is conservative
    pub uncertain: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InputDependency {
    pub name: String,
    pub whole: bool,
    pub paths: Vec<Vec<String>>,
}

struct Walk<'a> {
    slots: &'a [String],
    consts: &'a [String],
    uses: BTreeMap<u32, (bool, BTreeSet<Vec<String>>)>,
    clock: bool,
    host: BTreeSet<String>,
    uncertain: BTreeSet<String>,
}

fn const_str(consts: &[String], c: ConstId) -> String {
    serde_json::from_str::<String>(&consts[c as usize]).unwrap_or_default()
}

impl Walk<'_> {
    /// Path chain rooted at a slot: Path(Path(Slot(i), a), b) → (i, [a, b])
    fn chain(&self, ir: &Ir) -> Option<(u32, Vec<String>)> {
        match ir {
            Ir::Slot(i) => Some((*i, Vec::new())),
            Ir::Path(b, k, _) => {
                let (i, mut p) = self.chain(b)?;
                p.push(const_str(self.consts, *k));
                Some((i, p))
            }
            _ => None,
        }
    }

    fn walk(&mut self, ir: &Ir) {
        if let Some((i, p)) = self.chain(ir) {
            let e = self.uses.entry(i).or_default();
            if p.is_empty() {
                e.0 = true;
            } else {
                e.1.insert(p);
            }
            return;
        }
        match ir {
            Ir::ItemField { fallback, .. } => {
                self.uncertain.insert("implicit filter item names may shadow inputs".into());
                self.walk(fallback);
            }
            Ir::CallBuiltin { id, args, .. } => {
                let name = builtins::name_of(*id);
                if matches!(name, "now" | "today") {
                    self.clock = true;
                }
                if name == "get value" {
                    if !matches!(args.get(1), Some(Ir::Const(_))) {
                        self.uncertain.insert("get value with a computed key".into());
                    }
                }
                args.iter().for_each(|a| self.walk(a));
            }
            Ir::CallBuiltinNamed { id, args, .. } => {
                if matches!(builtins::name_of(*id), "now" | "today") {
                    self.clock = true;
                }
                args.iter().for_each(|(_, a)| self.walk(a));
            }
            Ir::CallValue { f, args, .. } => {
                self.call_target(f);
                args.iter().for_each(|a| self.walk(a));
            }
            Ir::CallValueNamed { f, args, .. } => {
                self.call_target(f);
                args.iter().for_each(|(_, a)| self.walk(a));
            }
            Ir::HostFn { name, .. } => {
                self.host.insert(const_str(self.consts, *name));
            }
            Ir::ExternalFn { mapping, .. } => {
                self.uncertain.insert("calls an externally defined function".into());
                self.walk(mapping);
            }
            Ir::BuiltinFn(id) => {
                if matches!(builtins::name_of(*id), "now" | "today") {
                    self.clock = true;
                }
            }
            Ir::Null | Ir::True | Ir::False | Ir::Const(_) | Ir::Local(_) | Ir::Slot(_) | Ir::Partial(_) => {}
            Ir::Path(a, _, _)
            | Ir::Neg(a, _)
            | Ir::IsNull(a, _)
            | Ir::Not(a, _)
            | Ir::InstanceOf(a, _)
            | Ir::Coerce(a, _, _)
            | Ir::PointRange { value: a, .. } => {
                self.walk(a)
            }
            Ir::Bin(_, a, b, _) | Ir::Cmp(_, a, b, _) | Ir::And(a, b) | Ir::Or(a, b) | Ir::Index(a, b, _) => {
                self.walk(a);
                self.walk(b);
            }
            Ir::If(a, b, c) => {
                self.walk(a);
                self.walk(b);
                self.walk(c);
            }
            Ir::Let(_, v, b) => {
                self.walk(v);
                self.walk(b);
            }
            Ir::List(xs) => xs.iter().for_each(|x| self.walk(x)),
            Ir::Context(es) => es.iter().for_each(|(_, _, x)| self.walk(x)),
            Ir::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.walk(s);
                }
                if let Some(e) = end {
                    self.walk(e);
                }
            }
            Ir::In { tests, .. } => {
                for t in tests {
                    match t {
                        TestIr::Cmp(_, e, _) => self.walk(e),
                        TestIr::Interval { start, end, .. } => {
                            self.walk(start);
                            self.walk(end);
                        }
                        TestIr::Value { value, .. } => self.walk(value),
                    }
                }
            }
            Ir::Filter { list, pred, .. } | Ir::FilterPath { list, pred, .. } => {
                self.walk(list);
                self.walk(pred);
            }
            Ir::For { iters, body, .. } => {
                for (_, s) in iters {
                    match s {
                        IterIr::Expr(e) => self.walk(e),
                        IterIr::Range(a, b) => {
                            self.walk(a);
                            self.walk(b);
                        }
                    }
                }
                self.walk(body);
            }
            Ir::Quant { iters, body, .. } => {
                iters.iter().for_each(|(_, e)| self.walk(e));
                self.walk(body);
            }
            Ir::Closure { captures, .. } => captures.iter().for_each(|x| self.walk(x)),
        }
    }

    fn call_target(&mut self, f: &Ir) {
        match f {
            Ir::HostFn { .. } | Ir::BuiltinFn(_) => self.walk(f),
            _ => {
                self.uncertain.insert("invokes a function value computed at runtime".into());
                self.walk(f);
            }
        }
    }
}

pub fn extract(prog: &ProgramIr, slots: &[String], consts: &[String]) -> Dependencies {
    let mut w = Walk {
        slots,
        consts,
        uses: BTreeMap::new(),
        clock: false,
        host: BTreeSet::new(),
        uncertain: BTreeSet::new(),
    };
    w.walk(&prog.main);
    for f in &prog.funcs {
        w.walk(&f.body);
    }
    let inputs = w
        .uses
        .iter()
        .filter(|(i, _)| w.slots.get(**i as usize).map(|n| n != "?").unwrap_or(false))
        .map(|(i, (whole, paths))| InputDependency {
            name: w.slots[*i as usize].clone(),
            whole: *whole,
            paths: paths.iter().cloned().collect(),
        })
        .collect();
    let uncertain: Vec<String> = w.uncertain.into_iter().collect();
    Dependencies {
        inputs,
        clock: w.clock,
        host_functions: w.host.into_iter().collect(),
        exact: uncertain.is_empty(),
        uncertain,
    }
}
