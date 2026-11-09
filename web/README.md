# FEEL → WebAssembly in the browser

TypeScript bindings, an offline demo, end-to-end tests and benchmarks for the
FEEL (DMN 1.5) → WebAssembly compiler in this repository.

Three Wasm artifacts are involved:

| File | Built from | What it is |
| --- | --- | --- |
| `dist/compiler.wasm` | `crates/feel-web` | The compiler itself (parse, check, emit), no imports. |
| `dist/runtime.wasm` | `crates/feel-runtime-wasm` | The shared FEEL runtime (`feel_rt_v1` ABI): exact decimals, temporal values, builtins, FEEL-JSON. |
| emitted modules | the compiler | One small module per expression (~1–4 KB), importing only runtime helpers. |

Compilation and evaluation run entirely in the page. After the artifacts are
loaded, nothing touches the network.

## Build

Requirements: the Rust toolchain from `rust-toolchain.toml` (with the
`wasm32-unknown-unknown` target), Node 22 and npm.

```sh
scripts/build-web.sh
```

This runs `cargo build --profile wasm --target wasm32-unknown-unknown -p feel-web -p feel-runtime-wasm`,
copies the two artifacts to `web/dist/compiler.wasm` and `web/dist/runtime.wasm`,
installs the pinned dev dependencies (`npm ci`: typescript 5.9.3, esbuild
0.28.2, feelin 7.0.2), compiles the TypeScript with `tsc` (ES modules in
`web/dist/src`, `web/dist/demo`, `web/dist/bench`) and bundles two files with
esbuild:

- `dist/worker.bundle.js` is the self-contained worker script, so `FeelWorker` can restart it from a `blob:` URL while offline.
- `dist/vendor/feelin.js` is feelin as one ES module. Only the benchmark uses it.

## Run the demo

```sh
node web/serve.mjs            # http://localhost:8080/demo/  (port: first argument or $PORT)
```

The page loads `compiler.wasm` and `runtime.wasm`. You can then:

- edit FEEL source (pre-filled with a loan-style context with a filter and decimals);
- choose *expression* or *unary tests*, optionally give a schema, and turn on strict checking;
- **Compile**: shows diagnostics (code, line:col, message, suggestion), the inferred result type, module size, compile and instantiate times, the runtime imports and the dependency report;
- **Evaluate all**: runs the already compiled module against every row of a JSON array of inputs, without recompiling. It shows exact results, evaluation diagnostics, aborts and per-row timings;
- optionally **run in worker with timeout**: runaway evaluations are cancelled by terminating the worker, which restarts transparently.

It works at phone width and offline after load. The server sends COOP/COEP
headers, so the page is cross-origin isolated and `performance.now()` has 5 µs
resolution.

## Tests

```sh
node web/tests/e2e.mjs        # or: cd web && npm test
```

The test drives Chromium through Playwright 1.63.0, resolved with
`require('playwright')`. It is not a dependency of this package: install it
globally if it is missing (`npm i -g playwright@1.63.0`).

The test serves `web/`, opens the demo and waits for the artifacts and the
worker. It then calls `context.setOffline(true)` and checks that no request
succeeds from that point on. While offline it checks:

1. A new expression typed into the page compiles. The result type, imports and size are shown.
2. The compiled module is evaluated against several rows without recompiling, with exact results: `0.1 + 0.2` gives `0.3`, 29-digit integers stay exact, `1/3` gives 34 digits, and `1.10` keeps its scale.
3. A syntax error produces a `FEEL-S0xx` diagnostic with line:col. A strict checker error shows a suggestion.
4. Unary tests are evaluated against several candidates.
5. A long-running evaluation in the worker is cancelled by its timeout (`abort.kind = "cancelled"`). The worker restarts and evaluates the same module and a new expression. A request queued behind the cancelled one is re-dispatched.
6. With default limits, the step limit produces a `limit` abort (`FEEL-L404`), not `null`.
7. Instance isolation holds across clocks, limits and host functions. Live values and linear memory stay bounded over 2000 evaluations. 6000 aborts inside nested runtime frames are absorbed.
8. FEEL-JSON round trips through the TS codec and the runtime are exact.
9. Every vector in `tests/vectors/semantic.json` runs in the browser through compiler.wasm, runtime.wasm and the generated modules. Each runs optimized and unoptimized, and the two outcomes must agree. The semantics follow `crates/feel-host/tests/vectors.rs`.

## Benchmarks

```sh
node web/bench/run.mjs        # ~1.5 min; writes web/bench/results.json and web/bench/RESULTS.md
node web/bench/run.mjs --quick         # smoke run of the harness
node web/bench/run.mjs --render-only   # re-render RESULTS.md from results.json
```

The benchmark measures the following, each reported with median, p90/p99, min, iteration and warmup counts and input sizes:

- cold startup in fresh browser contexts;
- parse, check and compile times per expression;
- instantiation;
- warm evaluation per call against feelin 7.0.2, only where the results are equivalent (exclusions are listed with reasons);
- one-shot compile + instantiate + evaluate;
- JS ↔ Wasm transfer cost for lists of 10, 1,000 and 100,000 elements, with a per-phase breakdown.

See [bench/RESULTS.md](bench/RESULTS.md).

## TypeScript API

Everything is exported from `dist/src/feel.js` (source `src/feel.ts`, with
JSDoc). By default the artifacts are resolved relative to that module
(`dist/compiler.wasm`, `dist/runtime.wasm`). Any URL, `Response`, bytes or
`WebAssembly.Module` can be passed instead.

### Compile in the browser and evaluate many times

```ts
import { loadCompiler, loadRuntime, FeelDecimal } from './dist/src/feel.js';

const compiler = await loadCompiler();
const runtime = await loadRuntime();

const out = compiler.compile('(principal * rate / 12) / (1 - (1 + rate / 12) ** -term)', {
  check: 'strict',
  schema: { inputs: { principal: 'number', rate: 'number', term: 'number' } },
});
if (!out.ok) {
  for (const d of out.diagnostics) console.log(`${d.code} ${d.line}:${d.column} ${d.message}`, d.suggestion ?? '');
} else {
  console.log(out.metadata!.resultType, out.bytes.length, out.metadata!.imports, out.metadata!.dependencies);
  const inst = await runtime.instantiate(out, { limits: { maxSteps: 1_000_000 } });
  for (const row of rows) {
    const r = inst.evaluate(row); // no recompilation, no re-instantiation
    if (r.ok) console.log(r.valueJson);                         // exact FEEL-JSON text
    else console.log(r.abort);                                   // {kind: 'limit'|'boundary', code, message}
    for (const d of r.diagnostics) console.log(d.code, d.message, d.start, d.end);
  }
}
```

The other compiler entry points are `compiler.parse(source, options)` (AST and
syntax diagnostics), `compiler.check(source, options)` (`{ok, resultType, diagnostics}`)
and `compiler.version` (from `fw_version()`).

`compiler.compileCached(source, options)` memoizes results. Its lookup key
covers the source, the canonicalized options and the compiler, runtime and ABI
versions. It also keeps the compiled `WebAssembly.Module`, so later
`instantiate` calls skip Wasm compilation. Every result also carries the
compiler's SHA-256 `cacheKey`, which suits persistent caches.

Diagnostic spans are UTF-8 **byte** offsets; `byteToCharOffset(source, n)`
converts them to JS string indices.

### Unary tests (decision-table cells)

```ts
const ut = compiler.compile('[18..65), > 90', { entry: 'unaryTests' });
const cell = await runtime.instantiate(ut);
cell.evaluateUnary(42).valueJson;    // "true"
cell.evaluateUnary(65).valueJson;    // "false"
cell.evaluateUnary(null).valueJson;  // "null"
```

### Precision-preserving inputs and outputs

Inputs are encoded to FEEL-JSON. Numbers never go through binary floating
point unless you hand the binding a JS `number`:

| JS input | FEEL value |
| --- | --- |
| `new FeelDecimal('12345678901234567890.25')`, `123n` | exact number |
| `0.1` (a JS `number`) | `String(n)`: the shortest decimal that round-trips the double. `0.1` stays `0.1`, but a computed `0.1 + 0.2` arrives as `0.30000000000000004`. `NaN`/`±Infinity` throw. |
| `'text'`, `true`, `null`/`undefined` | string, boolean, null |
| `FeelDate`, `FeelTime`, `FeelDateTime`, `FeelDuration` (string-backed) and JS `Date` | temporal values (`Date` becomes a UTC date and time) |
| `FeelRange` | range |
| arrays; plain objects or `Map`s | lists; contexts |

Outputs are parsed by a small hand-written FEEL-JSON parser, never `JSON.parse`.
Every number becomes a `FeelDecimal` (`toString()` returns the exact text,
scale included: `1.10`). Contexts become plain objects by default, or `Map`s
with `decode: { contexts: 'map' }`, which preserves key order exactly.

```ts
const inst = await runtime.instantiate(compiler.compile('a + b'));
const r = inst.evaluate({ a: new FeelDecimal('0.1'), b: new FeelDecimal('0.2') });
r.valueJson;                 // "0.3"
String(r.value);             // "0.3" (FeelDecimal)

inst.evaluateJson('{"a": 1.10, "b": 2}').valueJson;    // "3.10": raw FEEL-JSON in and out
```

`encodeFeelJson(value)` and `parseFeelJson(text, options)` are exported as well.

### Limits, aborts, clock, host functions

```ts
const inst = await runtime.instantiate(out, {
  limits: { maxSteps: 10_000_000, maxDepth: 256, maxValues: 1_000_000, maxCollection: 100_000 },
  clock: '2024-01-01T10:00:00Z',   // or 'system' (default) | 'none' | a JS Date
  hostFunctions: { greet: ([name]) => `hello ${name}` },   // declared in schema.functions
});
```

When a resource limit is exceeded, the result has `ok: false`, `valueJson: null`
and `abort: {kind: 'limit', code: 'FEEL-L40x', message}`. This is distinct
from a FEEL `null` result. Bad inputs give `kind: 'boundary'`: `FEEL-B505` for
invalid FEEL-JSON and `FEEL-B506` for a schema type violation when
`validateInputs` is on. A host function that throws yields `null` plus
`FEEL-E362`. These codes match the native host (`crates/feel-host`).

Binding-only codes: `FEEL-B590` (worker evaluation cancelled), `FEEL-B591`
(worker failed) and `FEEL-B592` (module incompatible with the runtime, thrown
as `FeelModuleError`). `FEEL-B503` means a trap without abort information,
for example an engine stack overflow from very deep recursion.

### Isolation and resource hygiene

Each `FeelInstance` owns its own runtime instance and expression-module
instance, so memory, value arena, limits, clock and host bindings are all
separate. The runtime releases the previous evaluation's values when the next
one starts (`instance.evalValues()` exposes the count). After an evaluation
aborts by trapping, the instance transparently replaces its runtime/module pair
before the next evaluation (about 1 ms). The reason: a trap abandons Rust frames
in `runtime.wasm` mid-call, and their shadow-stack space is never released (see
"Known runtime issues" below). `instance.recycles` counts these replacements.

`instance.profile = true` records per-phase timings in `instance.lastProfile`.

### Web Worker with cancellable timeouts

```ts
import { FeelWorker } from './dist/src/feel.js';

const w = await FeelWorker.create({ compiler: compiler.module, runtime: runtime.module }); // or defaults (fetch)
const prog = (await w.compile('count(for i in 1..n return i)')).program!; // or w.load(compileResult): no recompile
const r = await w.evaluate(prog, { n: 10 }, { timeoutMs: 200, limits: { maxSteps: 1e12 } });
if (r.abort?.kind === 'cancelled') { /* the worker was terminated and has already restarted */ }
```

The worker script is fetched once and runs from a `blob:` URL, and the compiled
Wasm modules are posted to it. Restarts therefore need no network. Requests
queued behind a cancelled evaluation are re-sent to the new worker. Host
functions cannot cross the worker boundary.

## Files

| Path | Purpose |
| --- | --- |
| `src/feel.ts` | Compiler/runtime bindings, `FeelInstance`, cache |
| `src/values.ts` | Value classes and the FEEL-JSON encoder/parser |
| `src/feel-worker.ts`, `src/worker.ts` | `FeelWorker` (main thread) and the worker entry |
| `demo/` | Demo page (`index.html`, `demo.ts`, `demo.css`) |
| `tests/e2e.mjs` | Playwright end-to-end test |
| `bench/` | Benchmark page (`bench.ts`), driver (`run.mjs`), results |
| `serve.mjs` | Static server (`createServer()` is also used by the test and the benchmark) |
| `build-bundles.mjs` | esbuild step for the worker and feelin bundles |

## Known runtime issues (reported to the Rust side)

- **Traps leak shadow stack.** Each abort that traps inside nested runtime frames (for example a step limit hit inside a `sort` comparator) leaves `__stack_pointer` lowered. After about 5,400 such aborts on one `runtime.wasm` instance, every call fails with "memory access out of bounds". The bindings work around this by recycling the instance after a trap.
- **`@context` escape does not decode.** The runtime's FEEL-JSON decoder rejects `{"@context": {"@x": 1}}` ("unknown tag @x"), which is its own encoding of a context whose only key starts with `@`. Such contexts can be returned but not passed back in as inputs.
- **Wrong step limit in the message.** `FEEL-L404` reports the default step limit (50,000,000) instead of the value passed to `__feel_set_limits`, because `rt_set_limits` has no steps parameter.
