//! `feel` — command-line interface to the FEEL → Wasm compiler.
//!
//! ```text
//! feel parse   [-e EXPR | FILE] [--unary-tests]            print the AST (JSON)
//! feel check   [-e EXPR | FILE] [--schema S] [--strict]    type-check, print diagnostics + type
//! feel compile [-e EXPR | FILE] -o out.wasm [--no-opt]     emit a Wasm module (+ .meta.json)
//! feel run     [-e EXPR | FILE] [--inputs JSON]            compile and execute the emitted Wasm
//! feel exec    MODULE.wasm [--inputs JSON] [--batch FILE]  execute a previously emitted module
//! feel inspect MODULE.wasm                                 print metadata, imports, exports
//! feel deps    [-e EXPR | FILE]                            static dependency report
//! ```

use clap::{Args, Parser, Subcommand};
use feel_compiler::check::CheckMode;
use feel_compiler::types::Schema;
use feel_compiler::{CompileOptions, Entry};
use feel_host::{ClockSpec, Engine, EvalOptions, InstanceOptions};
use std::io::Read;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "feel",
    version,
    about = "FEEL (DMN 1.5) to WebAssembly compiler"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Src {
    /// FEEL source text
    #[arg(short = 'e', long = "expr")]
    expr: Option<String>,
    /// File containing FEEL source ("-" for stdin)
    file: Option<String>,
    /// Treat the source as unary tests (candidate `?`)
    #[arg(long)]
    unary_tests: bool,
    /// Schema JSON (inline or @file)
    #[arg(long)]
    schema: Option<String>,
    /// Strict checking (checker findings are errors)
    #[arg(long)]
    strict: bool,
    /// Disable the checker
    #[arg(long)]
    no_check: bool,
    /// Disable optimizations
    #[arg(long)]
    no_opt: bool,
    /// Enable an extension (e.g. "has key")
    /// Dialect: feel-1.5 (default) or b-feel
    #[arg(long, default_value = "feel-1.5")]
    dialect: String,
    #[arg(long = "ext")]
    extensions: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
    Parse(Src),
    Check(Src),
    Compile {
        #[command(flatten)]
        src: Src,
        #[arg(short = 'o', long)]
        out: String,
        /// Print the WebAssembly text format (WAT-like dump) to stdout
        #[arg(long)]
        dump: bool,
    },
    Run {
        #[command(flatten)]
        src: Src,
        /// Inputs as FEEL-JSON (inline or @file)
        #[arg(long, default_value = "{}")]
        inputs: String,
        /// Candidate for unary tests (FEEL-JSON)
        #[arg(long)]
        candidate: Option<String>,
        /// Fixed clock instant for now()/today()
        #[arg(long)]
        clock: Option<String>,
        #[arg(long)]
        max_steps: Option<u64>,
    },
    Exec {
        module: String,
        #[arg(long, default_value = "{}")]
        inputs: String,
        /// File with one FEEL-JSON input object per line
        #[arg(long)]
        batch: Option<String>,
        #[arg(long)]
        candidate: Option<String>,
        #[arg(long)]
        clock: Option<String>,
    },
    Inspect {
        module: String,
    },
    Deps(Src),
}

fn read_arg(s: &str) -> Result<String, String> {
    if let Some(f) = s.strip_prefix('@') {
        std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))
    } else {
        Ok(s.to_string())
    }
}

fn source(s: &Src) -> Result<String, String> {
    if let Some(e) = &s.expr {
        return Ok(e.clone());
    }
    match s.file.as_deref() {
        Some("-") | None => {
            let mut b = String::new();
            std::io::stdin()
                .read_to_string(&mut b)
                .map_err(|e| e.to_string())?;
            Ok(b)
        }
        Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}")),
    }
}

pub fn parse_schema(text: &str) -> Result<Schema, String> {
    // accept `inputs` as an array of {name,type} or an object name → type
    let mut v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("schema: {e}"))?;
    if let Some(obj) = v.get_mut("inputs").and_then(|x| x.as_object().cloned()) {
        let arr: Vec<serde_json::Value> = obj
            .into_iter()
            .map(|(k, t)| serde_json::json!({"name": k, "type": t}))
            .collect();
        v["inputs"] = serde_json::Value::Array(arr);
    }
    if let Some(obj) = v.get_mut("types").and_then(|x| x.as_object().cloned()) {
        let arr: Vec<serde_json::Value> = obj
            .into_iter()
            .map(|(k, t)| serde_json::json!([k, t]))
            .collect();
        v["types"] = serde_json::Value::Array(arr);
    }
    serde_json::from_value(v).map_err(|e| format!("schema: {e}"))
}

fn options(s: &Src) -> Result<CompileOptions, String> {
    let mut o = CompileOptions::default();
    if s.unary_tests {
        o.entry = Entry::UnaryTests;
    }
    if let Some(sc) = &s.schema {
        o.schema = parse_schema(&read_arg(sc)?)?;
    }
    o.check = if s.no_check {
        CheckMode::Off
    } else if s.strict {
        CheckMode::Strict
    } else {
        CheckMode::Permissive
    };
    o.optimize = !s.no_opt;
    o.extensions = s.extensions.clone();
    o.dialect = match s.dialect.as_str() {
        "feel-1.5" => feel_compiler::Dialect::Feel15,
        "b-feel" => feel_compiler::Dialect::BFeel,
        d => {
            return Err(format!(
                "unknown dialect '{d}' (expected feel-1.5 or b-feel)"
            ))
        }
    };
    Ok(o)
}

fn print_diags(src: &str, diags: &[feel_compiler::diag::Diagnostic]) {
    for d in diags {
        eprintln!("{d}");
        let line = src
            .lines()
            .nth(d.line.saturating_sub(1) as usize)
            .unwrap_or("");
        if !line.is_empty() {
            eprintln!("    {line}");
            let width = (d.span.end.saturating_sub(d.span.start)).max(1);
            eprintln!(
                "    {}{}",
                " ".repeat(d.column.saturating_sub(1) as usize),
                "^".repeat(width.min(line.len().max(1)))
            );
        }
    }
}

fn clock(c: &Option<String>) -> ClockSpec {
    match c {
        Some(s) if s == "none" => ClockSpec::None,
        Some(s) => ClockSpec::Fixed(s.clone()),
        None => ClockSpec::System,
    }
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Parse(s) => {
            let src = source(&s)?;
            let o = options(&s)?;
            let p = feel_compiler::parse(&src, &o);
            print_diags(&src, &p.diagnostics);
            match p.program {
                Some(prog) => {
                    println!("{}", serde_json::to_string_pretty(&prog).unwrap());
                    Ok(ExitCode::SUCCESS)
                }
                None => Ok(ExitCode::from(1)),
            }
        }
        Cmd::Check(s) => {
            let src = source(&s)?;
            let o = options(&s)?;
            let c = feel_compiler::check(&src, &o);
            print_diags(&src, &c.diagnostics);
            println!("{}", serde_json::to_string_pretty(&c).unwrap());
            Ok(if c.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Cmd::Deps(s) => {
            let src = source(&s)?;
            let o = options(&s)?;
            let out = feel_compiler::compile(&src, &o);
            print_diags(&src, &out.diagnostics);
            match out.metadata {
                Some(m) => {
                    println!("{}", serde_json::to_string_pretty(&m.dependencies).unwrap());
                    Ok(ExitCode::SUCCESS)
                }
                None => Ok(ExitCode::from(1)),
            }
        }
        Cmd::Compile { src: s, out, dump } => {
            let src = source(&s)?;
            let o = options(&s)?;
            let c = feel_compiler::compile(&src, &o);
            print_diags(&src, &c.diagnostics);
            let Some(wasm) = &c.wasm else {
                return Ok(ExitCode::from(1));
            };
            std::fs::write(&out, wasm).map_err(|e| e.to_string())?;
            let meta_path = format!("{out}.meta.json");
            std::fs::write(
                &meta_path,
                serde_json::to_string_pretty(&c.metadata).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            eprintln!("wrote {out} ({} bytes) and {meta_path}", wasm.len());
            if dump {
                println!("{}", dump_module(wasm));
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Run {
            src: s,
            inputs,
            candidate,
            clock: ck,
            max_steps,
        } => {
            let src = source(&s)?;
            let o = options(&s)?;
            let c = feel_compiler::compile(&src, &o);
            print_diags(&src, &c.diagnostics);
            let Some(wasm) = &c.wasm else {
                return Ok(ExitCode::from(1));
            };
            let engine = Engine::new();
            let p = engine.load(wasm).map_err(|e| e.to_string())?;
            let mut io = InstanceOptions::default();
            if let Some(m) = max_steps {
                io.limits.max_steps = m;
            }
            io.host = feel_host_bindings();
            let mut inst = p.instantiate(&engine, io).map_err(|e| e.to_string())?;
            let inputs = read_arg(&inputs)?;
            let eo = EvalOptions { clock: clock(&ck) };
            let r = match &candidate {
                Some(cand) => inst.evaluate_unary(&read_arg(cand)?, &inputs, &eo),
                None => inst.evaluate(&inputs, &eo),
            };
            report(&src, &r)
        }
        Cmd::Exec {
            module,
            inputs,
            batch,
            candidate,
            clock: ck,
        } => {
            let bytes = std::fs::read(&module).map_err(|e| format!("{module}: {e}"))?;
            let engine = Engine::new();
            let p = engine.load(&bytes).map_err(|e| e.to_string())?;
            let io = InstanceOptions {
                host: feel_host_bindings(),
                ..Default::default()
            };
            let mut inst = p.instantiate(&engine, io).map_err(|e| e.to_string())?;
            let eo = EvalOptions { clock: clock(&ck) };
            if let Some(b) = batch {
                let text = std::fs::read_to_string(&b).map_err(|e| e.to_string())?;
                let lines: Vec<String> = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.to_string())
                    .collect();
                let mut ok = true;
                for r in inst.evaluate_batch(&lines, &eo) {
                    println!("{}", serde_json::to_string(&r).unwrap());
                    ok &= r.abort.is_none();
                }
                return Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(2)
                });
            }
            let inputs = read_arg(&inputs)?;
            let r = match &candidate {
                Some(cand) => inst.evaluate_unary(&read_arg(cand)?, &inputs, &eo),
                None => inst.evaluate(&inputs, &eo),
            };
            report("", &r)
        }
        Cmd::Inspect { module } => {
            let bytes = std::fs::read(&module).map_err(|e| format!("{module}: {e}"))?;
            feel_compiler::validate_module(&bytes)?;
            let (meta, consts) = feel_compiler::read_module_sections(&bytes)?;
            println!("{}", serde_json::to_string_pretty(&meta).unwrap());
            println!("constants: {consts}");
            println!("{}", dump_module(&bytes));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn report(src: &str, r: &feel_host::Evaluation) -> Result<ExitCode, String> {
    for d in &r.diagnostics {
        let snippet = src.get(d.start..d.end).unwrap_or("");
        if snippet.is_empty() {
            eprintln!("note[{}] {}", d.code, d.message);
        } else {
            eprintln!("note[{}] {} (at `{}`)", d.code, d.message, snippet);
        }
    }
    match (&r.result, &r.abort) {
        (Some(v), _) => {
            println!("{v}");
            Ok(ExitCode::SUCCESS)
        }
        (None, Some(a)) => {
            eprintln!("aborted [{}] {}: {}", a.kind, a.code, a.message);
            Ok(ExitCode::from(2))
        }
        _ => Ok(ExitCode::from(2)),
    }
}

/// Test bindings for external functions (DMN TCK 0076 uses java.lang.Math).
fn feel_host_bindings() -> feel_host::HostFunctions {
    feel_host::bindings::test_bindings()
}

/// A compact human-readable listing of the module structure.
fn dump_module(bytes: &[u8]) -> String {
    let mut out = String::new();
    for payload in wasmparser_iter(bytes) {
        out.push_str(&payload);
        out.push('\n');
    }
    out
}

fn wasmparser_iter(bytes: &[u8]) -> Vec<String> {
    feel_host::inspect::describe(bytes)
}

fn main() -> ExitCode {
    match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(3)
        }
    }
}
