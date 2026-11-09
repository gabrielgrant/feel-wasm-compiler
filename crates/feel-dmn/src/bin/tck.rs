//! `feel-tck` — run the DMN TCK against the generated Wasm backend.
//!
//! ```text
//! feel-tck <TCK TestCases dir> [--json out.json] [--md out.md] [--filter substr] [--verbose]
//! ```

use feel_dmn::tck::{
    discover, host_dt_differential, run_test_file_with, CaseResult, RunOptions, Status,
};
use feel_host::Engine;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: feel-tck <TestCases dir> [--json out.json] [--md out.md] [--filter s] [--verbose]");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[1]);
    let mut json_out = None;
    let mut md_out = None;
    let mut filter = None;
    let mut verbose = false;
    let mut ro = RunOptions::default();
    let mut host_dt = false;
    let mut levels: Vec<&str> = vec!["compliance-level-2", "compliance-level-3"];
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json_out = args.get(i + 1).cloned();
                i += 1
            }
            "--md" => {
                md_out = args.get(i + 1).cloned();
                i += 1
            }
            "--filter" => {
                filter = args.get(i + 1).cloned();
                i += 1
            }
            "--verbose" | "-v" => verbose = true,
            "--no-opt" => ro.no_opt = true,
            "--host-dt" => host_dt = true,
            // the TCK's own "non-compliant" folder (informational only)
            "--non-compliant" => levels = vec!["non-compliant"],
            _ => {}
        }
        i += 1;
    }
    let engine = Engine::new();
    let files = discover(&root, &levels);
    if host_dt {
        let (mut n, mut bad) = (0, Vec::new());
        for (dir, f) in &files {
            let (c, m) = host_dt_differential(dir, f, &engine);
            n += c;
            bad.extend(m);
        }
        for b in &bad {
            println!("MISMATCH {b}");
        }
        println!(
            "host-orchestrated decision tables: {n} results compared, {} mismatches",
            bad.len()
        );
        std::process::exit(if bad.is_empty() { 0 } else { 1 });
    }
    let mut all: Vec<CaseResult> = Vec::new();
    let t0 = std::time::Instant::now();
    let mut compile_ms = 0.0;
    let mut bytes = 0usize;
    for (dir, f) in files {
        if let Some(fl) = &filter {
            if !dir.to_string_lossy().contains(fl.as_str()) {
                continue;
            }
        }
        let rep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_test_file_with(&dir, &f, &engine, ro)
        }));
        match rep {
            Ok(r) => {
                compile_ms += r.compile_ms;
                bytes += r.module_bytes;
                all.extend(r.results)
            }
            Err(_) => all.push(CaseResult {
                dir: dir.file_name().unwrap().to_string_lossy().to_string(),
                file: f.file_name().unwrap().to_string_lossy().to_string(),
                case: "*".into(),
                node: "*".into(),
                status: Status::Fail,
                expected: String::new(),
                actual: String::new(),
                note: "runner panic".into(),
            }),
        }
    }
    let elapsed = t0.elapsed().as_secs_f64();
    let count = |s: Status| all.iter().filter(|r| r.status == s).count();
    let (p, f, s, o) = (
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Skip),
        count(Status::OutOfScope),
    );
    let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &all {
        let e = by_dir.entry(r.dir.clone()).or_default();
        if r.status == Status::Pass {
            e.0 += 1;
        } else {
            e.1 += 1;
        }
    }
    for r in all.iter().filter(|r| r.status == Status::Fail) {
        if verbose {
            println!(
                "FAIL {}/{} {} [{}]\n   expected {}\n   actual   {}\n   {}",
                r.dir, r.file, r.case, r.node, r.expected, r.actual, r.note
            );
        }
    }
    println!(
        "TCK result nodes: {} total, {} pass, {} fail, {} skip, {} out of scope ({:.1}% pass) in {:.1}s (compile {:.0} ms, {} KiB of Wasm)",
        all.len(),
        p,
        f,
        s,
        o,
        100.0 * p as f64 / all.len().max(1) as f64,
        elapsed,
        compile_ms,
        bytes / 1024
    );
    if verbose {
        for (d, (pp, ff)) in &by_dir {
            if *ff > 0 {
                println!("  {d}: {pp} pass, {ff} fail");
            }
        }
    }
    if let Some(j) = json_out {
        let v = serde_json::json!({
            "summary": {"total": all.len(), "pass": p, "fail": f, "skip": s, "outOfScope": o},
            "results": all,
        });
        std::fs::write(&j, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }
    if let Some(m) = md_out {
        let mut s = String::new();
        s.push_str("| directory | pass | fail |\n|---|---:|---:|\n");
        for (d, (pp, ff)) in &by_dir {
            s.push_str(&format!("| {d} | {pp} | {ff} |\n"));
        }
        std::fs::write(&m, s).unwrap();
    }
}
