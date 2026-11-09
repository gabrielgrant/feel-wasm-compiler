//! Native execution of compiled FEEL modules.
//!
//! The emitted module is executed by [wasmi](https://github.com/wasmi-labs/wasmi)
//! (a WebAssembly engine). Its `feel_rt_v1` imports are bound to the shared
//! Rust runtime compiled natively — the same source that produces
//! `runtime.wasm` for browsers.
//!
//! ```
//! use feel_compiler::{compile, CompileOptions};
//! use feel_host::{Engine, EvalOptions};
//! let out = compile("a + b", &CompileOptions::default());
//! let engine = Engine::new();
//! let program = engine.load(out.wasm.as_ref().unwrap()).unwrap();
//! let mut inst = program.instantiate(&engine, Default::default()).unwrap();
//! let r = inst.evaluate(r#"{"a": 0.1, "b": 0.2}"#, &EvalOptions::default());
//! assert_eq!(r.result.as_deref(), Some("0.3"));
//! ```

pub mod bindings;
pub mod inspect;

use feel_compiler::{read_module_sections, validate_module, Metadata};
use feel_runtime::abi::{self, ABI_MODULE};
use feel_runtime::{json, Cx, Limits, Runtime, Value, H, NULL};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use wasmi::{Caller, Linker, Module, Store, TypedFunc};

/// A host function: receives FEEL-JSON arguments, returns FEEL-JSON (or an
/// error message, which yields `null` plus a diagnostic).
pub type HostFn = Rc<dyn Fn(&[String]) -> Result<String, String>>;

/// Host capabilities bound into an instance.
#[derive(Clone, Default)]
pub struct HostFunctions {
    /// by name: schema-declared host functions, and external FEEL functions
    /// keyed as `java:<class>.<method signature>` / `pmml:<document>#<model>`
    pub functions: HashMap<String, HostFn>,
}

impl HostFunctions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, name: &str, f: impl Fn(&[String]) -> Result<String, String> + 'static) -> &mut Self {
        self.functions.insert(name.to_string(), Rc::new(f));
        self
    }
}

#[derive(Clone, Default)]
pub struct InstanceOptions {
    pub limits: Limits,
    pub host: HostFunctions,
}

/// The clock capability for one evaluation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ClockSpec {
    /// `now()`/`today()` yield null with a diagnostic.
    None,
    /// The host's current UTC time, sampled once at the start of evaluation.
    #[default]
    System,
    /// A fixed instant (date and time lexical form) for reproducible runs.
    Fixed(String),
}

#[derive(Clone, Debug, Default)]
pub struct EvalOptions {
    pub clock: ClockSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalDiagnostic {
    pub code: String,
    pub message: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AbortInfo {
    /// "limit" or "boundary"
    pub kind: String,
    pub code: String,
    pub message: String,
}

/// The outcome of one evaluation. Exactly one of `result` / `abort` is set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    /// FEEL-JSON of the result (`"null"` for a FEEL null result).
    pub result: Option<String>,
    pub diagnostics: Vec<EvalDiagnostic>,
    pub abort: Option<AbortInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostError {
    InvalidModule(String),
    Incompatible(String),
    Link(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::InvalidModule(m) => write!(f, "invalid module: {m}"),
            HostError::Incompatible(m) => write!(f, "{m}"),
            HostError::Link(m) => write!(f, "link error: {m}"),
        }
    }
}
impl std::error::Error for HostError {}

pub struct Engine {
    engine: wasmi::Engine,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Engine { engine: wasmi::Engine::default() }
    }

    /// Validate and compile a FEEL module.
    pub fn load(&self, bytes: &[u8]) -> Result<Program, HostError> {
        validate_module(bytes).map_err(HostError::InvalidModule)?;
        let (meta, consts) = read_module_sections(bytes).map_err(HostError::Incompatible)?;
        // the module may import only feel_rt_v1 functions
        let module = Module::new(&self.engine, bytes).map_err(|e| HostError::InvalidModule(e.to_string()))?;
        for imp in module.imports() {
            if imp.module() != ABI_MODULE {
                return Err(HostError::Incompatible(format!("unexpected import module '{}'", imp.module())));
            }
            let name = imp.name().strip_prefix("f_").unwrap_or("");
            if abi::abi_index(name).is_none() {
                return Err(HostError::Incompatible(format!("unknown runtime import '{}'", imp.name())));
            }
        }
        Ok(Program { module, meta, consts })
    }
}

pub struct Program {
    module: Module,
    pub meta: Metadata,
    consts: String,
}

pub struct HostState {
    pub rt: Runtime,
    apply: Option<TypedFunc<(i32, i32), i32>>,
    host: HostFunctions,
}

struct NativeCx<'a, 'b> {
    caller: &'a mut Caller<'b, HostState>,
}

impl Cx for NativeCx<'_, '_> {
    fn rt(&mut self) -> &mut Runtime {
        &mut self.caller.data_mut().rt
    }
    fn apply(&mut self, f: H, args: H) -> H {
        let Some(apply) = self.caller.data().apply else { return NULL };
        match apply.call(&mut *self.caller, (f as i32, args as i32)) {
            Ok(h) => h as u32,
            Err(e) => {
                let rt = &mut self.caller.data_mut().rt;
                if !rt.aborted() {
                    rt.boundary("FEEL-B503", format!("trap in callback: {e}"));
                }
                NULL
            }
        }
    }
    fn call_host(&mut self, f: H, args: H) -> H {
        let host = self.caller.data().host.clone();
        call_host_impl(&mut self.caller.data_mut().rt, &host, f, args)
    }
}

/// Resolve and invoke a host / external function value.
pub fn call_host_impl(rt: &mut Runtime, host: &HostFunctions, f: H, args: H) -> H {
    let key = match rt.get(f) {
        Value::Func(feel_runtime::Func::Host { name, .. }) => name.to_string(),
        Value::Func(feel_runtime::Func::External { mapping, .. }) => external_key(rt, *mapping),
        _ => return NULL,
    };
    let Some(func) = host.functions.get(&key).cloned() else {
        rt.diag("FEEL-E360", 0, format!("no host binding for external function '{key}'"));
        return NULL;
    };
    let arg_json: Vec<String> = rt.list_items(args).unwrap_or_default().iter().map(|h| json::encode(rt, *h)).collect();
    match func(&arg_json) {
        Ok(r) => match json::decode(rt, &r) {
            Ok(h) => h,
            Err(e) => {
                rt.diag("FEEL-E361", 0, format!("host function '{key}' returned invalid FEEL-JSON: {e}"));
                NULL
            }
        },
        Err(m) => {
            rt.diag("FEEL-E362", 0, format!("host function '{key}' failed: {m}"));
            NULL
        }
    }
}

/// Binding key for `function(..) external {java: {class: c, method signature: m}}`.
pub fn external_key(rt: &Runtime, mapping: H) -> String {
    let Some(c) = rt.as_ctx(mapping) else { return String::new() };
    if let Some(j) = c.get("java").and_then(|h| rt.as_ctx(h)) {
        let class = j.get("class").and_then(|h| rt.as_str(h)).map(|s| s.to_string()).unwrap_or_default();
        let sig = j.get("method signature").and_then(|h| rt.as_str(h)).map(|s| s.to_string()).unwrap_or_default();
        let sig: String = sig.chars().filter(|c| !c.is_whitespace()).collect();
        return format!("java:{class}.{sig}");
    }
    if let Some(p) = c.get("pmml").and_then(|h| rt.as_ctx(h)) {
        let doc = p.get("document").and_then(|h| rt.as_str(h)).map(|s| s.to_string()).unwrap_or_default();
        let model = p.get("model").and_then(|h| rt.as_str(h)).map(|s| s.to_string()).unwrap_or_default();
        return format!("pmml:{doc}#{model}");
    }
    String::new()
}

fn trap_if_aborted(caller: &Caller<'_, HostState>) -> Result<(), wasmi::Error> {
    match &caller.data().rt.abort {
        Some(a) => Err(wasmi::Error::new(format!("{}: {}", a.code(), a.message()))),
        None => Ok(()),
    }
}

fn link(engine: &wasmi::Engine) -> Result<Linker<HostState>, wasmi::Error> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    macro_rules! bind {
        ($($name:ident($($p:ident),*) -> $r:tt;)*) => { $( bind!(@one $name ($($p),*) $r); )* };
        (@one $name:ident ($($p:ident),*) 1) => {
            linker.func_wrap(ABI_MODULE, concat!("f_", stringify!($name)),
                |mut caller: Caller<'_, HostState>, $($p: i32),*| -> Result<i32, wasmi::Error> {
                    let v = { let mut cx = NativeCx { caller: &mut caller }; abi::$name(&mut cx, $($p),*) };
                    trap_if_aborted(&caller)?;
                    Ok(v)
                }).map_err(|e| wasmi::Error::new(e.to_string()))?;
        };
        (@one $name:ident ($($p:ident),*) 0) => {
            linker.func_wrap(ABI_MODULE, concat!("f_", stringify!($name)),
                |mut caller: Caller<'_, HostState>, $($p: i32),*| -> Result<(), wasmi::Error> {
                    { let mut cx = NativeCx { caller: &mut caller }; let _ = abi::$name(&mut cx, $($p),*); }
                    trap_if_aborted(&caller)
                }).map_err(|e| wasmi::Error::new(e.to_string()))?;
        };
    }
    feel_runtime::feel_abi_list!(bind);
    Ok(linker)
}

impl Program {
    /// Create an isolated instance: its own runtime (value arena, limits,
    /// clock, host bindings) and its own module instance.
    pub fn instantiate(&self, engine: &Engine, opts: InstanceOptions) -> Result<Instance, HostError> {
        let mut rt = Runtime::new();
        rt.limits = opts.limits.clone();
        rt.extensions = self.meta.extension_flags;
        feel_runtime::load_constants(&mut rt, &self.consts).map_err(HostError::InvalidModule)?;
        let state = HostState { rt, apply: None, host: opts.host.clone() };
        let mut store = Store::new(&engine.engine, state);
        let linker = link(&engine.engine).map_err(|e| HostError::Link(e.to_string()))?;
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(|e| HostError::Link(e.to_string()))?;
        let evaluate = instance
            .get_typed_func::<(), i32>(&store, "evaluate")
            .map_err(|e| HostError::Incompatible(format!("missing evaluate export: {e}")))?;
        let apply = instance
            .get_typed_func::<(i32, i32), i32>(&store, "__feel_apply")
            .map_err(|e| HostError::Incompatible(format!("missing __feel_apply export: {e}")))?;
        let set_limits = instance
            .get_typed_func::<(i64, i32), ()>(&store, "__feel_set_limits")
            .map_err(|e| HostError::Incompatible(format!("missing __feel_set_limits export: {e}")))?;
        set_limits
            .call(&mut store, (opts.limits.max_steps as i64, opts.limits.max_depth as i32))
            .map_err(|e| HostError::Link(e.to_string()))?;
        store.data_mut().apply = Some(apply);
        Ok(Instance { store, evaluate, meta: self.meta.clone(), evaluations: 0 })
    }
}

pub struct Instance {
    store: Store<HostState>,
    evaluate: TypedFunc<(), i32>,
    pub meta: Metadata,
    pub evaluations: u64,
}

fn system_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let nanos = d.as_millis() as i128 * 1_000_000;
    match feel_runtime::temporal::DateTime::from_epoch_nanos(nanos, feel_runtime::temporal::Zone::utc()) {
        Some(dt) => dt.to_string(),
        None => "1970-01-01T00:00:00Z".to_string(),
    }
}

impl Instance {
    /// Evaluate with inputs given as a FEEL-JSON object.
    pub fn evaluate(&mut self, inputs_json: &str, opts: &EvalOptions) -> Evaluation {
        self.run(inputs_json, None, opts)
    }

    /// Evaluate a unary-tests module against `candidate_json` (`?`).
    pub fn evaluate_unary(&mut self, candidate_json: &str, inputs_json: &str, opts: &EvalOptions) -> Evaluation {
        self.run(inputs_json, Some(candidate_json), opts)
    }

    /// Evaluate many input sets without re-instantiating.
    pub fn evaluate_batch(&mut self, inputs: &[String], opts: &EvalOptions) -> Vec<Evaluation> {
        inputs.iter().map(|i| self.evaluate(i, opts)).collect()
    }

    /// Values currently held in the evaluation region (for leak tests).
    pub fn live_values(&self) -> u32 {
        self.store.data().rt.eval_values()
    }
    pub fn fixed_values(&self) -> u32 {
        self.store.data().rt.fixed
    }

    fn boundary(code: &str, message: String) -> Evaluation {
        Evaluation {
            result: None,
            diagnostics: Vec::new(),
            abort: Some(AbortInfo { kind: "boundary".into(), code: code.into(), message }),
        }
    }

    fn run(&mut self, inputs_json: &str, candidate: Option<&str>, opts: &EvalOptions) -> Evaluation {
        self.evaluations += 1;
        let names: Vec<String> = self.meta.inputs.iter().map(|s| s.name.clone()).collect();
        let types: Vec<Option<String>> = self.meta.inputs.iter().map(|s| s.ty.clone()).collect();
        let validate = self.meta.validate_inputs;
        {
            let rt = &mut self.store.data_mut().rt;
            rt.begin_eval();
            match &opts.clock {
                ClockSpec::None => rt.clock = None,
                ClockSpec::System => {
                    abi::set_clock(rt, &system_now());
                }
                ClockSpec::Fixed(s) => {
                    if !abi::set_clock(rt, s) {
                        return Self::boundary("FEEL-B504", format!("invalid clock instant '{s}'"));
                    }
                }
            }
            let inputs = match json::decode(rt, if inputs_json.trim().is_empty() { "{}" } else { inputs_json }) {
                Ok(h) => h,
                Err(e) => return Self::boundary("FEEL-B505", format!("invalid inputs: {e}")),
            };
            let cand = match candidate {
                Some(c) => match json::decode(rt, c) {
                    Ok(h) => Some(h),
                    Err(e) => return Self::boundary("FEEL-B505", format!("invalid candidate: {e}")),
                },
                None => None,
            };
            if let Err(m) = abi::bind_inputs(rt, inputs, cand, &names, &types, validate) {
                return Self::boundary("FEEL-B506", m);
            }
        }
        let r = self.evaluate.call(&mut self.store, ());
        let rt = &self.store.data().rt;
        let sites = &self.meta.sites;
        let diagnostics = rt
            .diags
            .iter()
            .map(|d| {
                let s = sites.get(d.site as usize);
                EvalDiagnostic {
                    code: d.code.to_string(),
                    message: d.message.clone(),
                    start: s.map(|s| s.start).unwrap_or(0),
                    end: s.map(|s| s.end).unwrap_or(0),
                }
            })
            .collect();
        match r {
            Ok(h) => {
                if let Some(a) = &rt.abort {
                    return Evaluation { result: None, diagnostics, abort: Some(abort_info(a)) };
                }
                Evaluation { result: Some(json::encode(rt, h as u32)), diagnostics, abort: None }
            }
            Err(e) => {
                let abort = match &rt.abort {
                    Some(a) => abort_info(a),
                    None => AbortInfo { kind: "boundary".into(), code: "FEEL-B503".into(), message: format!("trap: {e}") },
                };
                Evaluation { result: None, diagnostics, abort: Some(abort) }
            }
        }
    }
}

fn abort_info(a: &feel_runtime::Abort) -> AbortInfo {
    AbortInfo {
        kind: match a {
            feel_runtime::Abort::Limit { .. } => "limit".into(),
            feel_runtime::Abort::Boundary { .. } => "boundary".into(),
        },
        code: a.code().to_string(),
        message: a.message().to_string(),
    }
}

/// Convenience: compile + instantiate + evaluate once.
pub fn eval_once(source: &str, inputs_json: &str, opts: &feel_compiler::CompileOptions) -> Result<Evaluation, String> {
    let out = feel_compiler::compile(source, opts);
    let Some(wasm) = out.wasm else {
        return Err(out.diagnostics.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"));
    };
    let engine = Engine::new();
    let p = engine.load(&wasm).map_err(|e| e.to_string())?;
    let mut i = p.instantiate(&engine, InstanceOptions::default()).map_err(|e| e.to_string())?;
    Ok(i.evaluate(inputs_json, &EvalOptions { clock: ClockSpec::Fixed("2024-01-01T12:00:00Z".into()) }))
}
