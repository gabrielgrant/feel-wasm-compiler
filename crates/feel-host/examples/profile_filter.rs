//! `cargo run --release -p feel-host --example profile_filter` — where the time
//! goes in a filter over 100 contexts: input decoding vs the compiled filter
//! loop vs projection (see docs/BENCHMARKS.md).
use feel_compiler::{compile, CompileOptions};
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use std::time::Instant;
fn med(src: &str, inputs: &str, n: usize) -> f64 {
    let engine = Engine::new();
    let p = engine
        .load(&compile(src, &CompileOptions::default()).wasm.unwrap())
        .unwrap();
    let mut i = p.instantiate(&engine, InstanceOptions::default()).unwrap();
    let eo = EvalOptions {
        clock: ClockSpec::None,
    };
    for _ in 0..300 {
        i.evaluate(inputs, &eo);
    }
    let mut v: Vec<f64> = (0..n)
        .map(|_| {
            let t = Instant::now();
            i.evaluate(inputs, &eo);
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[n / 2]
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
    let inp = format!(r#"{{"items":[{items}]}}"#);
    let nums: String = (0..100)
        .map(|i| format!("{}.{:02}", i % 37, i % 100))
        .collect::<Vec<_>>()
        .join(",");
    let inp_nums = format!(r#"{{"xs":[{nums}]}}"#);
    for (label, src, i) in [
        ("decode only: count(items)", "count(items)", &inp),
        (
            "filter only: count(items[price > 10])",
            "count(items[price > 10])",
            &inp,
        ),
        (
            "explicit item: count(items[item.price > 10])",
            "count(items[item.price > 10])",
            &inp,
        ),
        (
            "filter+project: items[price > 10].name",
            "items[price > 10].name",
            &inp,
        ),
        ("numbers decode only: count(xs)", "count(xs)", &inp_nums),
        (
            "numbers filter: count(xs[item > 10])",
            "count(xs[item > 10])",
            &inp_nums,
        ),
        ("constant: 1", "1", &"{}".to_string()),
    ] {
        println!("{:52} {:8.2} µs", label, med(src, i, 3000));
    }
}
