/**
 * {@link FeelWorker}: compile and evaluate FEEL inside a dedicated Web Worker,
 * with wall-clock timeouts that *cancel* runaway evaluations.
 *
 * Cancellation is implemented by terminating the worker (the only way to
 * stop a running Wasm call from outside); the worker is then transparently
 * restarted and previously compiled programs are reloaded on demand, so
 * the caller can keep using the same {@link WorkerProgram}s.
 *
 * The worker script is fetched once at {@link FeelWorker.create} and started
 * from a `blob:` URL, and the compiled `compiler.wasm` / `runtime.wasm`
 * modules are posted to it, so restarts need no network access.
 *
 * @module
 */

import { encodeFeelJson, parseFeelJson, type DecodeOptions, type FeelContext, type FeelValue } from './values.js';
import type {
  CompileOptions,
  CompileResult,
  EvalResult,
  Limits,
  ModuleMetadata,
  RawEvalResult,
  VersionInfo,
  WasmSource,
} from './feel.js';

/** A program compiled inside the worker; reusable across worker restarts. */
export interface WorkerProgram {
  readonly id: number;
  readonly source: string;
  readonly options?: CompileOptions;
  /** The emitted module bytes (kept to reload the program after a restart). */
  readonly bytes: Uint8Array;
  readonly metadata: ModuleMetadata;
}

export interface WorkerEvalOptions {
  /** Cancel (terminate the worker) when the evaluation takes longer than this. */
  timeoutMs?: number;
  /** Unary-test candidate (`?`). */
  candidate?: FeelValue;
  limits?: Partial<Limits>;
  /** `"system"` (default), `"none"` or a fixed date-and-time string. */
  clock?: string;
  decode?: DecodeOptions;
  /** Compile options when `evaluate` is given source text instead of a program. */
  compileOptions?: CompileOptions;
}

export interface FeelWorkerOptions {
  /** `compiler.wasm` (default `dist/compiler.wasm`). */
  compiler?: WasmSource;
  /** `runtime.wasm` (default `dist/runtime.wasm`). */
  runtime?: WasmSource;
  /** The bundled worker script (default `dist/worker.bundle.js`). */
  workerUrl?: string | URL;
  /** Timeout applied when an evaluation does not specify one (default: none). */
  defaultTimeoutMs?: number;
}

interface Pending {
  msg: Record<string, unknown> & { id: number; type: string };
  transfer: Transferable[];
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
  timer?: ReturnType<typeof setTimeout>;
  programId?: number;
}

const CANCELLED = 'FEEL-B590';
const WORKER_FAILED = 'FEEL-B591';

async function moduleOf(src: WasmSource | undefined, file: string): Promise<WebAssembly.Module> {
  if (src instanceof WebAssembly.Module) return src;
  let s: WasmSource = src ?? new URL(`../${file}`, import.meta.url);
  if (typeof s === 'string' || s instanceof URL) s = await fetch(s);
  if (s instanceof Response) return WebAssembly.compile(await s.arrayBuffer());
  return WebAssembly.compile((ArrayBuffer.isView(s) ? s : new Uint8Array(s as ArrayBuffer)) as BufferSource);
}

/**
 * Compile and evaluate FEEL in a dedicated worker with cancellable timeouts.
 *
 * ```ts
 * const w = await FeelWorker.create();
 * const prog = await w.compile('count(for i in 1..n return i)');
 * const r = await w.evaluate(prog.program!, { n: 10 }, { timeoutMs: 200 });
 * if (r.abort?.kind === 'cancelled') console.log('took too long; worker restarted');
 * ```
 */
export class FeelWorker {
  /** Versions reported by the worker's compiler. */
  version!: VersionInfo;
  /** Number of times the worker was terminated and restarted. */
  restarts = 0;
  private worker!: Worker;
  private generation = 0;
  private loaded = new Set<number>();
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private nextProgram = 1;
  private programs = new Map<number, WorkerProgram>();
  private bySource = new Map<string, Promise<CompileResult & { program?: WorkerProgram }>>();
  private closed = false;

  private constructor(
    private readonly scriptUrl: string,
    private readonly compilerModule: WebAssembly.Module,
    private readonly runtimeModule: WebAssembly.Module,
    private readonly defaultTimeoutMs: number | undefined,
  ) {}

  /** Start a worker. Fetches the worker script and the Wasm artifacts once. */
  static async create(options: FeelWorkerOptions = {}): Promise<FeelWorker> {
    const url = options.workerUrl ?? new URL('../worker.bundle.js', import.meta.url);
    const [code, compiler, runtime] = await Promise.all([
      fetch(url).then((r) => {
        if (!r.ok) throw new Error(`failed to fetch worker script ${String(url)}: HTTP ${r.status}`);
        return r.text();
      }),
      moduleOf(options.compiler, 'compiler.wasm'),
      moduleOf(options.runtime, 'runtime.wasm'),
    ]);
    const blobUrl = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
    const w = new FeelWorker(blobUrl, compiler, runtime, options.defaultTimeoutMs);
    await w.start();
    return w;
  }

  private async start(): Promise<void> {
    this.generation++;
    this.loaded = new Set();
    this.worker = new Worker(this.scriptUrl, { type: 'module', name: `feel-worker-${this.generation}` });
    const gen = this.generation;
    this.worker.onmessage = (ev: MessageEvent) => this.onMessage(ev.data as { id: number; ok: boolean; result?: unknown; error?: string });
    this.worker.onerror = (ev: ErrorEvent) => {
      ev.preventDefault();
      if (gen === this.generation) this.crash(`worker error: ${ev.message || 'unknown error'}`);
    };
    const init = this.post({ type: 'init', compiler: this.compilerModule, runtime: this.runtimeModule });
    this.version = (await init) as VersionInfo;
  }

  private onMessage(m: { id: number; ok: boolean; result?: unknown; error?: string }): void {
    const p = this.pending.get(m.id);
    if (!p) return;
    this.pending.delete(m.id);
    if (p.timer) clearTimeout(p.timer);
    if (m.ok) p.resolve(m.result);
    else p.reject(new Error(m.error ?? 'worker request failed'));
  }

  private send(p: Pending): void {
    if (p.programId !== undefined && !this.loaded.has(p.programId)) {
      const prog = this.programs.get(p.programId)!;
      // fire-and-forget load; messages are handled in order by the worker
      this.worker.postMessage({ id: 0, type: 'load', programId: prog.id, bytes: prog.bytes });
      this.loaded.add(p.programId);
    }
    this.worker.postMessage(p.msg, p.transfer);
  }

  private post(msg: Record<string, unknown> & { type: string }, opts: { timeoutMs?: number; programId?: number; transfer?: Transferable[] } = {}) {
    if (this.closed) return Promise.reject(new Error('FeelWorker was terminated'));
    const id = this.nextId++;
    return new Promise<unknown>((resolve, reject) => {
      const p: Pending = { msg: { ...msg, id }, transfer: opts.transfer ?? [], resolve, reject, programId: opts.programId };
      this.pending.set(id, p);
      if (opts.timeoutMs !== undefined && opts.timeoutMs >= 0) {
        p.timer = setTimeout(() => this.cancel(id, opts.timeoutMs!), opts.timeoutMs);
      }
      this.send(p);
    });
  }

  /** Terminate the worker because request `id` timed out; restart and re-dispatch the others. */
  private cancel(id: number, timeoutMs: number): void {
    const p = this.pending.get(id);
    if (!p) return;
    this.pending.delete(id);
    this.restart();
    p.resolve({
      ok: false,
      valueJson: null,
      diagnostics: [],
      abort: { kind: 'cancelled', code: CANCELLED, message: `evaluation cancelled after ${timeoutMs} ms (worker terminated and restarted)` },
    } satisfies RawEvalResult);
  }

  private crash(message: string): void {
    // fail everything in flight, then restart
    const all = [...this.pending.values()];
    this.pending.clear();
    this.restart();
    for (const p of all) {
      if (p.timer) clearTimeout(p.timer);
      if (p.msg.type === 'evaluate') {
        p.resolve({ ok: false, valueJson: null, diagnostics: [], abort: { kind: 'boundary', code: WORKER_FAILED, message } } satisfies RawEvalResult);
      } else p.reject(new Error(message));
    }
  }

  private restart(): void {
    this.worker.terminate();
    this.restarts++;
    const others = [...this.pending.values()];
    this.pending.clear();
    this.start().catch((e) => this.crash(String(e)));
    // re-dispatch requests that were queued behind the cancelled one (their timers keep running)
    for (const p of others) {
      if (p.msg.type === 'init') continue;
      this.pending.set(p.msg.id, p);
      this.send(p);
    }
  }

  /**
   * Compile in the worker (memoized there by source and options). On
   * success the result carries a {@link WorkerProgram} handle.
   */
  async compile(source: string, options?: CompileOptions): Promise<CompileResult & { program?: WorkerProgram }> {
    const programId = this.nextProgram++;
    const r = (await this.post({ type: 'compile', source, options, programId })) as CompileResult;
    if (!r.ok || !r.metadata) return r;
    this.loaded.add(programId); // the worker registered it while compiling
    const program: WorkerProgram = { id: programId, source, options, bytes: r.bytes, metadata: r.metadata };
    this.programs.set(programId, program);
    return { ...r, program };
  }

  /** Evaluate a program (or source text, compiled and cached) with decoded inputs/outputs. */
  async evaluate(program: WorkerProgram | string, inputs: FeelContext | null = {}, options: WorkerEvalOptions = {}): Promise<EvalResult> {
    const candidateJson = options.candidate === undefined ? null : encodeFeelJson(options.candidate);
    const r = await this.evaluateJson(program, encodeFeelJson(inputs ?? {}), { ...options, candidateJson });
    return r.valueJson === null ? r : { ...r, value: parseFeelJson(r.valueJson, options.decode) };
  }

  /** Evaluate with raw FEEL-JSON inputs/outputs. */
  async evaluateJson(
    program: WorkerProgram | string,
    inputsJson: string,
    options: WorkerEvalOptions & { candidateJson?: string | null } = {},
  ): Promise<RawEvalResult> {
    let prog: WorkerProgram;
    if (typeof program === 'string') {
      const key = JSON.stringify([program, options.compileOptions ?? {}]);
      let c = this.bySource.get(key);
      if (!c) this.bySource.set(key, (c = this.compile(program, options.compileOptions)));
      const r = await c;
      if (!r.program) {
        const d = r.diagnostics.find((x) => x.severity === 'error');
        return { ok: false, valueJson: null, diagnostics: [], abort: { kind: 'boundary', code: d?.code ?? 'FEEL-B501', message: d?.message ?? 'compilation failed' } };
      }
      prog = r.program;
    } else prog = program;
    const timeoutMs = options.timeoutMs ?? this.defaultTimeoutMs;
    return (await this.post(
      {
        type: 'evaluate',
        programId: prog.id,
        inputsJson,
        candidateJson: options.candidateJson ?? null,
        limits: options.limits ?? null,
        clock: options.clock ?? 'system',
      },
      { timeoutMs, programId: prog.id },
    )) as RawEvalResult;
  }

  /** Stop the worker for good. Pending requests are rejected. */
  terminate(): void {
    this.closed = true;
    this.worker.terminate();
    for (const p of this.pending.values()) {
      if (p.timer) clearTimeout(p.timer);
      p.reject(new Error('FeelWorker was terminated'));
    }
    this.pending.clear();
    URL.revokeObjectURL(this.scriptUrl);
  }
}
