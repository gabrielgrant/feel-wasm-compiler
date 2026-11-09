//! Emitted-module structure, isolation, cleanup and ABI-compatibility tests.

use feel_compiler::{compile, CompileOptions};
use feel_host::inspect::ModuleSummary;
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use feel_runtime::abi::{ABI_FUNCS, ABI_MODULE};

fn wasm(src: &str) -> Vec<u8> {
    compile(src, &CompileOptions::default()).wasm.expect("compiles")
}

fn eo() -> EvalOptions {
    EvalOptions { clock: ClockSpec::Fixed("2024-01-01T00:00:00Z".into()) }
}

#[test]
fn modules_validate_and_import_only_primitive_helpers() {
    for src in ["1 + 2", "for x in xs return x * 2", "{f: function(a) a + 1, r: f(1)}.r", "items[price > 10].name", "date(\"2020-01-01\") + @\"P1D\""] {
        let w = wasm(src);
        feel_compiler::validate_module(&w).unwrap();
        let s = ModuleSummary::of(&w).unwrap();
        assert!(!s.has_memory, "{src}: generated modules have no linear memory");
        for imp in &s.imports {
            let (m, n) = imp.split_once('.').unwrap();
            assert_eq!(m, ABI_MODULE, "{src}: unexpected import module {m}");
            let name = n.strip_prefix("f_").unwrap();
            assert!(ABI_FUNCS.iter().any(|f| f.0 == name), "{src}: unknown import {n}");
            // no import can evaluate FEEL source, an AST or bytecode
            assert!(!["eval", "interpret", "run", "parse", "compile"].iter().any(|x| name.contains(x)), "{src}: {name}");
        }
        // the runtime ABI itself has no interpreter entry point
        assert!(ABI_FUNCS.iter().all(|f| !f.0.contains("eval") && !f.0.contains("interp")));
    }
}

#[test]
fn code_is_specialized_to_the_expression() {
    let a = ModuleSummary::of(&wasm("a + b")).unwrap();
    let b = ModuleSummary::of(&wasm("if a > b then a * 2 else for i in 1..b return i + a")).unwrap();
    let c = ModuleSummary::of(&wasm("a > b")).unwrap();
    // `evaluate` (the first defined function) differs per expression
    assert_ne!(a.code_sizes[0], b.code_sizes[0]);
    assert_ne!(a.code_sizes[0], c.code_sizes[0]);
    // imports reflect the operations used
    assert!(a.imports.iter().any(|i| i.ends_with("f_add")) && !a.imports.iter().any(|i| i.ends_with("f_gt")));
    assert!(c.imports.iter().any(|i| i.ends_with("f_gt")) && !c.imports.iter().any(|i| i.ends_with("f_add")));
    // boolean logic is lowered to Wasm control flow (no runtime call for `and`)
    let d = ModuleSummary::of(&wasm("x and y or not(z)")).unwrap();
    assert!(!d.imports.iter().any(|i| i.contains("and") || i.contains("or")), "{:?}", d.imports);
}

#[test]
fn repeated_evaluation_does_not_accumulate_values() {
    let engine = Engine::new();
    let p = engine.load(&wasm("for x in xs return {v: x * 2, s: string(x)}")).unwrap();
    let mut inst = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let inputs = r#"{"xs": [1,2,3,4,5,6,7,8,9,10]}"#;
    let first = inst.evaluate(inputs, &eo());
    let live_after_first = inst.live_values();
    for _ in 0..5000 {
        let r = inst.evaluate(inputs, &eo());
        assert_eq!(r.result, first.result);
    }
    assert_eq!(inst.live_values(), live_after_first, "evaluation region is released between evaluations");
    assert!(live_after_first < 200);
}

#[test]
fn instances_are_isolated() {
    let engine = Engine::new();
    let p = engine.load(&wasm("count(xs) + n")).unwrap();
    let mut a = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let mut b = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let ra = a.evaluate(r#"{"xs":[1,2,3],"n":1}"#, &eo());
    let rb = b.evaluate(r#"{"xs":[],"n":100}"#, &eo());
    let ra2 = a.evaluate(r#"{"xs":[1,2,3],"n":1}"#, &eo());
    assert_eq!(ra.result.as_deref(), Some("4"));
    assert_eq!(rb.result.as_deref(), Some("100"));
    assert_eq!(ra2.result, ra.result);
    // an aborted evaluation in one instance does not affect another or the next call
    let q = engine.load(&wasm("count(for i in 1..n return i)")).unwrap();
    let mut io = InstanceOptions::default();
    io.limits.max_steps = 100;
    let mut c = q.instantiate(&engine, io).unwrap();
    let r1 = c.evaluate(r#"{"n": 1000}"#, &eo());
    assert_eq!(r1.abort.as_ref().map(|a| a.code.as_str()), Some("FEEL-L404"));
    assert_eq!(r1.result, None, "a limit failure is not a FEEL null");
    let r2 = c.evaluate(r#"{"n": 10}"#, &eo());
    assert_eq!(r2.result.as_deref(), Some("10"));
    assert_eq!(a.evaluate(r#"{"xs":[1],"n":1}"#, &eo()).result.as_deref(), Some("2"));
}

#[test]
fn incompatible_modules_are_rejected_with_clear_errors() {
    let w = wasm("1 + 1");
    // tamper with the ABI version in the metadata section
    let text = String::from_utf8_lossy(&w).to_string();
    assert!(text.contains("\"abiVersion\":1"));
    let pos = w.windows(14).position(|x| x == b"\"abiVersion\":1").unwrap();
    let mut bad = w.clone();
    bad[pos + 13] = b'9';
    let err = Engine::new().load(&bad).err().unwrap().to_string();
    assert!(err.contains("incompatible module") && err.contains("feel_rt_v1"), "{err}");
    // not a FEEL module at all
    let plain = wasm_encoder::Module::new().finish();
    let err = Engine::new().load(&plain).err().unwrap().to_string();
    assert!(err.contains("feel.meta"), "{err}");
}

#[test]
fn boundary_errors_are_distinct_from_null() {
    let engine = Engine::new();
    let p = engine.load(&wasm("x")).unwrap();
    let mut i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let r = i.evaluate("{not json", &eo());
    assert_eq!(r.abort.unwrap().code, "FEEL-B505");
    let r = i.evaluate(r#"{"x": null}"#, &eo());
    assert_eq!(r.result.as_deref(), Some("null"));
    assert!(r.abort.is_none());
}

#[test]
fn batch_evaluation_matches_single() {
    let engine = Engine::new();
    let p = engine.load(&wasm("price * qty")).unwrap();
    let mut i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let inputs: Vec<String> = (1..=50).map(|k| format!(r#"{{"price": 0.{k:02}, "qty": {k}}}"#)).collect();
    let batch = i.evaluate_batch(&inputs, &eo());
    for (k, r) in batch.iter().enumerate() {
        let single = i.evaluate(&inputs[k], &eo());
        assert_eq!(r.result, single.result);
    }
    assert_eq!(batch[9].result.as_deref(), Some("1.00"));
}

#[test]
fn host_functions_are_explicit_capabilities() {
    let mut o = CompileOptions::default();
    o.schema = feel_compiler::types::Schema {
        functions: vec![feel_compiler::types::HostFunctionSpec {
            name: "lookup rate".into(),
            params: vec![feel_compiler::types::InputSpec { name: "grade".into(), ty: Some("string".into()) }],
            returns: Some("number".into()),
        }],
        ..Default::default()
    };
    let out = compile("amount * lookup rate(grade)", &o);
    let w = out.wasm.unwrap();
    assert_eq!(out.metadata.unwrap().dependencies.host_functions, vec!["lookup rate".to_string()]);
    let engine = Engine::new();
    let p = engine.load(&w).unwrap();
    let mut io = InstanceOptions::default();
    io.host.add("lookup rate", |args| Ok(if args[0] == "\"A\"" { "0.05".into() } else { "0.10".into() }));
    let mut i = p.instantiate(&engine, io).unwrap();
    assert_eq!(i.evaluate(r#"{"amount": 100, "grade": "A"}"#, &eo()).result.as_deref(), Some("5.00"));
    assert_eq!(i.evaluate(r#"{"amount": 100, "grade": "B"}"#, &eo()).result.as_deref(), Some("10.00"));
    // without a binding: null + diagnostic, not a crash
    let mut j = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let r = j.evaluate(r#"{"amount": 100, "grade": "A"}"#, &eo());
    assert_eq!(r.result.as_deref(), Some("null"));
    assert!(r.diagnostics.iter().any(|d| d.code == "FEEL-E360"));
}

#[test]
fn clock_is_an_explicit_capability() {
    let engine = Engine::new();
    let p = engine.load(&wasm("today()")).unwrap();
    let mut i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let r = i.evaluate("{}", &EvalOptions { clock: ClockSpec::None });
    assert_eq!(r.result.as_deref(), Some("null"));
    let r = i.evaluate("{}", &EvalOptions { clock: ClockSpec::Fixed("2030-05-06T07:08:09Z".into()) });
    assert_eq!(r.result.as_deref(), Some(r#"{"@date":"2030-05-06"}"#));
}
