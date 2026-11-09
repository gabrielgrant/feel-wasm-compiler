//! `cargo run --release -p feel-host --example bench` — native timings:
//! compile (parse+check+lower+optimize+emit), load+instantiate, and warm
//! evaluation through wasmi with the natively bound runtime.
use feel_compiler::{compile, CompileOptions, Entry};
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use std::time::Instant;

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
    (p(0.5), p(0.9), p(0.99))
}

fn main() {
    let items: String = (0..100)
        .map(|i| {
            format!(
                r#"{{"name":"i{i}","price":{}.{:02},"qty":{}}}"#,
                i % 37,
                i % 100,
                i % 7 + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let xs: String = (0..100)
        .map(|i| ((i * 7919) % 1000).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cases: Vec<(&str, &str, String, Option<&str>)> = vec![
        (
            "predicate",
            r#"age >= 18 and income > 50000 and status in ("A","B")"#,
            r#"{"age":30,"income":72000,"status":"B"}"#.into(),
            None,
        ),
        (
            "loan",
            "(principal * rate / 12) / (1 - (1 + rate / 12) ** -term)",
            r#"{"principal":250000,"rate":0.045,"term":360}"#.into(),
            None,
        ),
        (
            "sum-decimal",
            "sum(for x in items return x.price * x.qty)",
            format!(r#"{{"items":[{items}]}}"#),
            None,
        ),
        (
            "filter-project",
            "items[price > 10].name",
            format!(r#"{{"items":[{items}]}}"#),
            None,
        ),
        (
            "map-double",
            "for x in xs return x * 2",
            format!(r#"{{"xs":[{xs}]}}"#),
            None,
        ),
        (
            "sort-lambda",
            "sort(xs, function(a,b) a < b)",
            format!(r#"{{"xs":[{xs}]}}"#),
            None,
        ),
        ("ut-range", "[18..65)", "{}".into(), Some("42")),
        ("ut-strings", r#""A","B","C""#, "{}".into(), Some(r#""C""#)),
    ];
    let engine = Engine::new();
    println!("| workload | compile median µs | instantiate median µs | eval median µs | eval p90 µs | eval p99 µs | module bytes |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for (name, src, inputs, cand) in cases {
        let mut o = CompileOptions::default();
        if cand.is_some() {
            o.entry = Entry::UnaryTests;
        }
        let mut ct = Vec::new();
        let mut wasm = Vec::new();
        for _ in 0..300 {
            let t = Instant::now();
            let out = compile(src, &o);
            ct.push(t.elapsed().as_secs_f64() * 1e6);
            wasm = out.wasm.unwrap();
        }
        let mut it = Vec::new();
        for _ in 0..200 {
            let t = Instant::now();
            let p = engine.load(&wasm).unwrap();
            let _i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
            it.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let p = engine.load(&wasm).unwrap();
        let mut inst = p.instantiate(&engine, InstanceOptions::default()).unwrap();
        let eo = EvalOptions {
            clock: ClockSpec::None,
        };
        let run = |inst: &mut feel_host::Instance| match cand {
            Some(c) => inst.evaluate_unary(c, &inputs, &eo),
            None => inst.evaluate(&inputs, &eo),
        };
        for _ in 0..200 {
            run(&mut inst);
        }
        let mut et = Vec::new();
        for _ in 0..2000 {
            let t = Instant::now();
            let r = run(&mut inst);
            et.push(t.elapsed().as_secs_f64() * 1e6);
            assert!(r.result.is_some());
        }
        let (c50, _, _) = stats(ct);
        let (i50, _, _) = stats(it);
        let (e50, e90, e99) = stats(et);
        println!(
            "| `{name}` | {c50:.1} | {i50:.1} | {e50:.2} | {e90:.2} | {e99:.2} | {} |",
            wasm.len()
        );
    }
}
