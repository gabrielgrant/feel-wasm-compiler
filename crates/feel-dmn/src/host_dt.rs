//! Host-orchestrated decision tables.
//!
//! Each cell of a decision table is compiled separately with the public API:
//! input expressions as ordinary expressions, input entries as **unary
//! tests** (`Entry::UnaryTests`, candidate `?`), output entries as ordinary
//! expressions. Evaluation feeds the value of each input expression as the
//! *candidate* of the unary-test modules of its column, collects the outputs
//! of matching rules and applies the hit policy in Rust. This exercises the
//! unary-test entry point exactly as a DMN engine would use it, and is
//! cross-checked against the lowered (single-module) decision tables.

use crate::model::DecisionTable;
use feel_compiler::{compile, CompileOptions, Entry};
use feel_host::{ClockSpec, Engine, EvalOptions, Instance, InstanceOptions};
use feel_runtime::{builtins, json, ops, NoCallbacks, Runtime, H};

pub struct HostDecisionTable {
    inputs: Vec<Instance>,
    /// rule → column → unary-tests module (None for `-`)
    entries: Vec<Vec<Option<Instance>>>,
    outputs: Vec<Vec<Instance>>,
    output_names: Vec<String>,
    output_values: Vec<Option<String>>,
    defaults: Vec<Option<Instance>>,
    hit_policy: String,
    aggregation: Option<String>,
    pub modules_compiled: usize,
}

fn build(engine: &Engine, src: &str, entry: Entry, names: &[String]) -> Result<Instance, String> {
    let opts = CompileOptions { entry, known_names: names.to_vec(), ..Default::default() };
    let out = compile(src, &opts);
    let wasm = out.wasm.ok_or_else(|| {
        format!("`{src}`: {}", out.diagnostics.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("; "))
    })?;
    let p = engine.load(&wasm).map_err(|e| e.to_string())?;
    p.instantiate(engine, InstanceOptions::default()).map_err(|e| e.to_string())
}

fn eo() -> EvalOptions {
    EvalOptions { clock: ClockSpec::Fixed("2024-06-15T12:00:00Z".into()) }
}

impl HostDecisionTable {
    pub fn compile(engine: &Engine, dt: &DecisionTable, names: &[String]) -> Result<Self, String> {
        let mut n = 0;
        let mut inputs = Vec::new();
        for i in &dt.inputs {
            inputs.push(build(engine, &i.expr, Entry::Expression, names)?);
            n += 1;
        }
        let mut entries = Vec::new();
        let mut outputs = Vec::new();
        for r in &dt.rules {
            let mut row = Vec::new();
            for (k, t) in r.inputs.iter().enumerate() {
                let t = t.trim();
                if t == "-" || t.is_empty() {
                    match dt.inputs.get(k).and_then(|i| i.input_values.as_ref()) {
                        Some(v) if !v.trim().is_empty() => {
                            row.push(Some(build(engine, v, Entry::UnaryTests, names)?));
                            n += 1;
                        }
                        _ => row.push(None),
                    }
                } else {
                    row.push(Some(build(engine, t, Entry::UnaryTests, names)?));
                    n += 1;
                }
            }
            entries.push(row);
            let mut orow = Vec::new();
            for o in &r.outputs {
                orow.push(build(engine, o, Entry::Expression, names)?);
                n += 1;
            }
            outputs.push(orow);
        }
        let mut defaults = Vec::new();
        for o in &dt.outputs {
            defaults.push(match &o.default {
                Some(d) => {
                    n += 1;
                    Some(build(engine, d, Entry::Expression, names)?)
                }
                None => None,
            });
        }
        Ok(HostDecisionTable {
            inputs,
            entries,
            outputs,
            output_names: dt.outputs.iter().enumerate().map(|(i, o)| o.name.clone().unwrap_or_else(|| format!("output{}", i + 1))).collect(),
            output_values: dt.outputs.iter().map(|o| o.output_values.clone()).collect(),
            defaults,
            hit_policy: dt.hit_policy.to_ascii_uppercase(),
            aggregation: dt.aggregation.clone().map(|a| a.to_ascii_uppercase()),
            modules_compiled: n,
        })
    }

    fn val(inst: &mut Instance, inputs: &str) -> Result<String, String> {
        let r = inst.evaluate(inputs, &eo());
        match (r.result, r.abort) {
            (Some(v), _) => Ok(v),
            (None, Some(a)) => Err(format!("{}: {}", a.code, a.message)),
            _ => Err("no result".into()),
        }
    }

    /// Evaluate the table for `inputs` (FEEL-JSON object). Returns FEEL-JSON.
    pub fn evaluate(&mut self, inputs: &str) -> Result<String, String> {
        // 1. input expressions produce candidate values
        let mut cands = Vec::new();
        for i in self.inputs.iter_mut() {
            cands.push(Self::val(i, inputs)?);
        }
        // 2. unary tests consume them
        let mut matched: Vec<usize> = Vec::new();
        for (r, row) in self.entries.iter_mut().enumerate() {
            let mut ok = true;
            for (k, cell) in row.iter_mut().enumerate() {
                if let Some(m) = cell {
                    let cand = cands.get(k).map(|s| s.as_str()).unwrap_or("null");
                    let res = m.evaluate_unary(cand, inputs, &eo());
                    if res.result.as_deref() != Some("true") {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                matched.push(r);
            }
        }
        // 3. output expressions of matched rules
        let single = self.output_names.len() <= 1;
        let mut outs: Vec<String> = Vec::new();
        for r in &matched {
            let vals: Vec<String> = self.outputs[*r].iter_mut().map(|o| Self::val(o, inputs)).collect::<Result<_, _>>()?;
            outs.push(if single { vals.into_iter().next().unwrap_or_else(|| "null".into()) } else { ctx_json(&self.output_names, &vals) });
        }
        // 4. hit policy
        let default = || -> Result<String, String> {
            Ok("null".to_string())
        };
        if outs.is_empty() {
            if self.defaults.iter().any(|d| d.is_some()) {
                let mut vals = Vec::new();
                for d in self.defaults.iter_mut() {
                    vals.push(match d {
                        Some(i) => Self::val(i, inputs)?,
                        None => "null".into(),
                    });
                }
                return Ok(if single { vals.remove(0) } else { ctx_json(&self.output_names, &vals) });
            }
            if self.hit_policy.starts_with('C') && self.aggregation.as_deref() == Some("COUNT") {
                return Ok("0".into());
            }
            return default();
        }
        let mut rt = Runtime::new();
        let hs: Vec<H> = outs.iter().map(|o| json::decode(&mut rt, o).unwrap_or(0)).collect();
        let pick = |rt: &Runtime, h: H| json::encode(rt, h);
        Ok(match self.hit_policy.as_str() {
            "UNIQUE" | "U" => {
                if hs.len() == 1 {
                    pick(&rt, hs[0])
                } else {
                    "null".into()
                }
            }
            "ANY" | "A" => {
                if hs.iter().all(|h| ops::eq(&rt, *h, hs[0]) == Some(true)) {
                    pick(&rt, hs[0])
                } else {
                    "null".into()
                }
            }
            "FIRST" | "F" => pick(&rt, hs[0]),
            "RULE ORDER" | "R" => pick_list(&mut rt, hs),
            "PRIORITY" | "P" | "OUTPUT ORDER" | "O" => {
                let sorted = self.priority_sort(&mut rt, hs);
                if self.hit_policy.starts_with('P') {
                    pick(&rt, sorted[0])
                } else {
                    pick_list(&mut rt, sorted)
                }
            }
            _ => {
                let l = rt.list(hs);
                let name = match self.aggregation.as_deref() {
                    Some("SUM") => "sum",
                    Some("COUNT") => "count",
                    Some("MIN") => "min",
                    Some("MAX") => "max",
                    _ => return Ok(json::encode(&rt, l)),
                };
                let id = builtins::lookup(name).unwrap();
                let r = builtins::call(&mut NoCallbacks(&mut rt), id, &[l], 0);
                json::encode(&rt, r)
            }
        })
    }

    fn priority_sort(&self, rt: &mut Runtime, hs: Vec<H>) -> Vec<H> {
        // priority lists from output values (literal values)
        let mut lists: Vec<Option<Vec<H>>> = Vec::new();
        for ov in &self.output_values {
            lists.push(ov.as_ref().and_then(|v| {
                let out = feel_host::eval_once(&format!("[{v}]"), "{}", &CompileOptions::default()).ok()?;
                let h = json::decode(rt, out.result.as_deref()?).ok()?;
                rt.list_items(h)
            }));
        }
        let single = self.output_names.len() <= 1;
        let names = self.output_names.clone();
        let key = |rt: &mut Runtime, h: H| -> Vec<usize> {
            let mut k = Vec::new();
            for (i, l) in lists.iter().enumerate() {
                let Some(l) = l else { continue };
                let v = if single {
                    h
                } else {
                    let kh = rt.str(&names[i]);
                    ops::path(rt, h, kh, 0)
                };
                k.push(l.iter().position(|x| ops::eq(rt, *x, v) == Some(true)).unwrap_or(usize::MAX));
            }
            k
        };
        let mut keyed: Vec<(Vec<usize>, H)> = hs.into_iter().map(|h| (key(rt, h), h)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed.into_iter().map(|(_, h)| h).collect()
    }
}

fn pick_list(rt: &mut Runtime, hs: Vec<H>) -> String {
    let l = rt.list(hs);
    json::encode(rt, l)
}

fn ctx_json(names: &[String], vals: &[String]) -> String {
    let mut s = String::from("{");
    for (i, (n, v)) in names.iter().zip(vals.iter()).enumerate() {
        if i > 0 {
            s.push(',');
        }
        json::write_json_string(&mut s, n);
        s.push(':');
        s.push_str(v);
    }
    s.push('}');
    s
}
