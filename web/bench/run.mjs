// Benchmark harness: runs bench/index.html in Chromium (Playwright) and writes
// bench/results.json and bench/RESULTS.md.
//
//   node web/bench/run.mjs [--quick | --render-only]
//
// Requires a prior scripts/build-web.sh. Total runtime is about 1.5 minutes
// (--quick: a few seconds, for smoke-testing the harness).
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from '../serve.mjs';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const HERE = path.dirname(fileURLToPath(import.meta.url));
const quick = process.argv.includes('--quick');

if (process.argv.includes('--render-only')) {
  // re-render RESULTS.md from an existing results.json (no measurements)
  const r = JSON.parse(fs.readFileSync(path.join(HERE, 'results.json'), 'utf8'));
  fs.writeFileSync(path.join(HERE, 'RESULTS.md'), renderMarkdown(r));
  console.log('re-rendered bench/RESULTS.md from bench/results.json');
  process.exit(0);
}

const P = quick
  ? { cold: 3, compile: [5, 20], inst: [3, 10], eval: [50, 10, 2], oneShot: [2, 5], transfer: [1, 5] }
  : { cold: 25, compile: [50, 500], inst: [20, 200], eval: [1000, 100, 10], oneShot: [10, 100], transfer: [5, 100] };

const server = createServer();
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/bench/`;
const browser = await chromium.launch();
const started = Date.now();
const log = (m) => console.log(`[${((Date.now() - started) / 1000).toFixed(1)}s] ${m}`);

async function freshPage() {
  const context = await browser.newContext({ deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('pageerror:', e.message));
  await page.goto(url);
  await page.waitForFunction(() => globalThis.__bench?.ready);
  return { context, page };
}

const results = { generatedAt: new Date().toISOString(), quick, parameters: P };
try {
  // ------------------------------------------------------------ cold start
  log(`cold start: ${P.cold} fresh browser contexts (empty HTTP and code caches)`);
  const cold = [];
  const breakdown = [];
  for (let i = 0; i < P.cold; i++) {
    let { context, page } = await freshPage();
    cold.push(await page.evaluate(() => globalThis.__bench.coldStart()));
    await context.close();
    ({ context, page } = await freshPage());
    breakdown.push(await page.evaluate(() => globalThis.__bench.coldStartBreakdown()));
    await context.close();
  }

  // ------------------------------------------------------------ warm page
  const { context, page } = await freshPage();
  const env = await page.evaluate(() => globalThis.__bench.init());
  const statsFn = (xs) => {
    const s = [...xs].sort((a, b) => a - b);
    const q = (p) => s[Math.min(s.length - 1, Math.max(0, Math.ceil(p * s.length) - 1))];
    return { n: s.length, min: s[0], median: q(0.5), p90: q(0.9), p99: q(0.99), max: s[s.length - 1], mean: s.reduce((a, b) => a + b, 0) / s.length };
  };
  results.environment = {
    chromium: browser.version(),
    playwright: require('playwright/package.json').version,
    node: process.version,
    os: `${os.type()} ${os.release()} ${os.arch()}`,
    cpu: os.cpus()[0]?.model ?? 'unknown',
    logicalCpus: os.cpus().length,
    totalMemGiB: +(os.totalmem() / 2 ** 30).toFixed(1),
    navigatorHardwareConcurrency: env.hardwareConcurrency,
    crossOriginIsolated: env.crossOriginIsolated,
    userAgent: env.userAgent,
    feelWasm: env.version,
    feelin: env.feelin,
  };
  results.coldStart = {
    samples: P.cold,
    totalMs: statsFn(cold.map((c) => c.totalMs)),
    firstCompileInstantiateEvaluateMs: statsFn(cold.map((c) => c.firstCompileInstantiateEvaluateMs)),
    breakdown: Object.fromEntries(Object.keys(breakdown[0]).map((k) => [k, statsFn(breakdown.map((b) => b[k]))])),
  };

  log('parse / check / compile times');
  results.compile = await page.evaluate(([w, n]) => globalThis.__bench.compileTimes(w, n), P.compile);
  log('instantiation times');
  results.instantiate = await page.evaluate(([w, n]) => globalThis.__bench.instantiateTimes(w, n), P.inst);
  log('warm evaluation (ours vs feelin, equivalence-checked)');
  results.evaluate = await page.evaluate(([w, b, t]) => globalThis.__bench.evalTimes(w, b, t), P.eval);
  log('one-shot compile + instantiate + evaluate');
  results.oneShot = await page.evaluate(([w, n]) => globalThis.__bench.oneShotTimes(w, n), P.oneShot);
  log('JS–Wasm transfer');
  results.transfer = await page.evaluate(([w, n]) => globalThis.__bench.transferTimes([10, 1000, 100000], w, n), P.transfer);
  await context.close();
} finally {
  await browser.close();
  server.close();
}
results.durationSeconds = +((Date.now() - started) / 1000).toFixed(1);

fs.writeFileSync(path.join(HERE, 'results.json'), JSON.stringify(results, null, 2) + '\n');
fs.writeFileSync(path.join(HERE, 'RESULTS.md'), renderMarkdown(results));
log(`wrote bench/results.json and bench/RESULTS.md (${results.durationSeconds}s)`);

// ---------------------------------------------------------------- report

function renderMarkdown(r) {
  const ms = (x) => (x >= 100 ? x.toFixed(0) : x >= 10 ? x.toFixed(1) : x >= 1 ? x.toFixed(2) : x.toFixed(3));
  const us = (x) => (x >= 100 ? x.toFixed(0) : x >= 10 ? x.toFixed(1) : x.toFixed(2));
  const row = (cells) => `| ${cells.join(' | ')} |`;
  const head = (cells) => `${row(cells)}\n${row(cells.map(() => '---'))}`;
  const e = r.environment;
  const W = Object.fromEntries(r.evaluate.map((x) => [x.id, x]));
  const out = [];
  out.push('# FEEL → Wasm in the browser: benchmark results');
  out.push('');
  out.push(`Generated by \`node web/bench/run.mjs\`${r.quick ? ' --quick (smoke run; numbers are not meaningful)' : ''} on ${r.generatedAt} (${r.durationSeconds} s). Raw data: [results.json](results.json).`);
  out.push('');
  out.push('## Environment');
  out.push('');
  out.push(head(['Item', 'Value']));
  out.push(row(['Browser', `Chromium ${e.chromium} (headless, Playwright ${e.playwright})`]));
  out.push(row(['Machine', `${e.cpu}; ${e.logicalCpus} logical CPUs; navigator.hardwareConcurrency = ${e.navigatorHardwareConcurrency}; ${e.totalMemGiB} GiB RAM; ${e.os}`]));
  out.push(row(['FEEL compiler (fw_version)', `compiler ${e.feelWasm.compiler}, runtime ${e.feelWasm.runtime}, ABI ${e.feelWasm.abiModule} v${e.feelWasm.abiVersion}, ${e.feelWasm.dialect}, tzdb ${e.feelWasm.tzdb}`]));
  out.push(row(['Comparison', `feelin ${e.feelin} (bundled with esbuild, loaded as an ES module from the same origin)`]));
  out.push(row(['Timer', `performance.now(); crossOriginIsolated = ${e.crossOriginIsolated} (${e.crossOriginIsolated ? '5 µs' : '100 µs'} resolution)`]));
  out.push('');
  out.push('Method: all numbers are wall-clock times measured inside the page. Statistics are nearest-rank percentiles over the samples listed. ');
  out.push('Per-call evaluation times are measured in **batches** (each batch runs the call k times, k chosen so a batch takes ≥ ' + r.parameters.eval[2] + ' ms) and the percentiles are over batch means, so p90/p99 describe batch-to-batch variation, not single-call tails. ');
  out.push('Everything else is timed call by call. One machine, one browser, one run: treat differences below ~10–20 % as noise.');
  out.push('');

  out.push('## 1. Cold browser startup');
  out.push('');
  out.push(`Each sample is a fresh browser context (empty HTTP cache, no Wasm code cache) loading the bench page, then timing the binding's \`loadCompiler()\` + \`loadRuntime()\` in parallel (fetch over localhost + streaming compile + instantiate), as the demo does. n = ${r.coldStart.samples}.`);
  out.push('');
  out.push(head(['Measure', 'median ms', 'p90 ms', 'min ms', 'max ms']));
  const cs = (label, s) => out.push(row([label, ms(s.median), ms(s.p90), ms(s.min), ms(s.max)]));
  cs('compiler.wasm + runtime.wasm ready (parallel)', r.coldStart.totalMs);
  cs('then first compile + instantiate + evaluate (`a + b`)', r.coldStart.firstCompileInstantiateEvaluateMs);
  const b = r.coldStart.breakdown;
  out.push('');
  out.push(`Sequential breakdown (separate fresh contexts; sizes: compiler.wasm ${b.compilerBytes.median.toLocaleString('en-US')} bytes, runtime.wasm ${b.runtimeBytes.median.toLocaleString('en-US')} bytes, uncompressed):`);
  out.push('');
  out.push(head(['Artifact', 'fetch ms (median)', 'WebAssembly.compile ms (median / p90)', 'instantiate ms (median)']));
  for (const a of ['compiler', 'runtime']) {
    out.push(row([`${a}.wasm`, ms(b[`${a}FetchMs`].median), `${ms(b[`${a}CompileMs`].median)} / ${ms(b[`${a}CompileMs`].p90)}`, ms(b[`${a}InstantiateMs`].median)]));
  }
  out.push('');
  out.push('Note: V8 compiles Wasm lazily/tiered (Liftoff first, TurboFan later), so `WebAssembly.compile` time is mostly validation + baseline compilation; hot functions are re-optimized in the background later.');
  out.push('');

  out.push('## 2. Compilation in the browser (parse / check / emit)');
  out.push('');
  out.push(`Times of the synchronous \`compiler.parse\`, \`compiler.check\` and \`compiler.compile\` calls (each includes UTF-8 transfer and JSON report serialization; \`parse\` also serializes the AST). \`compile\` = parse + type check + lowering + optimization + Wasm emission. Default options (permissive check, optimizer on). Warmup ${r.compile[0].warmup}, iterations ${r.compile[0].iterations} per phase.`);
  out.push('');
  out.push(head(['Workload', 'parse median ms', 'check median ms', 'compile median / p90 / p99 ms', 'compile min ms', 'module bytes', 'imports']));
  for (const c of r.compile) {
    out.push(row([`\`${c.id}\``, ms(c.parse.median), ms(c.check.median), `${ms(c.compile.median)} / ${ms(c.compile.p90)} / ${ms(c.compile.p99)}`, ms(c.compile.min), c.moduleBytes, c.imports]));
  }
  out.push('');

  out.push('## 3. Wasm instantiation');
  out.push('');
  out.push(`\`await runtime.instantiate(...)\`: a fresh runtime.wasm instance (own memory) + the expression-module instance + constant-pool load + limits. "with module compile" passes the emitted bytes (includes \`WebAssembly.compile\` of the small module); "instances only" passes a precompiled \`WebAssembly.Module\` (what \`compileCached\` enables). Warmup ${r.instantiate[0].warmup}, iterations ${r.instantiate[0].iterations}.`);
  out.push('');
  out.push(head(['Workload', 'module bytes', 'with module compile: median / p90 / p99 ms', 'instances only: median / p90 / p99 ms']));
  for (const x of r.instantiate) {
    const a = x.compileAndInstantiate;
    const o = x.instantiateOnly;
    out.push(row([`\`${x.id}\``, x.moduleBytes, `${ms(a.median)} / ${ms(a.p90)} / ${ms(a.p99)}`, `${ms(o.median)} / ${ms(o.p90)} / ${ms(o.p99)}`]));
  }
  out.push('');

  out.push('## 4. Warm evaluation per call (compiled once) vs feelin');
  out.push('');
  out.push('Ours: the module is compiled and instantiated once; each call is `instance.evaluate(inputs)` (JS values → FEEL-JSON → Wasm → FEEL-JSON → JS values with exact decimals) or `instance.evaluateJson(text)` (FEEL-JSON text in and out). ');
  out.push(`feelin: its public \`evaluate(source, context)\` / \`unaryTest(source, context)\` API, which parses the source on every call (feelin ${e.feelin} has no compile-once API), with JS numbers as inputs. `);
  out.push('Before timing, both results are compared (numbers by decimal value); speed is compared only where they are equivalent.');
  out.push('');
  out.push(head(['Workload', 'group', 'input size', 'ours evaluate µs/call (median / p90)', 'ours evaluateJson µs/call (median)', 'feelin µs/call (median / p90)', 'feelin / ours (median)']));
  for (const x of r.evaluate.filter((x) => x.equivalent)) {
    out.push(
      row([
        `\`${x.source}\``,
        x.group,
        x.size,
        `${us(x.ours.perCallUs.median)} / ${us(x.ours.perCallUs.p90)}`,
        us(x.oursJson.perCallUs.median),
        `${us(x.feelin.perCallUs.median)} / ${us(x.feelin.perCallUs.p90)}`,
        `${(x.feelin.perCallUs.median / x.ours.perCallUs.median).toFixed(1)}×`,
      ]),
    );
  }
  out.push('');
  out.push(`Calls per measurement: ${r.parameters.eval[1]} batches after ${r.parameters.eval[0]} warmup calls (batch sizes per workload are in results.json).`);
  out.push('');
  const excluded = r.evaluate.filter((x) => !x.equivalent);
  out.push(`### Excluded from the speed comparison (${excluded.length})`);
  out.push('');
  if (excluded.length === 0) out.push('None.');
  else {
    out.push(head(['Workload', 'reason', 'ours evaluate µs/call (median)']));
    for (const x of excluded) out.push(row([`\`${x.source}\` (${x.size})`, x.excludedReason.replace(/\|/g, '\\|'), us(x.ours.perCallUs.median)]));
  }
  out.push('');

  out.push('## 5. One-shot: compile + instantiate + evaluate');
  out.push('');
  out.push(`For workloads evaluated only once, compilation is not amortized. Each sample compiles (no cache), instantiates and evaluates once. Warmup ${r.oneShot[0].warmup}, iterations ${r.oneShot[0].iterations}. The feelin column repeats its per-call time from section 4 (it parses on every call anyway).`);
  out.push('');
  out.push(head(['Workload', 'ours one-shot median / p90 ms', 'feelin per call median ms']));
  for (const x of r.oneShot) {
    const f = W[x.id];
    out.push(row([`\`${x.id}\``, `${ms(x.oneShotMs.median)} / ${ms(x.oneShotMs.p90)}`, f.feelin ? ms(f.feelin.perCallUs.median / 1000) : `n/a (${f.equivalent ? '' : 'not equivalent'})`]));
  }
  out.push('');

  out.push('## 6. JS ↔ Wasm transfer cost');
  out.push('');
  out.push('Identity expression `x` with `x` = a list of n decimals (`0.25, 1.25, …`), so the result is as large as the input, and `count(x)` (input only). Phase medians come from the instance profiler (`instance.profile = true`): *JS encode* = JS values → FEEL-JSON text (TypeScript); *decode* = copy text into Wasm memory + runtime FEEL-JSON parse; *evaluate* = the generated module\'s `evaluate()`; *encode* = runtime FEEL-JSON encode + UTF-8 decode into a JS string; *JS decode* = FEEL-JSON → JS values (TypeScript parser, exact decimals).');
  out.push('');
  out.push(head(['n', 'input chars', 'case', 'total median / p90 µs', 'JS encode', 'decode', 'bind', 'evaluate', 'encode', 'JS decode', 'iterations']));
  const u = (x) => (x * 1000).toFixed(0);
  for (const t of r.transfer) {
    for (const name of ['evaluateJson x', 'evaluate x (JS values)', 'evaluateJson count(x)']) {
      const c = t[name];
      const p = c.phasesMedianMs;
      out.push(row([t.size.toLocaleString('en-US'), t.inputChars.toLocaleString('en-US'), name, `${u(c.totalMs.median)} / ${u(c.totalMs.p90)}`, u(p.jsEncodeMs), u(p.decodeMs), u(p.bindMs), u(p.evaluateMs), u(p.encodeMs), u(p.jsDecodeMs), c.iterations]));
    }
  }
  out.push('');
  out.push(`Phase columns are medians in µs, each taken separately (so they need not add up to the total); the timer resolution is ${e.crossOriginIsolated ? '5' : '100'} µs, so single-digit values are at the resolution limit. Per-element cost at n = 100,000: see the evaluateJson rows (decode ≈ input parse, encode ≈ output serialization).`);
  out.push('');
  out.push('## Caveats');
  out.push('');
  out.push('- Headless Chromium on a single machine; no CPU pinning or frequency control. Repeat runs to gauge variance.');
  out.push('- feelin evaluates with binary floating point; equivalence was checked per workload and non-equivalent workloads are excluded from the speed comparison rather than compared.');
  out.push('- feelin is measured through its public API, which re-parses the expression on each call; a compile-once interpreter would be faster than shown. Section 5 shows our cost when compilation is not amortized.');
  out.push('- Cold-start fetch times are over localhost; real networks add download time for ~5 MB of uncompressed Wasm (compressible).');
  out.push('');
  return out.join('\n');
}
