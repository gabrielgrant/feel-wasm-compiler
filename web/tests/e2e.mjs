// End-to-end test of the browser build in Chromium (Playwright).
//
//   node web/tests/e2e.mjs        (or: cd web && npm test)
//
// Requires a prior `scripts/build-web.sh`. Starts a static server for web/,
// opens the demo, waits for compiler.wasm/runtime.wasm/worker to load, then
// goes OFFLINE and checks that everything else happens in the browser:
// compiling new source, evaluating many rows without recompiling, syntax
// diagnostics, unary tests, worker cancellation + recovery, step-limit
// aborts, instance isolation, bounded memory over repeated evaluations, and
// every semantic vector from tests/vectors/semantic.json.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from '../serve.mjs';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const VECTORS = path.join(REPO, 'tests/vectors/semantic.json');

const failures = [];
let checks = 0;
function check(cond, message) {
  checks++;
  if (!cond) {
    failures.push(message);
    console.log(`  FAIL ${message}`);
  }
}
function section(name) {
  console.log(`\n== ${name}`);
}

for (const f of ['dist/compiler.wasm', 'dist/runtime.wasm', 'dist/src/feel.js', 'dist/demo/demo.js', 'dist/worker.bundle.js']) {
  if (!fs.existsSync(path.join(HERE, '..', f))) {
    console.error(`missing web/${f}: run scripts/build-web.sh first`);
    process.exit(2);
  }
}
if (!fs.existsSync(VECTORS)) {
  console.error(`missing ${VECTORS}`);
  process.exit(2);
}

const server = createServer();
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', (e) => pageErrors.push(e.message));
page.on('console', (m) => {
  // the deliberate offline probe logs ERR_INTERNET_DISCONNECTED; anything else is an error
  if (m.type() === 'error' && !(offline && /ERR_INTERNET_DISCONNECTED/.test(m.text()))) pageErrors.push(`console: ${m.text()}`);
});

let offline = false;
const finishedOffline = [];
const failedOffline = [];
page.on('requestfinished', (r) => {
  if (offline && /^https?:/.test(r.url())) finishedOffline.push(r.url());
});
page.on('requestfailed', (r) => {
  if (offline) failedOffline.push(r.url());
});
context.on('request', (r) => {
  if (offline && /^https?:/.test(r.url()) && !r.url().startsWith(base)) finishedOffline.push(`(cross-origin) ${r.url()}`);
});

const demo = () => page.evaluate(() => ({ ...window.__feelDemo, api: undefined }));

async function setSource(text, { entry = 'expression', strict = false, schema = null } = {}) {
  await page.selectOption('#entry', entry);
  await page.fill('#source', text);
  await page.setChecked('#strict', strict);
  await page.setChecked('#use-schema', schema !== null);
  if (schema !== null) {
    if (!(await page.evaluate(() => document.querySelector('#schema-box').open))) await page.click('#schema-box summary');
    await page.fill('#schema', JSON.stringify(schema));
  }
}
async function compile() {
  const before = (await demo()).compileCount;
  await page.click('#compile');
  // compileCount is bumped after compilation *and* instantiation finished
  await page.waitForFunction((n) => window.__feelDemo.compileCount > n, before);
  return page.evaluate(() => ({
    status: document.querySelector('#status').textContent,
    ok: document.querySelector('#status').classList.contains('ok'),
    resultType: document.querySelector('#result-type').textContent,
    diags: [...document.querySelectorAll('#diagnostics li[data-code]')].map((li) => ({
      code: li.dataset.code,
      severity: li.dataset.severity,
      text: li.textContent,
    })),
    imports: [...document.querySelectorAll('#imports .chip')].map((c) => c.textContent),
    size: document.querySelector('#module-size').textContent,
  }));
}
async function evaluateAll(rowsJson) {
  await page.fill('#inputs', rowsJson);
  const before = await page.evaluate(() => window.__feelDemo.evaluateCount);
  await page.click('#evaluate-all');
  await page.waitForFunction(
    (n) => window.__feelDemo.evaluateCount > n || document.querySelector('#eval-summary').classList.contains('err'),
    before,
    { timeout: 60_000 },
  );
  return page.evaluate(() =>
    [...document.querySelectorAll('#results > li')].map((li) => ({
      value: li.querySelector('pre.value')?.textContent ?? null,
      abort: li.querySelector('.abort') ? { kind: li.querySelector('.abort').dataset.kind, code: li.querySelector('.abort').dataset.code } : null,
      diags: [...li.querySelectorAll('.eval-diags li')].map((d) => d.dataset.code),
      time: li.querySelector('.time')?.textContent,
    })),
  );
}

try {
  section('load demo, then go offline');
  await page.goto(`${base}/demo/`);
  await page.waitForFunction(() => window.__feelDemo && window.__feelDemo.ready && window.__feelDemo.workerReady, null, { timeout: 60_000 });
  const loaded = await demo();
  check(loaded.error === '', `demo loaded without error (${loaded.error})`);
  await context.setOffline(true);
  offline = true;
  const probe = await page.evaluate(() =>
    fetch(`/dist/compiler.wasm?offline-probe=${Date.now()}`, { cache: 'no-store' }).then(
      () => 'succeeded',
      () => 'failed',
    ),
  );
  check(probe === 'failed', `network is offline (probe fetch ${probe})`);
  console.log(`  offline; probe fetch ${probe}`);

  section('(a)+(b) compile new source in the browser; evaluate many rows without recompiling');
  {
    const src = '(a + b) / c';
    const loadedSource = await page.inputValue('#source');
    check(loadedSource !== src, 'test source was not present at load');
    await setSource(src, { strict: true, schema: { inputs: { a: 'number', b: 'number', c: 'number' } } });
    const c = await compile();
    check(c.ok, `compiled: ${c.status}`);
    check(/^number\??$/.test(c.resultType), `inferred result type is number (got ${c.resultType})`);
    check(c.imports.includes('div') && c.imports.includes('add'), `imports listed (${c.imports.join(',')})`);
    check(/bytes/.test(c.size), `module size shown (${c.size})`);
    const compiles = (await demo()).compileCount;
    const rows = [
      ['{"a": 0.1, "b": 0.2, "c": 1}', '0.3'],
      ['{"a": 12345678901234567890123456789, "b": 1, "c": 1}', '12345678901234567890123456790'],
      ['{"a": 1, "b": 0, "c": 3}', '0.3333333333333333333333333333333333'],
      ['{"a": 1.10, "b": 0, "c": 1}', '1.10'],
      ['{"a": 2, "b": 0, "c": 0}', 'null'],
    ];
    const res = await evaluateAll(`[${rows.map((r) => r[0]).join(',\n')}]`);
    check(res.length === rows.length, `${rows.length} result rows (got ${res.length})`);
    rows.forEach(([input, expect], i) => check(res[i]?.value === expect, `row ${i} ${input} = ${expect} (got ${res[i]?.value})`));
    check(res[4]?.diags.includes('FEEL-E305'), `division by zero diagnostic (got ${res[4]?.diags})`);
    const res2 = await evaluateAll('[{"a": 100, "b": 0.001, "c": 10}, {"a": -7, "b": 7, "c": 5}]');
    check(res2[0]?.value === '10.0001' && res2[1]?.value === '0', `second batch exact (got ${res2.map((r) => r.value)})`);
    const after = await demo();
    check(after.compileCount === compiles, `no recompilation while evaluating (compiles ${compiles} -> ${after.compileCount})`);
    check(after.lastEvaluateMode === 'main', 'evaluated on the main thread');
    console.log(`  ${res.length + res2.length} rows evaluated with one compilation; e.g. 1/3 = ${res[2]?.value}`);
  }

  section('(c) syntax error diagnostic');
  {
    await setSource('if x > 1 then "big" else (2 *');
    const c = await compile();
    check(!c.ok, 'compilation failed');
    const syn = c.diags.find((d) => /^FEEL-S\d{3}$/.test(d.code));
    check(!!syn, `syntax diagnostic with code (${JSON.stringify(c.diags)})`);
    check(syn && /\d+:\d+/.test(syn.text), 'diagnostic shows line:col');
    console.log(`  ${syn?.code}: ${syn?.text}`);
    await setSource('agee >= 18', { strict: true, schema: { closed: true, inputs: { age: 'number' } } });
    const c2 = await compile();
    const sugg = await page.evaluate(() => document.querySelector('#diagnostics .suggestion')?.textContent ?? '');
    check(!c2.ok && c2.diags.some((d) => d.code === 'FEEL-C101' && d.severity === 'error'), `strict checker error (${JSON.stringify(c2.diags)})`);
    check(/did you mean 'age'/.test(sugg), `suggestion shown (${sugg})`);
    console.log(`  strict: ${c2.diags.map((d) => d.code)} ${sugg}`);
  }

  section('(d) unary tests with candidates');
  {
    await setSource('< 1000, [2000..3000], 4242.5', { entry: 'unaryTests' });
    const c = await compile();
    check(c.ok, `unary tests compiled (${c.status})`);
    const res = await evaluateAll('[{"?": 999.99}, {"?": 1000}, {"?": 2500}, {"?": 4242.50}, {"?": 5000}, {"?": null}]');
    const got = res.map((r) => r.value);
    const want = ['true', 'false', 'true', 'true', 'false', 'null'];
    check(JSON.stringify(got) === JSON.stringify(want), `unary results ${JSON.stringify(got)} == ${JSON.stringify(want)}`);
    await setSource('"A", "B", "C"', { entry: 'unaryTests' });
    await compile();
    const res2 = await evaluateAll('["A", "C", "D", {"?": "B"}]');
    check(JSON.stringify(res2.map((r) => r.value)) === '["true","true","false","true"]', `string disjunction ${JSON.stringify(res2.map((r) => r.value))}`);
  }

  const LONG = '{L: for i in 1..n return i, r: some x in L, y in L satisfies x < y and y < 0}.r';

  section('(e) worker cancellation by timeout, then recovery');
  {
    await setSource(LONG);
    const c = await compile();
    check(c.ok, 'long-running expression compiled');
    await page.setChecked('#use-worker', true);
    await page.fill('#timeout', '500');
    await page.fill('#max-steps', '1000000000000000');
    const t0 = Date.now();
    const res = await evaluateAll('[{"n": 30000}]');
    const elapsed = Date.now() - t0;
    check(res[0]?.abort?.kind === 'cancelled', `cancelled abort (got ${JSON.stringify(res[0])})`);
    check(elapsed < 10_000, `cancellation was prompt (${elapsed} ms)`);
    const d1 = await demo();
    check(d1.workerRestarts >= 1, `worker restarted (${d1.workerRestarts})`);
    console.log(`  cancelled after ${elapsed} ms wall (timeout 500 ms); restarts=${d1.workerRestarts}`);
    const res2 = await evaluateAll('[{"n": 200}, {"n": 0}]');
    check(res2[0]?.value === 'false' && res2[1]?.value === 'false', `worker recovered: same module evaluates (${JSON.stringify(res2)})`);
    await setSource('string length("recovered") + 0.5');
    await compile();
    const res3 = await evaluateAll('[{}]');
    check(res3[0]?.value === '9.5', `worker evaluates another expression after restart (got ${res3[0]?.value})`);
    check((await demo()).lastEvaluateMode === 'worker', 'evaluated in the worker');
    // also via the API: a request queued behind the cancelled one is re-dispatched
    const api = await page.evaluate(async (src) => {
      const w = window.__feelDemo.api.worker;
      const p = await w.compile(src);
      const slow = w.evaluate(p.program, { n: 30000 }, { timeoutMs: 300, limits: { maxSteps: 1e15 } });
      const queued = w.evaluate('1 + 1', {}, { timeoutMs: 20000 });
      const [a, b] = await Promise.all([slow, queued]);
      return { a: a.abort?.kind, b: b.valueJson, restarts: w.restarts };
    }, LONG);
    check(api.a === 'cancelled' && api.b === '2', `API: cancelled + queued request served after restart (${JSON.stringify(api)})`);
    await page.setChecked('#use-worker', false);
  }

  section('(f) step limit with default limits is a limit abort, not null');
  {
    await page.fill('#max-steps', '50000000');
    await setSource(LONG);
    await compile();
    const res = await evaluateAll('[{"n": 30000}]');
    check(res[0]?.abort?.kind === 'limit' && res[0]?.abort?.code === 'FEEL-L404', `FEEL-L404 limit abort (got ${JSON.stringify(res[0])})`);
    check(res[0]?.value === null, 'no value for an aborted evaluation');
    await setSource('count(for i in 1..1000000, j in 1..1000 return i)');
    await compile();
    const res2 = await evaluateAll('[{}]');
    check(res2[0]?.abort?.kind === 'limit' && /^FEEL-L4\d\d$/.test(res2[0]?.abort?.code), `count(for ...) aborts with a limit (got ${JSON.stringify(res2[0])})`);
    console.log(`  ${res[0]?.abort?.code}; count(for i in 1..1000000, j in 1..1000 return i) -> ${res2[0]?.abort?.code}`);
    // the instance keeps working after the abort
    const res3 = await evaluateAll('[{}]');
    check(res3[0]?.abort?.code === res2[0]?.abort?.code, 'same abort on re-evaluation (instance recycled, still usable)');
  }

  section('isolation, memory bound, abort recycling (in-page API)');
  {
    const r = await page.evaluate(async () => {
      const { compiler, runtime } = window.__feelDemo.api;
      const out = {};
      const m = compiler.compile('{now: string(now()), sum: sum(for i in 1..n return i), host: tag()}', {
        schema: { functions: [{ name: 'tag', params: [], returns: 'string' }] },
      });
      const A = await runtime.instantiate(m, { clock: '2020-01-01T00:00:00Z', hostFunctions: { tag: () => 'A' } });
      const B = await runtime.instantiate(m, { clock: '2030-06-15T12:00:00Z', limits: { maxSteps: 100 }, hostFunctions: { tag: () => 'B' } });
      const a1 = A.evaluate({ n: 10 });
      const aVals = A.evalValues();
      const b1 = B.evaluate({ n: 1000 }); // exceeds B's step limit
      const b2 = B.evaluate({ n: 5 });
      const a2 = A.evaluate({ n: 10 });
      out.a1 = a1.valueJson;
      out.a2 = a2.valueJson;
      out.aValsStable = aVals === A.evalValues();
      out.b1 = b1.abort?.code;
      out.b2 = b2.valueJson;
      // repeated evaluations: bounded live values and memory
      const L = compiler.compile('for x in xs return x * 2 + 0.5');
      const inst = await runtime.instantiate(L);
      const xs = Array.from({ length: 50 }, (_, i) => i);
      inst.evaluate({ xs });
      const first = inst.evalValues();
      let max = 0;
      let mem100 = 0;
      for (let i = 0; i < 2000; i++) {
        const res = inst.evaluate({ xs });
        if (!res.ok) throw new Error('evaluation failed');
        max = Math.max(max, inst.evalValues());
        if (i === 100) mem100 = inst.memoryBytes;
      }
      out.leak = { first, max, last: inst.evalValues(), mem100, mem2000: inst.memoryBytes };
      // many aborts inside nested runtime frames must not wear the instance out
      const S = compiler.compile('sort(xs, function(a, b) a < b)');
      const s = await runtime.instantiate(S, { limits: { maxSteps: 30 } });
      const ys = Array.from({ length: 40 }, (_, i) => (i * 7919) % 40);
      let aborted = 0;
      const t0 = performance.now();
      for (let i = 0; i < 6000; i++) if (s.evaluate({ xs: ys }).abort?.code === 'FEEL-L404') aborted++;
      out.aborts = {
        aborted,
        recycles: s.recycles,
        ms: Math.round(performance.now() - t0),
        after: s.evaluate({ xs: ys }).abort?.code,
        small: s.evaluate({ xs: [3, 1, 2] }).valueJson,
      };
      const s2 = await runtime.instantiate(S);
      out.sorted = s2.evaluate({ xs: [3, 1, 2] }).valueJson;
      // batch evaluation and the compiled-module cache
      const c1 = compiler.compileCached('x * 2', { check: 'strict', schema: { inputs: { x: 'number' } } });
      const c2 = compiler.compileCached('x * 2', { schema: { inputs: { x: 'number' } }, check: 'strict' });
      out.cacheSame = c1 === c2 && compiler.cacheStats.hits >= 1 && /^[0-9a-f]{64}$/.test(c1.cacheKey);
      const bi = await runtime.instantiate(c1);
      out.batch = bi.evaluateBatch([{ x: 1 }, { x: '2' }, { x: 2.5 }]).map((r) => r.valueJson ?? r.abort?.code);
      // deep recursion beyond the engine stack is a boundary abort, and the instance recovers
      const R = compiler.compile('{f: function(n) if n = 0 then 0 else 1 + f(n - 1), r: f(depth)}.r');
      const ri = await runtime.instantiate(R, { limits: { maxDepth: 1000000 } });
      out.deep = [ri.evaluate({ depth: 50000 }).abort?.code, ri.evaluate({ depth: 10 }).valueJson];
      return out;
    });
    check(r.a1 === r.a2 && r.a1 === '{"now":"2020-01-01T00:00:00Z","sum":55,"host":"A"}', `instance A unaffected by B (${r.a1} / ${r.a2})`);
    check(r.aValsStable, "B's evaluations do not touch A's live values");
    check(r.b1 === 'FEEL-L404', `B aborts on its own step limit (${r.b1})`);
    check(r.b2 === '{"now":"2030-06-15T12:00:00Z","sum":15,"host":"B"}', `B uses its own clock/host function (${r.b2})`);
    check(r.leak.max <= r.leak.first && r.leak.last === r.leak.first, `live values bounded over 2000 evaluations (${JSON.stringify(r.leak)})`);
    check(r.leak.mem2000 === r.leak.mem100, `linear memory stable over evaluations (${r.leak.mem100} -> ${r.leak.mem2000})`);
    check(
      r.aborts.aborted === 6000 && r.aborts.after === 'FEEL-L404' && r.aborts.small === '[1,2,3]',
      `6000 aborts handled, instance still usable (${JSON.stringify(r.aborts)})`,
    );
    check(r.sorted === '[1,2,3]', 'sort works on a fresh instance');
    check(r.cacheSame, 'compileCached returns the cached result for equivalent options');
    check(JSON.stringify(r.batch) === '["2","FEEL-B506","5.0"]', `evaluateBatch with boundary validation (${JSON.stringify(r.batch)})`);
    check(r.deep[0] === 'FEEL-B503' && r.deep[1] === '10', `engine stack overflow reported and recovered (${JSON.stringify(r.deep)})`);
    console.log(`  live values: first=${r.leak.first} max=${r.leak.max}; memory ${r.leak.mem100} -> ${r.leak.mem2000} bytes`);
    console.log(`  6000 nested-frame aborts: ${r.aborts.recycles} runtime recycles in ${r.aborts.ms} ms`);
  }

  section('precision-preserving FEEL-JSON codec and value round trips');
  {
    const r = await page.evaluate(async () => {
      const feel = await import('/dist/src/feel.js');
      const { compiler, runtime } = window.__feelDemo.api;
      const text =
        '{"n":12345678901234567890.123456789,"s":"a\\"b\u00e9\\n","d":{"@date":"2020-02-29"},"t":{"@time":"10:30:00@Europe/Paris"},' +
        '"dt":{"@dateTime":"2020-01-01T10:00:00+01:00"},"du":{"@duration":"P1Y2M"},"r":{"@range":{"start":1,"end":10,"startIncluded":true,"endIncluded":false}},' +
        '"l":[1.10,null,true,[]],"__proto__":{"k":1}}';
      const v = feel.parseFeelJson(text, { contexts: 'map' });
      const again = feel.encodeFeelJson(v);
      const inst = await runtime.instantiate(compiler.compile('v'));
      const echoed = inst.evaluate({ v }).valueJson;
      // a context whose only key starts with '@' is escaped as {"@context": …}
      const escText = '{"@context":{"@x":1}}';
      const escTs = feel.encodeFeelJson(feel.parseFeelJson(escText, { contexts: 'map' }));
      const escOut = (await runtime.instantiate(compiler.compile('{"@x": 1}'))).evaluate({}).valueJson;
      const escIn = inst.evaluateJson(`{"v":${escText}}`);
      const asObject = feel.parseFeelJson(text);
      const typed = inst.evaluate({
        v: [new feel.FeelDecimal('0.1'), 0.1, 10n ** 30n, new Date(Date.UTC(2024, 0, 2, 3, 4, 5)), new feel.FeelDuration('PT1H'), new feel.FeelRange(undefined, new feel.FeelDecimal('5'), false, true)],
      }).valueJson;
      let nanError = '';
      try {
        feel.encodeFeelJson(Number.NaN);
      } catch (e) {
        nanError = e.name;
      }
      return { text, again, echoed, escText, escTs, escOut, escIn, protoOwn: Object.prototype.hasOwnProperty.call(asObject, '__proto__'), typed, nanError, decimalClass: v.get('n') instanceof feel.FeelDecimal };
    });
    check(r.again === r.text, `codec round trip is exact (${r.again})`);
    check(r.echoed === r.text, `runtime round trip is exact (${r.echoed})`);
    check(r.protoOwn, '"__proto__" key decoded as an own property');
    check(r.decimalClass, 'numbers decode to FeelDecimal');
    check(
      r.typed === '[0.1,0.1,1000000000000000000000000000000,{"@dateTime":"2024-01-02T03:04:05Z"},{"@duration":"PT1H"},{"@range":{"end":5,"startIncluded":false,"endIncluded":true}}]',
      `JS input types encoded as documented (${r.typed})`,
    );
    check(r.nanError === 'RangeError', 'NaN is rejected');
    check(r.escTs === r.escText && r.escOut === r.escText, `@context escape: TS codec and runtime encoder agree (${r.escTs} / ${r.escOut})`);
    check(r.escIn.ok && r.escIn.valueJson === r.escText, `runtime decodes its own @context escape (${r.escIn.valueJson ?? r.escIn.abort?.message})`);
  }

  section('(g) semantic vectors in the browser (compiler.wasm + runtime.wasm + generated modules)');
  {
    const text = fs.readFileSync(VECTORS, 'utf8');
    const summary = await page.evaluate(async (text) => {
      const feel = await import('/dist/src/feel.js');
      const { compiler, runtime } = window.__feelDemo.api;
      const plain = JSON.parse(text); // ids, expressions, options, expectations
      const exact = feel.parseFeelJson(text, { contexts: 'map' }); // inputs/candidates with exact numbers
      const results = [];
      for (let i = 0; i < plain.length; i++) {
        const v = plain[i];
        const vx = exact[i];
        const outcomes = [];
        for (const optimize of [true, false]) {
          const options = { ...(v.options ?? {}), optimize };
          if (v.entry === 'unaryTests') options.entry = 'unaryTests';
          const c = compiler.compile(v.expr, options);
          if (!c.ok) {
            outcomes.push({ compileError: c.diagnostics.map((d) => d.code) });
            continue;
          }
          const limits = {};
          for (const k of ['maxSteps', 'maxDepth', 'maxValues', 'maxCollection']) if (v.limits && k in v.limits) limits[k] = v.limits[k];
          const inst = await runtime.instantiate(c, { limits, clock: v.clock ?? '2024-01-01T10:00:00Z' });
          const inputsJson = vx.has('inputs') ? feel.encodeFeelJson(vx.get('inputs')) : '{}';
          const r =
            options.entry === 'unaryTests'
              ? inst.evaluateJson(inputsJson, vx.has('candidate') ? feel.encodeFeelJson(vx.get('candidate')) : 'null')
              : inst.evaluateJson(inputsJson);
          outcomes.push({ result: r.valueJson, abort: r.abort?.code ?? null, diags: r.diagnostics.map((d) => d.code) });
        }
        const check = (o) => {
          if (v.expectCompileError) return o.compileError?.includes(v.expectCompileError) ? null : `expected compile error ${v.expectCompileError}, got ${JSON.stringify(o)}`;
          if (o.compileError) return `unexpected compile error ${o.compileError}`;
          if (v.expectAbort) return o.abort === v.expectAbort ? null : `expected abort ${v.expectAbort}, got ${JSON.stringify(o)}`;
          if (o.result !== v.expect) return `expected ${v.expect}, got ${o.result} (abort ${o.abort}, diags ${o.diags})`;
          for (const d of v.expectDiag ?? []) if (!o.diags.includes(d)) return `expected diagnostic ${d}, got ${o.diags}`;
          return null;
        };
        const errs = [];
        const e1 = check(outcomes[0]);
        const e2 = check(outcomes[1]);
        if (e1) errs.push(`[opt] ${e1}`);
        if (e2) errs.push(`[no-opt] ${e2}`);
        if (JSON.stringify(outcomes[0]) !== JSON.stringify(outcomes[1])) errs.push(`[differential] ${JSON.stringify(outcomes)}`);
        results.push({ id: v.id, clause: v.clause, errs });
      }
      return results;
    }, text);
    const failed = summary.filter((r) => r.errs.length);
    for (const f of failed) check(false, `vector ${f.id} (${f.clause}): ${f.errs.join('; ')}`);
    const counted = summary.length - failed.length;
    checks += counted;
    console.log(`  semantic vectors: ${counted}/${summary.length} passed (each compiled optimized + unoptimized in the browser)`);
  }

  section('offline / page health');
  check(finishedOffline.length === 0, `no network request succeeded while offline (${finishedOffline.join(', ')})`);
  console.log(`  requests attempted while offline: ${failedOffline.length} (all failed), succeeded: ${finishedOffline.length}`);
  check(pageErrors.length === 0, `no page errors (${pageErrors.join(' | ')})`);
} catch (e) {
  failures.push(`exception: ${e?.stack ?? e}`);
  console.log(e);
} finally {
  await browser.close();
  server.close();
}

console.log(`\n${checks - failures.length}/${checks} checks passed`);
if (failures.length) {
  console.log(`${failures.length} FAILED:`);
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
