/**
 * Demo page: compile FEEL in the browser (compiler.wasm), instantiate the
 * emitted module against runtime.wasm, and evaluate it against many input
 * rows without recompiling — on the main thread or in a cancellable worker.
 */
import {
  loadCompiler,
  loadRuntime,
  FeelWorker,
  parseFeelJson,
  byteToCharOffset,
  type CompileOptions,
  type CompileResult,
  type FeelCompiler,
  type FeelContext,
  type FeelInstance,
  type FeelRuntime,
  type FeelValue,
  type RawEvalResult,
  type WorkerProgram,
} from '../src/feel.js';

const $ = <T extends HTMLElement = HTMLElement>(id: string): T => document.getElementById(id) as T;

const EXAMPLES = {
  expression: {
    source: `{
  monthly rate: rate / 12,
  payment: decimal(principal * monthly rate / (1 - (1 + monthly rate) ** -term), 2),
  fees: sum(charges[amount > 0].amount),
  total cost: payment * term + fees,
  affordable: payment <= income * 0.35,
  decision: if affordable then "approve" else "refer"
}`,
    schema: `{
  "inputs": {
    "principal": "number", "rate": "number", "term": "number", "income": "number",
    "charges": "list<context<name: string, amount: number>>"
  }
}`,
    inputs: `[
  {"principal": 250000, "rate": 0.045, "term": 360, "income": 6500,
   "charges": [{"name": "origination", "amount": 1250.50}, {"name": "waiver", "amount": -200}]},
  {"principal": 18000, "rate": 0.069, "term": 60, "income": 2100.75,
   "charges": [{"name": "doc fee", "amount": 99.99}]},
  {"principal": 1000000, "rate": 0.0399, "term": 240, "income": 12000, "charges": []}
]`,
    hint: 'A JSON array of input contexts (FEEL-JSON: numbers keep their exact decimal text; dates as {"@date": "2024-01-31"}).',
  },
  unaryTests: {
    source: `[18..65), > 90`,
    schema: `{ "candidate": "number" }`,
    inputs: `[
  {"?": 17}, {"?": 18}, {"?": 64.999}, {"?": 65}, {"?": 90.5}, {"?": null}
]`,
    hint: 'A JSON array of rows; the "?" key is the candidate, other keys are inputs. A non-object row is the candidate itself.',
  },
} as const;

type Entry = keyof typeof EXAMPLES;

interface Compiled {
  result: CompileResult;
  source: string;
  options: CompileOptions;
  instances: Map<number, Promise<FeelInstance>>;
  workerProgram?: WorkerProgram;
}

/** Introspection hooks for the end-to-end test. */
const hooks = {
  ready: false,
  workerReady: false,
  compileCount: 0,
  evaluateCount: 0,
  lastEvaluateMode: '' as '' | 'main' | 'worker',
  workerRestarts: 0,
  error: '' as string,
  /** The loaded API objects (for in-page tests while offline). */
  api: {} as { compiler?: FeelCompiler; runtime?: FeelRuntime; worker?: FeelWorker },
};
(window as unknown as { __feelDemo: typeof hooks }).__feelDemo = hooks;

let compiler: FeelCompiler;
let runtime: FeelRuntime;
let worker: FeelWorker | undefined;
let workerPromise: Promise<FeelWorker> | undefined;
let compiled: Compiled | undefined;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & { className?: string } = {}, ...children: (Node | string)[]) {
  const e = document.createElement(tag);
  Object.assign(e, props);
  for (const c of children) e.append(c);
  return e;
}

function setStatus(id: string, text: string, cls: '' | 'ok' | 'err' = '') {
  const s = $(id);
  s.textContent = text;
  s.className = `status ${cls}`;
}

function fmtMs(ms: number): string {
  return ms < 1 ? `${(ms * 1000).toFixed(0)} µs` : `${ms.toFixed(2)} ms`;
}

function applyExample(entry: Entry) {
  const ex = EXAMPLES[entry];
  $<HTMLTextAreaElement>('source').value = ex.source;
  $<HTMLTextAreaElement>('schema').value = ex.schema;
  $<HTMLTextAreaElement>('inputs').value = ex.inputs;
  $('inputs-hint').textContent = ex.hint;
}

function currentOptions(): CompileOptions {
  const entry = $<HTMLSelectElement>('entry').value as Entry;
  const options: CompileOptions = { entry, check: $<HTMLInputElement>('strict').checked ? 'strict' : 'permissive' };
  const schemaText = $<HTMLTextAreaElement>('schema').value.trim();
  if ($<HTMLInputElement>('use-schema').checked && schemaText) {
    try {
      options.schema = JSON.parse(schemaText);
    } catch (e) {
      throw new Error(`schema is not valid JSON: ${(e as Error).message}`);
    }
  }
  return options;
}

function renderDiagnostics(source: string, result: CompileResult) {
  const ul = $('diagnostics');
  ul.replaceChildren();
  if (result.diagnostics.length === 0) {
    ul.append(el('li', { className: 'muted' }, 'no diagnostics'));
    return;
  }
  for (const d of result.diagnostics) {
    const li = el('li');
    li.dataset.code = d.code;
    li.dataset.severity = d.severity;
    const s = byteToCharOffset(source, d.span.start);
    const e = byteToCharOffset(source, d.span.end);
    const snippet = source.slice(s, Math.min(e, s + 60));
    li.append(
      el('span', { className: `sev ${d.severity}` }, d.severity),
      ' ',
      el('code', {}, d.code),
      ' ',
      el('span', { className: 'mono pos' }, `${d.line}:${d.column}`),
      ' ',
      d.message,
    );
    if (snippet) li.append(' ', el('code', { className: 'muted' }, `«${snippet}»`));
    if (d.suggestion) li.append(el('div', { className: 'suggestion' }, `suggestion: ${d.suggestion}`));
    ul.append(li);
  }
}

function renderCompile(source: string, result: CompileResult) {
  renderDiagnostics(source, result);
  const meta = result.metadata;
  $('result-type').textContent = meta?.resultType ?? (result.ok ? 'unknown (check off)' : '–');
  $('module-size').textContent = result.ok ? `${result.bytes.length.toLocaleString('en-US')} bytes` : '–';
  $('compile-time').textContent = fmtMs(result.compileMs);
  $('instantiate-time').textContent = '–';
  const imports = $('imports');
  imports.replaceChildren();
  imports.className = 'chips';
  if (meta) for (const name of meta.imports) imports.append(el('span', { className: 'chip' }, name));
  else imports.append(el('span', { className: 'muted' }, '–'));
  const deps = $('deps');
  deps.replaceChildren();
  deps.className = '';
  if (meta) {
    const d = meta.dependencies;
    const list = el('ul', { className: 'diags' });
    for (const i of d.inputs) {
      const paths = i.paths.length ? ` paths: ${i.paths.map((p) => p.join('.')).join(', ')}` : '';
      list.append(el('li', {}, el('code', {}, i.name), i.whole ? ' (whole value)' : '', paths));
    }
    if (d.inputs.length === 0) list.append(el('li', { className: 'muted' }, 'no inputs'));
    list.append(
      el('li', {}, `clock: ${d.clock ? 'yes' : 'no'} · host functions: ${d.hostFunctions.length ? d.hostFunctions.join(', ') : 'none'} · exact: ${d.exact}`),
    );
    for (const u of d.uncertain) list.append(el('li', { className: 'muted' }, `uncertain: ${u}`));
    deps.append(list);
  } else deps.append(el('span', { className: 'muted' }, '–'));
}

async function onCompile() {
  const source = $<HTMLTextAreaElement>('source').value;
  let options: CompileOptions;
  try {
    options = currentOptions();
  } catch (e) {
    setStatus('status', (e as Error).message, 'err');
    return;
  }
  setStatus('status', 'compiling…');
  const result = compiler.compile(source, options);
  renderCompile(source, result);
  $('results').replaceChildren();
  setStatus('eval-summary', '');
  if (!result.ok) {
    compiled = undefined;
    const n = result.diagnostics.filter((d) => d.severity === 'error').length;
    setStatus('status', `compilation failed (${n} error${n === 1 ? '' : 's'})`, 'err');
    hooks.compileCount++;
    return;
  }
  compiled = { result, source, options, instances: new Map() };
  const t0 = performance.now();
  await instanceFor(maxSteps());
  $('instantiate-time').textContent = fmtMs(performance.now() - t0);
  const warnings = result.diagnostics.length;
  setStatus('status', `compiled${warnings ? ` with ${warnings} warning${warnings === 1 ? '' : 's'}` : ''}`, 'ok');
  hooks.compileCount++;
}

function maxSteps(): number {
  const n = Number($<HTMLInputElement>('max-steps').value);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 50_000_000;
}

async function instanceFor(steps: number): Promise<FeelInstance> {
  const c = compiled!;
  let inst = c.instances.get(steps);
  if (!inst) {
    inst = runtime.instantiate(c.result, { limits: { maxSteps: steps } });
    c.instances.set(steps, inst);
  }
  return inst;
}

/** Split a row into (inputs, candidate) according to the entry kind. */
function splitRow(row: FeelValue, unary: boolean): { inputs: FeelContext; candidate?: FeelValue } {
  const isCtx = row instanceof Map;
  if (!unary) {
    if (!isCtx) throw new Error('each row must be a JSON object of inputs');
    return { inputs: row as Map<string, FeelValue> };
  }
  if (isCtx && (row as Map<string, FeelValue>).has('?')) {
    const m = new Map(row as Map<string, FeelValue>);
    const candidate = m.get('?');
    m.delete('?');
    return { inputs: m, candidate };
  }
  return { inputs: new Map(), candidate: row };
}

function renderRow(i: number, r: RawEvalResult, ms: number, mode: string) {
  const li = el('li');
  li.dataset.row = String(i);
  li.dataset.ok = String(r.ok);
  li.append(el('div', { className: 'head' }, el('span', {}, `#${i + 1}`), el('span', { className: 'time' }, `${fmtMs(ms)} · ${mode}`)));
  if (r.valueJson !== null) li.append(el('pre', { className: 'value' }, r.valueJson));
  if (r.abort) {
    const a = el('p', { className: 'abort' }, `${r.abort.kind} abort `, el('code', {}, r.abort.code), `: ${r.abort.message}`);
    a.dataset.kind = r.abort.kind;
    a.dataset.code = r.abort.code;
    li.append(a);
  }
  if (r.diagnostics.length) {
    const ul = el('ul', { className: 'eval-diags' });
    for (const d of r.diagnostics) {
      const item = el('li', {}, el('code', {}, d.code), ` ${d.message}`);
      item.dataset.code = d.code;
      ul.append(item);
    }
    li.append(ul);
  }
  $('results').append(li);
}

async function onEvaluateAll() {
  if (!compiled) {
    setStatus('eval-summary', 'compile first', 'err');
    return;
  }
  let rows: FeelValue;
  try {
    rows = parseFeelJson($<HTMLTextAreaElement>('inputs').value, { contexts: 'map' });
  } catch (e) {
    setStatus('eval-summary', `inputs: ${(e as Error).message}`, 'err');
    return;
  }
  const list = Array.isArray(rows) ? rows : [rows];
  const unary = compiled.result.metadata?.entry === 'unaryTests';
  const useWorker = $<HTMLInputElement>('use-worker').checked;
  const steps = maxSteps();
  const timeoutMs = Math.max(1, Number($<HTMLInputElement>('timeout').value) || 1000);
  $('results').replaceChildren();
  const button = $<HTMLButtonElement>('evaluate-all');
  button.disabled = true;
  setStatus('eval-summary', `evaluating ${list.length} row(s)${useWorker ? ' in the worker' : ''}…`);
  const t0 = performance.now();
  let failed = 0;
  try {
    if (useWorker) {
      const w = worker ?? (await workerPromise!);
      compiled.workerProgram ??= w.load(compiled.result, compiled.source);
      for (let i = 0; i < list.length; i++) {
        const { inputs, candidate } = splitRow(list[i], unary);
        const s = performance.now();
        const r = await w.evaluate(compiled.workerProgram, inputs, { timeoutMs, candidate, limits: { maxSteps: steps } });
        renderRow(i, r, performance.now() - s, 'worker');
        if (!r.ok) failed++;
      }
      hooks.workerRestarts = w.restarts;
      hooks.lastEvaluateMode = 'worker';
    } else {
      const inst = await instanceFor(steps);
      for (let i = 0; i < list.length; i++) {
        const { inputs, candidate } = splitRow(list[i], unary);
        const s = performance.now();
        const r = candidate !== undefined ? inst.evaluateUnary(candidate, inputs) : inst.evaluate(inputs);
        renderRow(i, r, performance.now() - s, 'main thread');
        if (!r.ok) failed++;
      }
      hooks.lastEvaluateMode = 'main';
    }
    hooks.evaluateCount += list.length;
    const total = performance.now() - t0;
    const restarts = useWorker && worker ? ` · worker restarts: ${worker.restarts}` : '';
    setStatus(
      'eval-summary',
      `${list.length} row(s) in ${fmtMs(total)} using the module compiled once (no recompilation)${failed ? ` · ${failed} aborted` : ''}${restarts}`,
      failed ? 'err' : 'ok',
    );
  } catch (e) {
    setStatus('eval-summary', (e as Error).message, 'err');
  } finally {
    button.disabled = false;
  }
}

async function main() {
  applyExample('expression');
  const entrySel = $<HTMLSelectElement>('entry');
  entrySel.addEventListener('change', () => {
    const entry = entrySel.value as Entry;
    const other: Entry = entry === 'expression' ? 'unaryTests' : 'expression';
    // switch examples only if the user has not edited the other example
    if ($<HTMLTextAreaElement>('source').value === EXAMPLES[other].source) applyExample(entry);
    else $('inputs-hint').textContent = EXAMPLES[entry].hint;
  });
  $<HTMLTextAreaElement>('source').addEventListener('input', () => {
    if (compiled && $<HTMLTextAreaElement>('source').value !== compiled.source) {
      setStatus('status', 'source edited since the last compile — Evaluate all still uses the compiled module');
    }
  });
  $('compile').addEventListener('click', () => void onCompile());
  $('evaluate-all').addEventListener('click', () => void onEvaluateAll());

  const t0 = performance.now();
  try {
    [compiler, runtime] = await Promise.all([loadCompiler(), loadRuntime()]);
  } catch (e) {
    hooks.error = String(e);
    setStatus('load-status', `failed to load: ${(e as Error).message}`, 'err');
    return;
  }
  const v = compiler.version;
  setStatus('load-status', `Ready: compiler.wasm + runtime.wasm loaded in ${fmtMs(performance.now() - t0)}`, 'ok');
  $('versions').textContent = `compiler ${v.compiler} · runtime ${v.runtime} · ABI ${v.abiModule} v${v.abiVersion} · ${v.dialect} · tzdb ${v.tzdb}`;
  $<HTMLButtonElement>('compile').disabled = false;
  $<HTMLButtonElement>('evaluate-all').disabled = false;
  hooks.api.compiler = compiler;
  hooks.api.runtime = runtime;
  hooks.ready = true;

  // Start the worker now so its script is fetched while online; restarts reuse a blob: URL.
  workerPromise = FeelWorker.create({ compiler: compiler.module, runtime: runtime.module });
  workerPromise.then(
    (w) => {
      worker = w;
      hooks.api.worker = w;
      hooks.workerReady = true;
    },
    (e) => {
      hooks.error = `worker: ${String(e)}`;
      $<HTMLInputElement>('use-worker').disabled = true;
    },
  );
}

void main();
