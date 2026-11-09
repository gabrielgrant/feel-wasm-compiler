//! DMN TCK test-case reader, runner and result comparison.
//!
//! Each `*-test-*.xml` file is run against its model: the model is lowered
//! to FEEL (`lower.rs`), compiled **once** to a Wasm module, instantiated
//! once, and evaluated per test case with that case's input values. Results
//! are compared with the expected values the way the reference TCK runners do
//! (numbers within 1e-8, temporal values by FEEL equality, strings exactly).

use crate::lower::{self, Lowerer, ModelSet};
use crate::model::{parse_definitions, Definitions};
use feel_compiler::ast::{Args, ExprKind, Name, Program};
use feel_compiler::diag::Span;
use feel_compiler::parser::name_tokens;
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use feel_runtime::{json, Runtime, Value, H};
use roxmltree::{Document, Node};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TestCase {
    pub id: String,
    pub invocable: Option<String>,
    pub kind: Option<String>,
    pub inputs: Vec<(String, String)>,
    pub results: Vec<ResultNode>,
}

#[derive(Clone, Debug)]
pub struct ResultNode {
    pub name: String,
    pub expected: String,
    pub error_result: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub dir: String,
    pub file: String,
    pub case: String,
    pub node: String,
    pub status: Status,
    pub expected: String,
    pub actual: String,
    pub note: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pass,
    Fail,
    Skip,
    OutOfScope,
}

fn local<'a>(n: &Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}

/// FEEL-JSON for a TCK `<value>`, `<list>` or component container.
pub fn value_json(n: &Node) -> String {
    // n is an inputNode / resultNode-expected / item / component
    if let Some(v) = n.children().find(|c| c.is_element() && local(c) == "value") {
        return scalar_json(&v);
    }
    if let Some(l) = n.children().find(|c| c.is_element() && local(c) == "list") {
        let items: Vec<String> = l
            .children()
            .filter(|c| c.is_element() && local(c) == "item")
            .map(|i| value_json(&i))
            .collect();
        return format!("[{}]", items.join(","));
    }
    let comps: Vec<String> = n
        .children()
        .filter(|c| c.is_element() && local(c) == "component")
        .map(|c| {
            let mut k = String::new();
            json::write_json_string(&mut k, c.attribute("name").unwrap_or(""));
            format!("{k}:{}", value_json(&c))
        })
        .collect();
    if comps.is_empty() {
        return "null".into();
    }
    // a context whose only key starts with '@' needs the escape form
    format!("{{{}}}", comps.join(","))
}

fn scalar_json(v: &Node) -> String {
    let nil = v
        .attributes()
        .any(|a| a.name() == "nil" && a.value() == "true");
    if nil {
        return "null".into();
    }
    let ty = v
        .attributes()
        .find(|a| a.name() == "type")
        .map(|a| a.value())
        .unwrap_or("");
    let ty = ty.rsplit(':').next().unwrap_or(ty);
    let text: String = v.text().unwrap_or("").to_string();
    // nested structure inside <value> (some TCK files nest components)
    if v.children().any(|c| c.is_element()) {
        return value_json(v);
    }
    let mut s = String::new();
    match ty {
        "decimal" | "double" | "integer" | "int" | "long" | "float" => {
            let t = text.trim();
            match t {
                "NaN" | "INF" | "-INF" | "Infinity" | "-Infinity" => "null".into(),
                _ => {
                    json::write_json_string(&mut s, t);
                    format!("{{\"@number\":{s}}}")
                }
            }
        }
        "boolean" => text.trim().to_string(),
        "date" => {
            json::write_json_string(&mut s, text.trim());
            format!("{{\"@date\":{s}}}")
        }
        "dateTime" => {
            json::write_json_string(&mut s, text.trim());
            format!("{{\"@dateTime\":{s}}}")
        }
        "time" => {
            json::write_json_string(&mut s, text.trim());
            format!("{{\"@time\":{s}}}")
        }
        "duration" | "dayTimeDuration" | "yearMonthDuration" => {
            json::write_json_string(&mut s, text.trim());
            format!("{{\"@duration\":{s}}}")
        }
        _ => {
            json::write_json_string(&mut s, &text);
            s
        }
    }
}

pub fn parse_tests(xml: &str) -> Result<(String, Vec<TestCase>), String> {
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let model = root
        .children()
        .find(|c| c.is_element() && local(c) == "modelName")
        .and_then(|c| c.text())
        .unwrap_or("")
        .to_string();
    let mut cases = Vec::new();
    for tc in root
        .children()
        .filter(|c| c.is_element() && local(c) == "testCase")
    {
        let mut c = TestCase {
            id: tc.attribute("id").unwrap_or("").to_string(),
            invocable: tc.attribute("invocableName").map(|s| s.to_string()),
            kind: tc.attribute("type").map(|s| s.to_string()),
            inputs: Vec::new(),
            results: Vec::new(),
        };
        for n in tc.children().filter(|c| c.is_element()) {
            match local(&n) {
                "inputNode" => {
                    let name = n.attribute("name").unwrap_or("").to_string();
                    let key = match n.attribute("namespace") {
                        Some(ns) => format!("{ns}::{name}"),
                        None => name,
                    };
                    c.inputs.push((key, value_json(&n)))
                }
                "resultNode" => {
                    let exp = n
                        .children()
                        .find(|x| x.is_element() && local(x) == "expected");
                    c.results.push(ResultNode {
                        name: n.attribute("name").unwrap_or("").to_string(),
                        expected: exp.map(|e| value_json(&e)).unwrap_or_else(|| "null".into()),
                        error_result: n.attribute("errorResult") == Some("true"),
                    });
                }
                _ => {}
            }
        }
        cases.push(c);
    }
    Ok((model, cases))
}

/// Compare an actual FEEL-JSON result with an expected FEEL-JSON value.
pub fn values_match(expected: &str, actual: &str) -> bool {
    let mut rt = Runtime::new();
    let (Ok(e), Ok(a)) = (
        json::decode(&mut rt, expected),
        json::decode(&mut rt, actual),
    ) else {
        return expected == actual;
    };
    deep_match(&rt, e, a)
}

fn deep_match(rt: &Runtime, e: H, a: H) -> bool {
    match (rt.get(e), rt.get(a)) {
        (Value::Null, Value::Null) => true,
        (Value::Num(x), Value::Num(y)) => {
            if x == y {
                return true;
            }
            match x.sub(y) {
                Some(d) => d.abs() <= feel_runtime::Decimal::parse("0.00000001").unwrap(),
                None => false,
            }
        }
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| deep_match(rt, *p, *q))
        }
        (Value::Ctx(x), Value::Ctx(y)) => {
            x.len() == y.len()
                && x.keys
                    .iter()
                    .zip(x.vals.iter())
                    .all(|(k, v)| y.get(k).map(|w| deep_match(rt, *v, w)).unwrap_or(false))
        }
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Time(x), Value::Time(y)) => {
            feel_runtime::temporal::eq_time_exact(x, y).unwrap_or(false)
                || x.to_string() == y.to_string()
        }
        (Value::DateTime(x), Value::DateTime(y)) => {
            x.to_string() == y.to_string()
                || (x.zone.is_some() == y.zone.is_some()
                    && feel_runtime::temporal::eq_datetime_exact(x, y).unwrap_or(false))
        }
        _ => feel_runtime::ops::eq(rt, e, a) == Some(true),
    }
}

pub struct ModelRun {
    pub instance: feel_host::Instance,
    pub lower_errors: Vec<(String, String)>,
    pub unsupported: Vec<String>,
}

fn load_model_set(dir: &Path, model_file: &str) -> Result<ModelSet, String> {
    let main_path = dir.join(model_file);
    let xml =
        std::fs::read_to_string(&main_path).map_err(|e| format!("{}: {e}", main_path.display()))?;
    let main = parse_definitions(&xml)?;
    // index all models in the directory by namespace
    let mut others: Vec<Definitions> = Vec::new();
    if !main.imports.is_empty() {
        for f in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
            let p = f.path();
            if p.extension().map(|e| e == "dmn").unwrap_or(false) && p != main_path {
                if let Ok(x) = std::fs::read_to_string(&p) {
                    if let Ok(d) = parse_definitions(&x) {
                        others.push(d);
                    }
                }
            }
        }
    }
    // transitively imported models
    let mut all: Vec<Definitions> = Vec::new();
    let mut queue: Vec<String> = main.imports.iter().map(|i| i.namespace.clone()).collect();
    while let Some(ns) = queue.pop() {
        if ns == main.namespace || all.iter().any(|d| d.namespace == ns) {
            continue;
        }
        if let Some(d) = others.iter().find(|d| d.namespace == ns) {
            queue.extend(d.imports.iter().map(|i| i.namespace.clone()));
            all.push(d.clone());
        }
    }
    let imports = main
        .imports
        .iter()
        .filter_map(|im| {
            all.iter()
                .find(|d| d.namespace == im.namespace)
                .map(|d| (im.name.clone(), d.clone()))
        })
        .collect();
    Ok(ModelSet { main, imports, all })
}

/// Build the program for a test file: the model context, and for invocable
/// test cases a call of the invocable with named arguments from `__args`.
fn build_program<'a>(set: &'a ModelSet, invocable: Option<&str>) -> (Program, Lowerer<'a>) {
    let mut l = Lowerer::new(set);
    let model = l.lower_model();
    let prog = match invocable {
        None => model,
        Some(inv) => {
            // parameters of the invocable
            let params: Vec<String> = invocable_params(set, inv);
            let args = params
                .iter()
                .map(|p| {
                    (
                        Name {
                            text: p.clone(),
                            tokens: name_tokens(p),
                        },
                        Span::default(),
                        lower::path(lower::name("__args"), p),
                    )
                })
                .collect();
            let call = lower::mk(ExprKind::Call(
                Box::new(lower::path(lower::name("__model"), inv)),
                Args::Named(args),
            ));
            lower::path(
                lower::ctx(vec![("__model".into(), model), ("__invoke".into(), call)]),
                "__invoke",
            )
        }
    };
    (Program::Expression(prog), l)
}

fn invocable_params(set: &ModelSet, inv: &str) -> Vec<String> {
    let m = &set.main;
    if let Some(b) = m.bkms.iter().find(|b| b.name == inv) {
        return b
            .logic
            .as_ref()
            .map(|f| f.params.iter().map(|p| p.0.clone()).collect())
            .unwrap_or_default();
    }
    if let Some(s) = m.services.iter().find(|s| s.name == inv) {
        let find = |id: &String| {
            m.decisions
                .iter()
                .find(|d| &d.id == id)
                .map(|d| d.name.clone())
                .or_else(|| {
                    m.inputs
                        .iter()
                        .find(|i| &i.id == id)
                        .map(|i| i.name.clone())
                })
        };
        return s
            .input_data
            .iter()
            .chain(s.input_decisions.iter())
            .filter_map(find)
            .collect();
    }
    Vec::new()
}

pub struct FileReport {
    pub results: Vec<CaseResult>,
    pub compile_ms: f64,
    pub module_bytes: usize,
}

const CLOCK: &str = "2024-06-15T12:00:00Z";

/// Runner switches.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    /// emit unoptimized modules (differential runs)
    pub no_opt: bool,
}

pub fn run_test_file(dir: &Path, test_file: &Path, engine: &Engine) -> FileReport {
    run_test_file_with(dir, test_file, engine, RunOptions::default())
}

pub fn run_test_file_with(
    dir: &Path,
    test_file: &Path,
    engine: &Engine,
    ro: RunOptions,
) -> FileReport {
    let dname = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let fname = test_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut results = Vec::new();
    let mk = |case: &str, node: &str, status: Status, expected: &str, actual: &str, note: &str| {
        CaseResult {
            dir: dname.clone(),
            file: fname.clone(),
            case: case.to_string(),
            node: node.to_string(),
            status,
            expected: expected.to_string(),
            actual: actual.to_string(),
            note: note.to_string(),
        }
    };
    let xml = match std::fs::read_to_string(test_file) {
        Ok(x) => x,
        Err(e) => {
            results.push(mk(
                "*",
                "*",
                Status::Fail,
                "",
                "",
                &format!("cannot read test file: {e}"),
            ));
            return FileReport {
                results,
                compile_ms: 0.0,
                module_bytes: 0,
            };
        }
    };
    let (model_file, cases) = match parse_tests(&xml) {
        Ok(x) => x,
        Err(e) => {
            results.push(mk(
                "*",
                "*",
                Status::Fail,
                "",
                "",
                &format!("cannot parse test file: {e}"),
            ));
            return FileReport {
                results,
                compile_ms: 0.0,
                module_bytes: 0,
            };
        }
    };
    let set = match load_model_set(dir, &model_file) {
        Ok(s) => s,
        Err(e) => {
            for c in &cases {
                for r in &c.results {
                    results.push(mk(
                        &c.id,
                        &r.name,
                        Status::Fail,
                        &r.expected,
                        "",
                        &format!("model error: {e}"),
                    ));
                }
            }
            return FileReport {
                results,
                compile_ms: 0.0,
                module_bytes: 0,
            };
        }
    };
    // one compiled module per distinct invocable (None = whole model)
    let mut compiled: Vec<(Option<String>, Result<ModelRun, String>)> = Vec::new();
    let mut compile_ms = 0.0;
    let mut module_bytes = 0;
    for c in &cases {
        let key = c.invocable.clone();
        if !compiled.iter().any(|(k, _)| *k == key) {
            let t0 = std::time::Instant::now();
            let (prog, l) = build_program(&set, key.as_deref());
            if std::env::var("FEEL_TCK_PRINT").is_ok() {
                if let Program::Expression(e) = &prog {
                    eprintln!(
                        "lowered program for {fname}:\n{e}\nerrors: {:?}",
                        l.report.errors
                    );
                }
            }
            let mut opts = l.options(&[], feel_compiler::Entry::Expression);
            opts.optimize = !ro.no_opt;
            let errors = l.report.errors.clone();
            let unsupported = l.report.unsupported.clone();
            let out = feel_compiler::compile_program(&prog, "", Vec::new(), &opts);
            let run = match out.wasm {
                Some(w) => {
                    module_bytes += w.len();
                    engine
                        .load(&w)
                        .and_then(|p| {
                            let io = InstanceOptions {
                                host: feel_host::bindings::test_bindings(),
                                ..Default::default()
                            };
                            p.instantiate(engine, io)
                        })
                        .map(|instance| ModelRun {
                            instance,
                            lower_errors: errors,
                            unsupported,
                        })
                        .map_err(|e| e.to_string())
                }
                None => Err(out
                    .diagnostics
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")),
            };
            compile_ms += t0.elapsed().as_secs_f64() * 1000.0;
            compiled.push((key.clone(), run));
        }
    }
    for c in &cases {
        let run = compiled
            .iter_mut()
            .find(|(k, _)| *k == c.invocable)
            .map(|(_, r)| r)
            .unwrap();
        let run = match run {
            Ok(r) => r,
            Err(e) => {
                for r in &c.results {
                    results.push(mk(
                        &c.id,
                        &r.name,
                        Status::Fail,
                        &r.expected,
                        "",
                        &format!("compile error: {e}"),
                    ));
                }
                continue;
            }
        };
        let main_ns = format!("{}::", set.main.namespace);
        let mut inputs = String::from("{");
        for (i, (k, v)) in c.inputs.iter().enumerate() {
            let k = k.strip_prefix(&main_ns).unwrap_or(k);
            if i > 0 {
                inputs.push(',');
            }
            json::write_json_string(&mut inputs, k);
            inputs.push(':');
            inputs.push_str(v);
        }
        inputs.push('}');
        let inputs = if c.invocable.is_some() {
            format!("{{\"__args\":{inputs}}}")
        } else {
            inputs
        };
        let ev = run.instance.evaluate(
            &inputs,
            &EvalOptions {
                clock: ClockSpec::Fixed(CLOCK.into()),
            },
        );
        if std::env::var("FEEL_TCK_DUMP").is_ok() {
            eprintln!(
                "case {} inputs {inputs}\n  result {:?}\n  abort {:?}",
                c.id, ev.result, ev.abort
            );
        }
        for r in &c.results {
            let actual = match (&ev.result, &ev.abort) {
                (Some(v), _) => {
                    if let Some(inv) = &c.invocable {
                        // a result node naming an output decision of a
                        // multi-output decision service selects that entry
                        let sel = select_entry(v, &r.name);
                        if &r.name != inv && !sel.starts_with('<') {
                            sel
                        } else {
                            v.clone()
                        }
                    } else {
                        select_entry(v, &r.name)
                    }
                }
                (None, Some(a)) => format!("<abort {}: {}>", a.code, a.message),
                _ => "<none>".into(),
            };
            let pass = values_match(&r.expected, &actual);
            let mut note = String::new();
            if !pass {
                let errs: Vec<String> = run
                    .lower_errors
                    .iter()
                    .filter(|(w, _)| w.contains(&format!("'{}'", r.name)))
                    .map(|(w, m)| format!("{w}: {m}"))
                    .collect();
                if !errs.is_empty() {
                    note = errs.join(" | ");
                } else if !run.unsupported.is_empty() {
                    note = format!("unsupported: {}", run.unsupported.join(", "));
                }
                let ds: Vec<String> = ev
                    .diagnostics
                    .iter()
                    .take(3)
                    .map(|d| format!("{} {}", d.code, d.message))
                    .collect();
                if !ds.is_empty() {
                    if !note.is_empty() {
                        note.push_str(" | ");
                    }
                    note.push_str(&ds.join("; "));
                }
            }
            results.push(mk(
                &c.id,
                &r.name,
                if pass { Status::Pass } else { Status::Fail },
                &r.expected,
                &actual,
                &note,
            ));
        }
    }
    FileReport {
        results,
        compile_ms,
        module_bytes,
    }
}

fn select_entry(ctx_json: &str, key: &str) -> String {
    let mut rt = Runtime::new();
    match json::decode(&mut rt, ctx_json) {
        Ok(h) => match rt.as_ctx(h).and_then(|c| c.get(key)) {
            Some(v) => json::encode(&rt, v),
            None => "<missing>".into(),
        },
        Err(_) => "<invalid>".into(),
    }
}

/// Find `(dir, test file)` pairs under a TCK `TestCases` root.
pub fn discover(root: &Path, levels: &[&str]) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for lvl in levels {
        let base = root.join(lvl);
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .map(|n| {
                            n.to_string_lossy().contains("-test-")
                                && n.to_string_lossy().ends_with(".xml")
                        })
                        .unwrap_or(false)
                })
                .collect();
            files.sort();
            for f in files {
                out.push((d.clone(), f));
            }
        }
    }
    out
}

/// Differential check of decision tables: every top-level decision whose
/// logic is a decision table referencing only input data is evaluated by the
/// host-orchestrated evaluator (one unary-test module per cell) and compared
/// with the lowered single-module result. Returns (compared, mismatches).
pub fn host_dt_differential(dir: &Path, test_file: &Path, engine: &Engine) -> (usize, Vec<String>) {
    use crate::host_dt::HostDecisionTable;
    use crate::model::Boxed;
    let mut compared = 0;
    let mut mism = Vec::new();
    let Ok(xml) = std::fs::read_to_string(test_file) else {
        return (0, mism);
    };
    let Ok((model_file, cases)) = parse_tests(&xml) else {
        return (0, mism);
    };
    let Ok(set) = load_model_set(dir, &model_file) else {
        return (0, mism);
    };
    let input_names: Vec<String> = set.main.inputs.iter().map(|i| i.name.clone()).collect();
    let rep = run_test_file(dir, test_file, engine);
    for d in &set.main.decisions {
        let Some(Boxed::DecisionTable(dt)) = &d.expr else {
            continue;
        };
        if !d.required_decisions.is_empty() || !d.required_knowledge.is_empty() {
            continue;
        }
        let Ok(mut h) = HostDecisionTable::compile(engine, dt, &input_names) else {
            continue;
        };
        for c in cases.iter().filter(|c| c.invocable.is_none()) {
            let Some(lowered) = rep
                .results
                .iter()
                .find(|r| r.case == c.id && r.node == d.name)
            else {
                continue;
            };
            let mut inputs = String::from("{");
            for (i, (k, v)) in c.inputs.iter().enumerate() {
                if i > 0 {
                    inputs.push(',');
                }
                json::write_json_string(&mut inputs, k);
                inputs.push(':');
                inputs.push_str(v);
            }
            inputs.push('}');
            let host = h
                .evaluate(&inputs)
                .unwrap_or_else(|e| format!("<error {e}>"));
            compared += 1;
            if !values_match(&lowered.actual, &host) {
                mism.push(format!(
                    "{} {} {}: lowered {} host {}",
                    lowered.dir, c.id, d.name, lowered.actual, host
                ));
            }
        }
    }
    (compared, mism)
}
