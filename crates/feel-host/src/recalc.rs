//! Incremental recalculation with extracted dependencies and a module cache.
//!
//! A [`Workbook`] holds named FEEL formulas that may reference input cells
//! and other formulas by name. Each formula is compiled once (through a
//! [`ModuleCache`] keyed by source, schema, dialect, options and
//! compiler/runtime/ABI versions), its static dependencies are recorded, and
//! when inputs change only the formulas that (transitively) depend on them are
//! re-evaluated. Formulas whose dependency analysis is not exact (dynamic
//! lookups, dynamic calls, clock access) are conservatively re-evaluated on
//! every change. (A spreadsheet grid is out of scope.)

use crate::{ClockSpec, Engine, EvalOptions, Instance, InstanceOptions};
use feel_compiler::{compile, CompileOptions, Dependencies};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Compiled modules keyed by `CompileOptions::cache_key`.
#[derive(Default)]
pub struct ModuleCache {
    entries: HashMap<String, (Vec<u8>, Dependencies)>,
    pub hits: u64,
    pub misses: u64,
}

impl ModuleCache {
    pub fn get_or_compile(
        &mut self,
        source: &str,
        opts: &CompileOptions,
    ) -> Result<(Vec<u8>, Dependencies), String> {
        let key = opts.cache_key(source);
        if let Some(e) = self.entries.get(&key) {
            self.hits += 1;
            return Ok(e.clone());
        }
        self.misses += 1;
        let out = compile(source, opts);
        let wasm = out.wasm.ok_or_else(|| {
            out.diagnostics
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let deps = out.metadata.map(|m| m.dependencies).unwrap_or_default();
        self.entries.insert(key, (wasm.clone(), deps.clone()));
        Ok((wasm, deps))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

struct Formula {
    inst: Instance,
    deps: Dependencies,
    value: String,
}

pub struct Workbook {
    engine: Engine,
    pub cache: ModuleCache,
    inputs: BTreeMap<String, String>,
    formulas: BTreeMap<String, Formula>,
    order: Vec<String>,
    /// names evaluated by the last `set_inputs`/`recalc_all`
    pub last_evaluated: Vec<String>,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    pub fn new() -> Self {
        Workbook {
            engine: Engine::new(),
            cache: ModuleCache::default(),
            inputs: BTreeMap::new(),
            formulas: BTreeMap::new(),
            order: Vec::new(),
            last_evaluated: Vec::new(),
        }
    }

    /// Define (or redefine) a formula. Formulas are evaluated in definition
    /// order, so a formula may reference formulas defined before it.
    pub fn define(&mut self, name: &str, source: &str) -> Result<(), String> {
        let mut opts = CompileOptions::default();
        // multi-word input names are made known to the parser (sorted, so
        // that the cache key only depends on the declared inputs)
        opts.known_names = self.inputs.keys().cloned().collect();
        let (wasm, deps) = self.cache.get_or_compile(source, &opts)?;
        let p = self.engine.load(&wasm).map_err(|e| e.to_string())?;
        let inst = p
            .instantiate(&self.engine, InstanceOptions::default())
            .map_err(|e| e.to_string())?;
        if !self.order.iter().any(|n| n == name) {
            self.order.push(name.to_string());
        }
        self.formulas.insert(
            name.to_string(),
            Formula {
                inst,
                deps,
                value: "null".into(),
            },
        );
        Ok(())
    }

    fn env_json(&self) -> String {
        let mut s = String::from("{");
        let mut first = true;
        for (k, v) in self
            .inputs
            .iter()
            .map(|(k, v)| (k, v.as_str()))
            .chain(self.formulas.iter().map(|(k, f)| (k, f.value.as_str())))
        {
            if !first {
                s.push(',');
            }
            first = false;
            feel_runtime::json::write_json_string(&mut s, k);
            s.push(':');
            s.push_str(v);
        }
        s.push('}');
        s
    }

    fn eval(&mut self, name: &str) {
        let env = self.env_json();
        let f = self.formulas.get_mut(name).unwrap();
        let r = f.inst.evaluate(
            &env,
            &EvalOptions {
                clock: ClockSpec::System,
            },
        );
        f.value = r.result.unwrap_or_else(|| "null".into());
        self.last_evaluated.push(name.to_string());
    }

    pub fn recalc_all(&mut self) {
        self.last_evaluated.clear();
        for n in self.order.clone() {
            self.eval(&n);
        }
    }

    /// Change input cells (FEEL-JSON values) and recompute only what depends
    /// on them.
    pub fn set_inputs(&mut self, changes: &[(&str, &str)]) {
        let mut dirty: BTreeSet<String> = BTreeSet::new();
        for (k, v) in changes {
            self.inputs.insert(k.to_string(), v.to_string());
            dirty.insert(k.to_string());
        }
        self.last_evaluated.clear();
        for n in self.order.clone() {
            let f = &self.formulas[&n];
            let affected = !f.deps.exact
                || f.deps.clock
                || f.deps.inputs.iter().any(|d| dirty.contains(&d.name));
            if affected {
                let before = f.value.clone();
                self.eval(&n);
                if self.formulas[&n].value != before {
                    dirty.insert(n.clone());
                }
            }
        }
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.formulas
            .get(name)
            .map(|f| f.value.as_str())
            .or_else(|| self.inputs.get(name).map(|s| s.as_str()))
    }
}
