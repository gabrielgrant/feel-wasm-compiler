/**
 * Dedicated-worker entry point used by {@link FeelWorker}. Bundled into
 * `dist/worker.bundle.js` (self-contained, so it can run from a `blob:` URL).
 *
 * Protocol (main → worker, each with a numeric `id`; replies are
 * `{id, ok, result | error}`; messages are handled strictly in order):
 * - `init {compiler: WebAssembly.Module, runtime: WebAssembly.Module}` → version info
 * - `compile {source, options, programId}` → CompileResult (registers the program)
 * - `load {programId, bytes}` → (no reply) register a program after a restart
 * - `evaluate {programId, inputsJson, candidateJson, limits, clock}` → RawEvalResult
 *
 * @module
 */

import { FeelCompiler, FeelInstance, FeelRuntime, loadCompiler, loadRuntime, type CompileResult, type Limits } from './feel.js';

interface Scope {
  postMessage(msg: unknown, transfer?: Transferable[]): void;
  onmessage: ((ev: MessageEvent) => void) | null;
}
const scope = self as unknown as Scope;

let compiler: FeelCompiler | undefined;
let runtime: FeelRuntime | undefined;
const programs = new Map<number, WebAssembly.Module>();
const instances = new Map<string, FeelInstance>();

type Msg =
  | { id: number; type: 'init'; compiler: WebAssembly.Module; runtime: WebAssembly.Module }
  | { id: number; type: 'compile'; source: string; options?: object; programId: number }
  | { id: number; type: 'load'; programId: number; bytes: Uint8Array }
  | {
      id: number;
      type: 'evaluate';
      programId: number;
      inputsJson: string;
      candidateJson: string | null;
      limits: Partial<Limits> | null;
      clock: string;
    };

async function handle(m: Msg): Promise<unknown> {
  switch (m.type) {
    case 'init':
      compiler = await loadCompiler(m.compiler);
      runtime = await loadRuntime(m.runtime);
      return compiler.version;
    case 'compile': {
      const r: CompileResult = compiler!.compileCached(m.source, m.options);
      if (r.ok) programs.set(m.programId, await WebAssembly.compile(r.bytes as BufferSource));
      // copy: the cached result keeps its own bytes
      return { ...r, bytes: r.bytes.slice() };
    }
    case 'load':
      programs.set(m.programId, await WebAssembly.compile(m.bytes as BufferSource));
      return undefined;
    case 'evaluate': {
      const key = `${m.programId}\u0000${JSON.stringify(m.limits)}\u0000${m.clock}`;
      let inst = instances.get(key);
      if (!inst) {
        const mod = programs.get(m.programId);
        if (!mod) throw new Error(`unknown program ${m.programId}`);
        inst = await runtime!.instantiate(mod, { limits: m.limits ?? undefined, clock: m.clock });
        instances.set(key, inst);
      }
      await inst.ready();
      return inst.evaluateJson(m.inputsJson, m.candidateJson);
    }
  }
}

let chain: Promise<void> = Promise.resolve();
scope.onmessage = (ev: MessageEvent) => {
  const m = ev.data as Msg;
  chain = chain.then(async () => {
    try {
      const result = await handle(m);
      if (m.id !== 0) {
        const transfer = m.type === 'compile' && result && (result as CompileResult).bytes ? [(result as CompileResult).bytes.buffer as ArrayBuffer] : [];
        scope.postMessage({ id: m.id, ok: true, result }, transfer);
      }
    } catch (e) {
      if (m.id !== 0) scope.postMessage({ id: m.id, ok: false, error: e instanceof Error ? e.message : String(e) });
    }
  });
};
