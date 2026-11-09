# feel-wasm-compiler

A compiler from **FEEL** (the *Friendly Enough Expression Language* of OMG DMN 1.5) to **WebAssembly**, written in Rust.

* **The compiler itself runs in Wasm.** The same Rust source builds natively (CLI, library) and for `wasm32-unknown-unknown` (`compiler.wasm`). A browser can compile FEEL entered at runtime, entirely offline.
* **Every FEEL program compiles to its own Wasm module.** The module holds code specialised to the expression: control flow, three-valued logic, loops for filters, `for`, `some` and `every`, closures through a funcref table, and step and depth counters. It calls only primitive helpers of a shared, versioned runtime (`feel_rt_v1`: decimal arithmetic, comparisons, collections, temporal values, builtins). It never embeds or calls an interpreter of FEEL source, ASTs or bytecode.
* **FEEL numbers are decimals.** Arithmetic uses decimal128 with 34 significant digits and round-half-even, as the spec requires (no `f64`). Values cross the JS boundary as precision-preserving FEEL-JSON.

```text
0.1 + 0.2                         → 0.3
1/3                               → 0.3333333333333333333333333333333333
decimal(0.515, 2)                 → 0.52
-4 ** 2                           → 16           (FEEL precedence)
[ {x:1, y:2}, {x:2, y:3} ][x=1].y → [2]
for i in 0..5 return if i = 0 then 1 else i * partial[-1] → [1,1,2,6,24,120]
date and time("2018-07-10T10:30:00@Europe/Paris").time offset → PT2H
```

## Results at a glance

| Evidence | Result |
|---|---|
| DMN TCK, pinned revision `5cd912f2`, compliance levels 2 and 3, through generated Wasm | **3,511 / 3,511** result nodes pass (0 fail, 0 skip, 0 out of scope) — optimized and unoptimized |
| Decision tables: one Wasm module per cell (host-orchestrated) vs one module per table (lowered) | 83 TCK results compared, 0 mismatches |
| Semantic vectors (`tests/vectors/semantic.json`) | **407 / 407**, native (optimized and unoptimized, differential) and in Chromium (compiler.wasm + runtime.wasm) |
| Browser end-to-end (Playwright, offline after load) | 464 / 464 checks |
| Decimal library vs Python `decimal` oracle | 21,738 vectors, 0 mismatches |
| TCK `non-compliant` folder (informational, outside the claim) | 39 / 50 |

The conformance claim, its scope and the exact clauses are in [docs/CONFORMANCE.md](docs/CONFORMANCE.md). The clause-by-clause and builtin-by-builtin matrix is in [docs/CONFORMANCE-MATRIX.md](docs/CONFORMANCE-MATRIX.md).

## Build

Requirements: Rust 1.97.0 (pinned in `rust-toolchain.toml`, which also installs the `wasm32-unknown-unknown` target) and Node 22 for the browser part. All dependency versions are pinned exactly (`Cargo.toml` uses `=x.y.z`; `Cargo.lock` and `web/package-lock.json` are committed).

```bash
cargo build --release                      # native compiler, CLI (target/release/feel), TCK runner
```

```bash
scripts/build-web.sh                       # compiler.wasm + runtime.wasm + TypeScript → web/dist/
```

```bash
cargo test --workspace --release           # unit, vector, module, checker, decision-table tests
```

```bash
scripts/run-tck.sh                         # fetches the pinned TCK into .tck/ and writes reports/
```

```bash
node web/tests/e2e.mjs                     # Playwright: offline in-browser compile + evaluate
```

```bash
node web/bench/run.mjs                     # browser benchmarks → web/bench/RESULTS.md
```

`scripts/check-all.sh` runs everything CI runs (`.github/workflows/ci.yml`).

## Try it

**Browser demo.** Run `scripts/build-web.sh`, then `node web/serve.mjs` and open <http://localhost:8080/demo/>. Type a formula and compile it in the page; it shows diagnostics, the inferred type, imports and dependencies. Then evaluate the emitted module against several input rows without recompiling, optionally inside a Web Worker with a timeout.

**Native CLI**:

```bash
target/release/feel run -e 'sum(for x in items return x.price * x.qty)' \
  --inputs '{"items":[{"price":19.99,"qty":3},{"price":0.01,"qty":7}]}'
```

```bash
target/release/feel check --strict --schema '{"inputs":{"applicant":"context<age: number>"},"closed":true}' -e 'applicant.agee > 18'
```

```bash
target/release/feel compile -e '[18..65), > 100' --unary-tests -o age.wasm
```

```bash
target/release/feel exec age.wasm --candidate 30
```

```bash
target/release/feel inspect age.wasm
```

The incremental recalculation example uses extracted dependencies and a module cache: `cargo run --release -p feel-host --example recalc`.

## Repository layout

| Path | Contents |
|---|---|
| `crates/feel-decimal` | decimal128 arithmetic: 34 digits, all rounding modes, exact `/`, correctly rounded `sqrt`/`exp`/`ln`/`**` |
| `crates/feel-temporal` | date, time, date and time, both duration kinds, IANA zones (chrono-tz, tzdb 2025b), XSD lexical forms |
| `crates/feel-runtime` | values, operators, the full builtin catalog, FEEL-JSON, runtime types, the `feel_rt_v1` ABI |
| `crates/feel-runtime-wasm` | `runtime.wasm` (the runtime as a Wasm module) |
| `crates/feel-compiler` | lexer, scope-aware parser, checker, IR lowering, optimizer, Wasm emitter, dependency extraction |
| `crates/feel-host` | native execution of emitted modules (wasmi) with the runtime bound natively; test bindings; recalculation example |
| `crates/feel-cli` | the `feel` command |
| `crates/feel-web` | `compiler.wasm` (the compiler as a Wasm module, C ABI) |
| `crates/feel-dmn` | DMN integration layer: DMN XML, decision tables and hit policies, DRG lowering, TCK runner |
| `web/` | TypeScript bindings, Web Worker, demo, Playwright e2e, benchmarks |
| `tests/vectors/` | semantic vectors and their generator scripts |
| `docs/` | architecture, ABI, API, conformance, benchmarks |
| `reports/` | TCK reports (JSON and Markdown) |

## Documentation

* [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): pipeline, IR, code generation, runtime, optimizations
* [docs/ABI.md](docs/ABI.md): the `feel_rt_v1` runtime ABI, module layout, ownership, failure propagation, versioning
* [docs/API.md](docs/API.md): Rust, TypeScript and CLI APIs, schema language, diagnostics codes, limits, capabilities
* [docs/CONFORMANCE.md](docs/CONFORMANCE.md): pinned spec and TCK, the claim, dialects and extensions, semantic choices, gaps
* [docs/BENCHMARKS.md](docs/BENCHMARKS.md): measured performance (browser and native)
* [web/README.md](web/README.md): browser bindings and demo
