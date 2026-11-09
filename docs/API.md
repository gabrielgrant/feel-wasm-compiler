# API

The same workflow is exposed in Rust, TypeScript and the CLI. It mirrors CEL:

| Step | Rust | TypeScript (`web/src/feel.ts`) | CLI |
|---|---|---|---|
| parse | `feel_compiler::parse(src, &opts)` | `compiler.parse(src, opts)` | `feel parse` |
| check | `feel_compiler::check(src, &opts)` | `compiler.check(src, opts)` | `feel check` |
| compile | `feel_compiler::compile(src, &opts)` → bytes, diagnostics, metadata | `compiler.compile(src, opts)` / `compileCached` | `feel compile -o m.wasm` |
| instantiate | `Engine::load(bytes)?.instantiate(&engine, InstanceOptions)` | `runtime.instantiate(compiled, opts)` | (implicit) |
| evaluate | `instance.evaluate(inputs_json, &EvalOptions)` | `instance.evaluate(inputs)` / `evaluateJson(text)` | `feel run`, `feel exec m.wasm` |
| unary tests | `CompileOptions{entry: Entry::UnaryTests}` + `instance.evaluate_unary(candidate, inputs, ..)` | `{entry: 'unaryTests'}` + `instance.evaluateUnary(candidate, inputs)` | `--unary-tests`, `--candidate` |
| batch | `instance.evaluate_batch(&inputs, ..)` | `instance.evaluateBatch(rows)` | `feel exec m.wasm --batch rows.ndjson` |
| inspect | `read_module_sections`, `validate_module`, `inspect::ModuleSummary` | `metadata` on the compile result | `feel inspect m.wasm`, `feel deps` |

## Rust

```rust
use feel_compiler::{compile, check, CompileOptions, Entry, check::CheckMode};
use feel_host::{Engine, InstanceOptions, EvalOptions, ClockSpec};

let mut opts = CompileOptions::from_json(r#"{
  "check": "strict",
  "schema": {
    "inputs": { "applicant": "context<name: string, age: number, email?: string>", "score": "number?" },
    "expected": "boolean",
    "closed": true
  }
}"#)?;
let report = check("applicant.age >= 18 and (score = null or score > 600)", &opts);
assert!(report.ok);                                  // result type, diagnostics

let out = compile("applicant.age >= 18 and (score = null or score > 600)", &opts);
let wasm = out.wasm.expect("compiled");              // Vec<u8>
let meta = out.metadata.unwrap();                    // inputs, imports, sites, dependencies, cache key

let engine = Engine::new();
let program = engine.load(&wasm)?;                   // validates, checks ABI version
let mut inst = program.instantiate(&engine, InstanceOptions::default())?;
let r = inst.evaluate(r#"{"applicant": {"name": "Ann", "age": 30}, "score": 712}"#,
                      &EvalOptions { clock: ClockSpec::Fixed("2024-01-01T00:00:00Z".into()) });
assert_eq!(r.result.as_deref(), Some("true"));      // FEEL-JSON text; r.diagnostics; r.abort
```

* `feel_compiler::compile_program(&Program, ..)` compiles an AST built by an integration layer, as the DMN layer does. `lower()` returns the IR for inspection.
* `InstanceOptions { limits: Limits {max_steps, max_depth, max_values, max_collection, max_string, max_diags}, host: HostFunctions }`.
* `HostFunctions::add(name, |args_json: &[String]| -> Result<String, String>)` binds a schema-declared host function, or an external FEEL function under its key (`java:java.lang.Math.max(double,double)`). `feel_host::bindings::test_bindings()` provides the `java.lang` methods used by TCK 0076.
* `feel_host::recalc::{Workbook, ModuleCache}`: dependency-driven incremental recalculation, with compiled modules cached by `CompileOptions::cache_key(source)`.

## TypeScript

See [web/README.md](../web/README.md) for the full API. In short:

```ts
import { loadCompiler, loadRuntime, FeelDecimal } from './dist/src/feel.js';
const compiler = await loadCompiler('/dist/compiler.wasm');
const runtime = await loadRuntime('/dist/runtime.wasm');
const c = compiler.compile('price * qty', { schema: { inputs: { price: 'number', qty: 'number' } } });
const inst = await runtime.instantiate(c, { limits: { maxSteps: 1_000_000 } });
inst.evaluate({ price: new FeelDecimal('19.99'), qty: 3 }).value;          // FeelDecimal("59.97")
inst.evaluateBatch([{ price: '0.10', qty: 3 }, { price: 1, qty: 2 }]);  // no recompilation
```

Decimals are carried as strings (`FeelDecimal`); JSON results are parsed by a decimal-preserving parser, never by `JSON.parse`. A JS `number` input is converted with `String(n)`, the shortest round-trip decimal of the double; use `FeelDecimal`, strings or `bigint` for exact inputs.

## Options

```jsonc
{
  "entry": "expression" | "unaryTests",
  "dialect": "feel-1.5",
  "check": "off" | "permissive" | "strict",       // default permissive
  "optimize": true,
  "extensions": ["has key"],                       // opt-in, outside the DMN 1.5 claim
  "validateInputs": true,                          // check declared input types at the boundary
  "knownNames": ["Profit and loss"],               // extra multi-word names for the parser
  "knownFields": ["Monthly Salary"],
  "schema": {
    "inputs": { "name": "type", ... },             // or [{"name": .., "type": ..}]
    "types": { "tPerson": "context<name: string, age: number>" },   // aliases (e.g. item definitions)
    "functions": [{ "name": "lookup rate", "params": [{"name": "grade", "type": "string"}], "returns": "number" }],
    "candidate": "number",                         // type of `?` for unary tests
    "expected": "boolean",                         // expected result type
    "closed": false                                // true: undeclared names are unknown (not inputs)
  }
}
```

**Schema type language:** rule 52 type expressions (`number`, `string`, `boolean`, `date`, `time`, `date and time`, `days and time duration`, `years and months duration`, `list<T>`, `range<T>`, `context<k: T, ...>`, `function<T, ...>->R`, `Any`, `Null`), plus:

* `T?` for a **nullable** type (the value may be null);
* `k?: T` for an **optional** context field (the key may be absent);
* `...` in a context type to mark it **open**.

Selecting either kind of field gives `T?`. `get entries` keeps present null-valued entries, and only absent keys are missing (`has key` distinguishes them when enabled).

## Diagnostics

| Codes | Kind | Effect |
|---|---|---|
| `FEEL-S0xx` | syntax (lexer/parser/literals) | compilation fails |
| `FEEL-C1xx` | checker findings (unknown name or field, operator/comparison/argument type, arity, non-boolean condition, expected type, unary-test candidate, non-function, unknown type) | errors under `strict`, warnings under `permissive` |
| `FEEL-W2xx` | warnings: `-a ** b` precedence, possibly-null operands (strict), filtering a non-list | never fatal |
| `FEEL-E3xx` | evaluation diagnostics that accompany a FEEL value (usually null): e.g. E301 null operand, E302 undefined operation, E303 out of range, E305 division by zero, E307 incomparable, E31x selection/index, E32x invocation, E330 binding conversion, E340 builtin domain, E350 missing input, E36x host functions | the value is still produced |
| `FEEL-L40x` | resource limits: L401 values, L402 collection size, L403 string size, L404 steps, L405 call depth | evaluation aborted; no FEEL value |
| `FEEL-B5xx` | boundary/ABI: B501 schema, B502 slot, B503 trap, B504 clock, B505 invalid FEEL-JSON, B506 declared type violated, B590 cancelled (browser worker) | evaluation aborted |

Every diagnostic has a stable `code`, `severity`, `message`, byte `span`, 1-based `line`/`column`, and an optional `suggestion` (e.g. "did you mean 'age'?", "guard with `score != null`").

## Dependencies

`metadata.dependencies` reports:

* `inputs` — each name, whether the whole value is read, and the statically known field paths;
* `clock` — whether `now()`/`today()` are used;
* `hostFunctions`;
* `exact`;
* `uncertain` — reasons, such as `get value` with a computed key, invoking a computed function value, implicit filter names that may shadow inputs, or external functions.

A host invalidates a formula when any listed input changes. When `exact` is false, field-level invalidation is not sound.

## CLI

```
feel parse   [-e EXPR | FILE] [--unary-tests]
feel check   [-e EXPR | FILE] [--schema JSON|@file] [--strict] [--ext "has key"]
feel compile [-e EXPR | FILE] -o out.wasm [--no-opt] [--dump]      # also writes out.wasm.meta.json
feel run     [-e EXPR | FILE] [--inputs JSON|@file] [--candidate JSON] [--clock INSTANT|none] [--max-steps N]
feel exec    MODULE.wasm [--inputs JSON] [--batch FILE.ndjson] [--candidate JSON] [--clock ..]
feel inspect MODULE.wasm
feel deps    [-e EXPR | FILE]
feel-tck     TCK/TestCases [--json out] [--md out] [--no-opt] [--host-dt] [--non-compliant] [--filter s] [-v]
```

Exit codes: 0 success, 1 compile or check errors, 2 evaluation aborted, 3 usage/IO errors.
