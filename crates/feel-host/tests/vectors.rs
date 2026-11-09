//! Semantic vectors (`tests/vectors/semantic.json`) executed through
//! natively-hosted generated Wasm, with the optimizer on and off
//! (differential test: results and diagnostics codes must agree).

use feel_compiler::{compile, CompileOptions, Entry};
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use serde_json::Value;

#[derive(Debug)]
struct Outcome {
    result: Option<String>,
    abort: Option<String>,
    compile_error: Vec<String>,
    diags: Vec<String>,
}

fn run(v: &Value, optimize: bool) -> Outcome {
    let expr = v["expr"].as_str().unwrap();
    let mut opts = match v.get("options") {
        Some(o) => CompileOptions::from_json(&o.to_string()).unwrap(),
        None => CompileOptions::default(),
    };
    if v.get("entry").and_then(|e| e.as_str()) == Some("unaryTests") {
        opts.entry = Entry::UnaryTests;
    }
    opts.optimize = optimize;
    let out = compile(expr, &opts);
    let Some(wasm) = out.wasm else {
        return Outcome {
            result: None,
            abort: None,
            compile_error: out.diagnostics.iter().map(|d| d.code.clone()).collect(),
            diags: vec![],
        };
    };
    feel_compiler::validate_module(&wasm).expect("emitted module validates");
    let engine = Engine::new();
    let program = engine.load(&wasm).expect("load");
    let mut io = InstanceOptions::default();
    if let Some(l) = v.get("limits") {
        if let Some(x) = l.get("maxSteps").and_then(|x| x.as_u64()) {
            io.limits.max_steps = x;
        }
        if let Some(x) = l.get("maxDepth").and_then(|x| x.as_u64()) {
            io.limits.max_depth = x as u32;
        }
        if let Some(x) = l.get("maxValues").and_then(|x| x.as_u64()) {
            io.limits.max_values = x as u32;
        }
        if let Some(x) = l.get("maxCollection").and_then(|x| x.as_u64()) {
            io.limits.max_collection = x as u32;
        }
    }
    let mut inst = program.instantiate(&engine, io).expect("instantiate");
    let inputs = v
        .get("inputs")
        .map(|i| i.to_string())
        .unwrap_or_else(|| "{}".into());
    let clock = ClockSpec::Fixed(
        v.get("clock")
            .and_then(|c| c.as_str())
            .unwrap_or("2024-01-01T10:00:00Z")
            .to_string(),
    );
    let eo = EvalOptions { clock };
    let ev = if opts.entry == Entry::UnaryTests {
        let cand = v
            .get("candidate")
            .map(|c| c.to_string())
            .unwrap_or_else(|| "null".into());
        inst.evaluate_unary(&cand, &inputs, &eo)
    } else {
        inst.evaluate(&inputs, &eo)
    };
    Outcome {
        result: ev.result,
        abort: ev.abort.map(|a| a.code),
        compile_error: vec![],
        diags: ev.diagnostics.iter().map(|d| d.code.clone()).collect(),
    }
}

fn check(v: &Value, o: &Outcome) -> Result<(), String> {
    let id = v["id"].as_str().unwrap();
    if let Some(code) = v.get("expectCompileError").and_then(|c| c.as_str()) {
        if !o.compile_error.iter().any(|c| c == code) {
            return Err(format!("{id}: expected compile error {code}, got {o:?}"));
        }
        return Ok(());
    }
    if !o.compile_error.is_empty() {
        return Err(format!(
            "{id}: unexpected compile error {:?}",
            o.compile_error
        ));
    }
    if let Some(code) = v.get("expectAbort").and_then(|c| c.as_str()) {
        if o.abort.as_deref() != Some(code) {
            return Err(format!("{id}: expected abort {code}, got {o:?}"));
        }
        return Ok(());
    }
    let exp = v["expect"].as_str().unwrap();
    if o.result.as_deref() != Some(exp) {
        return Err(format!(
            "{id}: expected {exp}, got {:?} (abort {:?}, diags {:?})",
            o.result, o.abort, o.diags
        ));
    }
    if let Some(ds) = v.get("expectDiag").and_then(|d| d.as_array()) {
        for d in ds {
            let d = d.as_str().unwrap();
            if !o.diags.iter().any(|x| x == d) {
                return Err(format!("{id}: expected diagnostic {d}, got {:?}", o.diags));
            }
        }
    }
    Ok(())
}

#[test]
fn semantic_vectors() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/vectors/semantic.json"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let vs: Vec<Value> = serde_json::from_str(&text).unwrap();
    let mut failures = Vec::new();
    for v in &vs {
        let a = run(v, true);
        let b = run(v, false);
        if let Err(e) = check(v, &a) {
            failures.push(format!("[opt] {e}"));
        }
        if let Err(e) = check(v, &b) {
            failures.push(format!("[no-opt] {e}"));
        }
        if a.result != b.result || a.abort != b.abort || a.diags != b.diags {
            failures.push(format!(
                "[differential] {}: opt {a:?} vs no-opt {b:?}",
                v["id"]
            ));
        }
    }
    for f in &failures {
        eprintln!("{f}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} vector checks failed",
        failures.len(),
        vs.len()
    );
    eprintln!(
        "{} semantic vectors passed (optimized + unoptimized)",
        vs.len()
    );
}
