/**
 * In-page benchmark functions, driven by `bench/run.mjs` through Playwright.
 * Every function returns plain JSON. Timings use `performance.now()`
 * (5 µs resolution under cross-origin isolation, which serve.mjs enables).
 */
import {
  loadCompiler,
  loadRuntime,
  encodeFeelJson,
  parseFeelJson,
  FeelDecimal,
  type CompileOptions,
  type FeelCompiler,
  type FeelContext,
  type FeelInstance,
  type FeelRuntime,
  type FeelValue,
} from '../src/feel.js';

interface Feelin {
  evaluate(expr: string, ctx?: Record<string, unknown>): { value: unknown; warnings: unknown[] };
  unaryTest(expr: string, ctx?: Record<string, unknown>): { value: unknown; warnings: unknown[] };
  FEELIN_VERSION: string;
}

export interface Stats {
  n: number;
  min: number;
  median: number;
  p90: number;
  p99: number;
  max: number;
  mean: number;
}

/** Nearest-rank percentiles. */
export function stats(samples: number[]): Stats {
  const s = [...samples].sort((a, b) => a - b);
  const q = (p: number) => s[Math.min(s.length - 1, Math.max(0, Math.ceil(p * s.length) - 1))];
  return { n: s.length, min: s[0], median: q(0.5), p90: q(0.9), p99: q(0.99), max: s[s.length - 1], mean: s.reduce((a, b) => a + b, 0) / s.length };
}

const now = () => performance.now();

/** A benchmark workload. Inputs are FEEL-JSON text (exact numbers). */
export interface Workload {
  id: string;
  group: 'scalar predicates' | 'decimal formulas' | 'list transformations' | 'decision-table conditions';
  source: string;
  entry?: 'expression' | 'unaryTests';
  inputs: string;
  /** FEEL-JSON candidate for unary tests. */
  candidate?: string;
  /** Human description of the input size. */
  size: string;
}

function items(n: number, decimals: boolean): string {
  const a = [];
  for (let i = 0; i < n; i++) {
    const price = decimals ? `${(i * 37) % 50}.${String((i * 13) % 100).padStart(2, '0')}` : String((i * 37) % 50);
    a.push(`{"name":"item${i}","price":${price},"qty":${(i % 5) + 1}}`);
  }
  return `[${a.join(',')}]`;
}
function ints(n: number, shuffle: boolean): string {
  const a = [];
  for (let i = 0; i < n; i++) a.push(shuffle ? (i * 7919) % n : i);
  return `[${a.join(',')}]`;
}

export const WORKLOADS: Workload[] = [
  {
    id: 'predicate',
    group: 'scalar predicates',
    source: 'age >= 18 and income > 50000 and status in ("A","B")',
    inputs: '{"age": 34, "income": 72000, "status": "B"}',
    size: '3 scalar inputs',
  },
  {
    id: 'predicate-if',
    group: 'scalar predicates',
    source: 'if score >= 700 then "low" else if score >= 600 then "medium" else "high"',
    inputs: '{"score": 640}',
    size: '1 scalar input',
  },
  {
    id: 'loan-exact',
    group: 'decimal formulas',
    source: '(principal * rate / 12) / (1 - (1 + rate / 12) ** -term)',
    inputs: '{"principal": 250000, "rate": 0.045, "term": 360}',
    size: '3 scalar inputs',
  },
  {
    id: 'loan-rounded',
    group: 'decimal formulas',
    source: 'decimal((principal * rate / 12) / (1 - (1 + rate / 12) ** -term), 2)',
    inputs: '{"principal": 250000, "rate": 0.045, "term": 360}',
    size: '3 scalar inputs',
  },
  {
    id: 'sum-decimal',
    group: 'decimal formulas',
    source: 'sum(for x in items return x.price * x.qty)',
    inputs: `{"items": ${items(20, true)}}`,
    size: '20 items, decimal prices',
  },
  {
    id: 'sum-integer',
    group: 'decimal formulas',
    source: 'sum(for x in items return x.price * x.qty)',
    inputs: `{"items": ${items(20, false)}}`,
    size: '20 items, integer prices',
  },
  {
    id: 'filter-project',
    group: 'list transformations',
    source: 'items[price > 10].name',
    inputs: `{"items": ${items(100, false)}}`,
    size: '100 items',
  },
  {
    id: 'map-double',
    group: 'list transformations',
    source: 'for x in xs return x * 2',
    inputs: `{"xs": ${ints(100, false)}}`,
    size: '100 numbers',
  },
  {
    id: 'sort-lambda',
    group: 'list transformations',
    source: 'sort(xs, function(a,b) a < b)',
    inputs: `{"xs": ${ints(100, true)}}`,
    size: '100 numbers (shuffled)',
  },
  {
    id: 'ut-range',
    group: 'decision-table conditions',
    source: '[18..65)',
    entry: 'unaryTests',
    inputs: '{}',
    candidate: '42',
    size: '1 candidate',
  },
  {
    id: 'ut-strings',
    group: 'decision-table conditions',
    source: '"A","B","C"',
    entry: 'unaryTests',
    inputs: '{}',
    candidate: '"C"',
    size: '1 candidate',
  },
  {
    id: 'ut-less',
    group: 'decision-table conditions',
    source: '< 1000',
    entry: 'unaryTests',
    inputs: '{}',
    candidate: '999.99',
    size: '1 candidate',
  },
];

let compiler: FeelCompiler;
let runtime: FeelRuntime;
let feelin: Feelin;

/** Load everything (warm path). */
export async function init(): Promise<{ version: unknown; feelin: string; crossOriginIsolated: boolean; hardwareConcurrency: number; userAgent: string }> {
  [compiler, runtime] = await Promise.all([loadCompiler(), loadRuntime()]);
  feelin = (await import(new URL('../vendor/feelin.js', import.meta.url).href)) as Feelin;
  return {
    version: compiler.version,
    feelin: feelin.FEELIN_VERSION,
    crossOriginIsolated: globalThis.crossOriginIsolated === true,
    hardwareConcurrency: navigator.hardwareConcurrency,
    userAgent: navigator.userAgent,
  };
}

/** Cold start in a fresh page: fetch + compile + instantiate both artifacts. */
export async function coldStart(): Promise<Record<string, number>> {
  const base = new URL('../', import.meta.url);
  const t0 = now();
  // what the demo does: both artifacts in parallel, streaming compilation
  const [c, r] = await Promise.all([loadCompiler(new URL('compiler.wasm', base)), loadRuntime(new URL('runtime.wasm', base))]);
  const total = now() - t0;
  // first useful work right after startup
  const t1 = now();
  const out = c.compile('a + b');
  const inst = await r.instantiate(out);
  const v = inst.evaluate({ a: 1, b: 2 }).valueJson;
  const firstEval = now() - t1;
  if (v !== '3') throw new Error(`unexpected ${v}`);
  return { totalMs: total, firstCompileInstantiateEvaluateMs: firstEval };
}

/** Cold start broken down (sequential, in a fresh page): fetch, compile, instantiate per artifact. */
export async function coldStartBreakdown(): Promise<Record<string, number>> {
  const base = new URL('../', import.meta.url);
  const out: Record<string, number> = {};
  for (const [name, file] of [
    ['compiler', 'compiler.wasm'],
    ['runtime', 'runtime.wasm'],
  ]) {
    let t = now();
    const bytes = await (await fetch(new URL(file, base))).arrayBuffer();
    out[`${name}FetchMs`] = now() - t;
    out[`${name}Bytes`] = bytes.byteLength;
    t = now();
    const mod = await WebAssembly.compile(bytes);
    out[`${name}CompileMs`] = now() - t;
    t = now();
    await WebAssembly.instantiate(mod, name === 'runtime' ? { feel_host_v1: { apply: () => 0, call_host: () => 0 } } : {});
    out[`${name}InstantiateMs`] = now() - t;
  }
  return out;
}

function options(w: Workload): CompileOptions {
  return w.entry === 'unaryTests' ? { entry: 'unaryTests' } : {};
}

/** Parse / check / compile (emit) times per workload, measured call by call. */
export function compileTimes(warmup: number, iterations: number) {
  return WORKLOADS.map((w) => {
    const o = options(w);
    const res: Record<string, Stats | number | string | null> = { id: w.id };
    for (const [phase, fn] of [
      ['parse', () => compiler.parse(w.source, o)],
      ['check', () => compiler.check(w.source, o)],
      ['compile', () => compiler.compile(w.source, o)],
    ] as const) {
      for (let i = 0; i < warmup; i++) fn();
      const samples: number[] = [];
      for (let i = 0; i < iterations; i++) {
        const t = now();
        fn();
        samples.push(now() - t);
      }
      res[phase] = stats(samples);
    }
    const c = compiler.compile(w.source, o);
    res.moduleBytes = c.bytes.length;
    res.resultType = c.metadata?.resultType ?? null;
    res.imports = c.metadata?.imports.length ?? 0;
    res.warmup = warmup;
    res.iterations = iterations;
    return res;
  });
}

/** Instantiation: (a) module compile + runtime instance + module instance; (b) instances only (module precompiled). */
export async function instantiateTimes(warmup: number, iterations: number) {
  const out = [];
  for (const w of WORKLOADS) {
    const c = compiler.compile(w.source, options(w));
    const module = await WebAssembly.compile(c.bytes as BufferSource);
    const res: Record<string, unknown> = { id: w.id, moduleBytes: c.bytes.length, warmup, iterations };
    for (const [name, arg] of [
      ['compileAndInstantiate', () => c.bytes.slice()],
      ['instantiateOnly', () => module],
    ] as const) {
      for (let i = 0; i < warmup; i++) await runtime.instantiate(arg());
      const samples: number[] = [];
      for (let i = 0; i < iterations; i++) {
        const a = arg();
        const t = now();
        await runtime.instantiate(a);
        samples.push(now() - t);
      }
      res[name] = stats(samples);
    }
    out.push(res);
  }
  return out;
}

// ---------------------------------------------------------------- equivalence with feelin

function toFeelin(v: FeelValue): unknown {
  if (v instanceof FeelDecimal) return v.toNumber();
  if (Array.isArray(v)) return v.map(toFeelin);
  if (v instanceof Map) return Object.fromEntries([...v.entries()].map(([k, x]) => [k, toFeelin(x)]));
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, toFeelin(x as FeelValue)]));
  return v;
}

function normNumber(text: string): string {
  let t = text;
  if (/[eE]/.test(t)) return String(Number(t));
  if (t.includes('.')) t = t.replace(/0+$/, '').replace(/\.$/, '');
  return t === '-0' ? '0' : t;
}

/** Value equality: numbers compared by decimal value (not scale), lists/contexts structurally. */
function equivalent(ours: FeelValue, theirs: unknown): boolean {
  if (ours instanceof FeelDecimal) return typeof theirs === 'number' && normNumber(ours.text) === normNumber(String(theirs));
  if (ours === null || typeof ours !== 'object') return ours === theirs;
  if (Array.isArray(ours)) return Array.isArray(theirs) && ours.length === theirs.length && ours.every((x, i) => equivalent(x, theirs[i]));
  if (ours instanceof Map) {
    if (!theirs || typeof theirs !== 'object') return false;
    const keys = Object.keys(theirs);
    return keys.length === ours.size && [...ours.entries()].every(([k, x]) => equivalent(x, (theirs as Record<string, unknown>)[k]));
  }
  return false;
}

function show(v: unknown): string {
  return typeof v === 'number' ? String(v) : JSON.stringify(v);
}

/** Batches: per-call time = batch time / calls; the batch size is chosen so that a batch takes ≥ targetMs. */
function measure(fn: () => void, warmupCalls: number, batches: number, targetMs: number) {
  for (let i = 0; i < warmupCalls; i++) fn();
  let k = 1;
  for (;;) {
    const t = now();
    for (let i = 0; i < k; i++) fn();
    if (now() - t >= targetMs || k >= 1 << 20) break;
    k *= 2;
  }
  const perCall: number[] = [];
  for (let b = 0; b < batches; b++) {
    const t = now();
    for (let i = 0; i < k; i++) fn();
    perCall.push(((now() - t) / k) * 1000); // µs
  }
  return { perCallUs: stats(perCall), batchSize: k, batches, calls: k * batches, warmupCalls };
}

/** Warm evaluation: ours (JS values / raw FEEL-JSON) vs feelin, after an equivalence check. */
export async function evalTimes(warmupCalls: number, batches: number, targetMs: number) {
  const out = [];
  for (const w of WORKLOADS) {
    const c = compiler.compile(w.source, options(w));
    const inst: FeelInstance = await runtime.instantiate(c);
    const inputs = parseFeelJson(w.inputs, { contexts: 'map' }) as FeelContext;
    const cand = w.candidate !== undefined ? parseFeelJson(w.candidate, { contexts: 'map' }) : undefined;
    const run = () => (cand !== undefined ? inst.evaluateUnary(cand, inputs) : inst.evaluate(inputs));
    const runJson = () => inst.evaluateJson(w.inputs, w.candidate);
    const ours = run();
    const res: Record<string, unknown> = { id: w.id, group: w.group, source: w.source, size: w.size, oursResult: ours.valueJson };
    if (!ours.ok) throw new Error(`${w.id}: ${JSON.stringify(ours.abort)}`);
    // feelin context: JS numbers (feelin uses binary floating point)
    const ctx = toFeelin(inputs) as Record<string, unknown>;
    let theirs: unknown;
    let feelinError: string | undefined;
    const feelinRun =
      w.entry === 'unaryTests'
        ? () => feelin.unaryTest(w.source, { ...ctx, '?': toFeelin(cand!) })
        : () => feelin.evaluate(w.source, ctx);
    try {
      theirs = feelinRun().value;
    } catch (e) {
      feelinError = String(e);
    }
    res.feelinResult = feelinError ?? show(theirs);
    res.equivalent = feelinError === undefined && equivalent(ours.value!, theirs);
    if (!res.equivalent) {
      res.excludedReason = feelinError
        ? `feelin error: ${feelinError}`
        : `results differ: ours ${ours.valueJson} vs feelin ${show(theirs)}` +
          (/\d\.\d{17,}/.test(ours.valueJson ?? '') || /\.\d*0000\d|\.\d*9999\d/.test(show(theirs)) ? ' (decimal vs binary floating point)' : '');
    }
    res.ours = measure(run, warmupCalls, batches, targetMs);
    res.oursJson = measure(runJson, warmupCalls, batches, targetMs);
    if (res.equivalent) res.feelin = measure(feelinRun, warmupCalls, batches, targetMs);
    out.push(res);
  }
  return out;
}

/** One-shot: compile + instantiate + evaluate (no caches), vs feelin's parse + evaluate per call. */
export async function oneShotTimes(warmup: number, iterations: number) {
  const out = [];
  for (const w of WORKLOADS) {
    const inputs = parseFeelJson(w.inputs, { contexts: 'map' }) as FeelContext;
    const cand = w.candidate !== undefined ? parseFeelJson(w.candidate, { contexts: 'map' }) : undefined;
    const once = async () => {
      const inst = await runtime.instantiate(compiler.compile(w.source, options(w)));
      return cand !== undefined ? inst.evaluateUnary(cand, inputs) : inst.evaluate(inputs);
    };
    for (let i = 0; i < warmup; i++) await once();
    const samples: number[] = [];
    for (let i = 0; i < iterations; i++) {
      const t = now();
      await once();
      samples.push(now() - t);
    }
    out.push({ id: w.id, oneShotMs: stats(samples), warmup, iterations });
  }
  return out;
}

/** JS–Wasm transfer: identity expression `x` on lists of growing size; phase breakdown. */
export async function transferTimes(sizes: number[], warmup: number, iterations: number) {
  const out = [];
  const instX = await runtime.instantiate(compiler.compile('x'));
  const instCount = await runtime.instantiate(compiler.compile('count(x)'));
  for (const n of sizes) {
    const list: FeelDecimal[] = [];
    for (let i = 0; i < n; i++) list.push(new FeelDecimal(`${i}.25`));
    const inputs = new Map<string, FeelValue>([['x', list]]);
    const json = encodeFeelJson(inputs);
    const res: Record<string, unknown> = { size: n, inputChars: json.length, warmup, iterations };
    const cases: [string, FeelInstance, () => void][] = [
      ['evaluateJson x', instX, () => instX.evaluateJson(json)],
      ['evaluate x (JS values)', instX, () => instX.evaluate(inputs)],
      ['evaluateJson count(x)', instCount, () => instCount.evaluateJson(json)],
    ];
    for (const [name, inst, fn] of cases) {
      const iters = n >= 100000 ? Math.max(5, Math.floor(iterations / 5)) : iterations;
      for (let i = 0; i < warmup; i++) fn();
      inst.profile = true;
      const total: number[] = [];
      const phases: Record<string, number[]> = { jsEncodeMs: [], decodeMs: [], bindMs: [], evaluateMs: [], encodeMs: [], jsDecodeMs: [] };
      for (let i = 0; i < iters; i++) {
        const t = now();
        fn();
        total.push(now() - t);
        const p = inst.lastProfile!;
        for (const k of Object.keys(phases)) phases[k].push(p[k as keyof typeof p] as number);
      }
      inst.profile = false;
      res[name] = {
        iterations: iters,
        totalMs: stats(total),
        phasesMedianMs: Object.fromEntries(Object.entries(phases).map(([k, v]) => [k, stats(v).median])),
      };
    }
    out.push(res);
  }
  return out;
}

(globalThis as unknown as { __bench: unknown }).__bench = {
  init,
  coldStart,
  coldStartBreakdown,
  compileTimes,
  instantiateTimes,
  evalTimes,
  oneShotTimes,
  transferTimes,
  WORKLOADS,
  ready: true,
};
