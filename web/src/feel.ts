/**
 * Browser (and Node) bindings for the FEEL (DMN 1.5) → WebAssembly compiler.
 *
 * Three Wasm artifacts are involved:
 * - `compiler.wasm` — the compiler itself (parse / check / emit), no imports;
 * - `runtime.wasm` — the shared FEEL runtime (`feel_rt_v1` ABI): decimal
 *   arithmetic, temporal values, builtins, FEEL-JSON codec;
 * - the *expression modules* emitted by the compiler: code specialized to
 *   one FEEL expression, importing only primitive helpers from the runtime.
 *
 * ```ts
 * import { loadCompiler, loadRuntime, FeelDecimal } from './feel.js';
 * const compiler = await loadCompiler();          // fetches ../compiler.wasm
 * const runtime = await loadRuntime();            // fetches ../runtime.wasm
 * const out = compiler.compile('a + b', { check: 'permissive' });
 * const inst = await runtime.instantiate(out);
 * inst.evaluate({ a: new FeelDecimal('0.1'), b: 0.2 }).valueJson; // "0.3"
 * ```
 *
 * Isolation: every {@link FeelInstance} owns a private runtime instance (its
 * own linear memory, value arena, limits, clock and host bindings) and a
 * private expression-module instance. Instances never share mutable state.
 *
 * @module
 */

import {
  encodeFeelJson,
  parseFeelJson,
  FeelDateTime,
  type DecodeOptions,
  type FeelValue,
  type FeelContext,
} from './values.js';

export * from './values.js';

// ======================================================================
// Shared types
// ======================================================================

/** Anything a Wasm artifact can be loaded from. */
export type WasmSource = string | URL | Response | ArrayBuffer | ArrayBufferView | WebAssembly.Module;

/** A compiler diagnostic (syntax error, checker finding, schema error). */
export interface Diagnostic {
  /** Stable code: `FEEL-S0xx` syntax, `FEEL-C1xx` checker, `FEEL-W2xx` lint, `FEEL-B5xx` boundary. */
  code: string;
  severity: 'error' | 'warning' | 'info';
  message: string;
  /** UTF-8 **byte** offsets into the source (use {@link byteToCharOffset} for JS string indices). */
  span: { start: number; end: number };
  /** 1-based line of `span.start`. */
  line: number;
  /** 1-based column (in Unicode scalar values) of `span.start`. */
  column: number;
  suggestion?: string;
}

/** Input schema (all optional). See `crates/feel-compiler/src/types.rs`. */
export interface Schema {
  /** Declared inputs: `{"age": "number"}` or `[{"name": "age", "type": "number"}]`. */
  inputs?: Record<string, string> | { name: string; type?: string }[];
  /** Named types (item definitions): `{"Applicant": "context<name: string, age: number>"}`. */
  types?: Record<string, string> | [string, string][];
  /** Host functions callable by name from FEEL. */
  functions?: { name: string; params: { name: string; type?: string }[]; returns?: string }[];
  /** Type of the unary-test candidate `?`. */
  candidate?: string;
  /** Expected result type (checked). */
  expected?: string;
  /** When `true`, only declared inputs may be referenced. */
  closed?: boolean;
}

/** Compile options (the JSON form of `feel_compiler::CompileOptions`). */
export interface CompileOptions {
  /** `"expression"` (default) or `"unaryTests"` (decision-table cells, applied to a candidate `?`). */
  entry?: 'expression' | 'unaryTests';
  schema?: Schema;
  /** Type checking policy (default `"permissive"`: findings are warnings). */
  check?: 'off' | 'permissive' | 'strict';
  /** Constant folding, filter fusion, … (default `true`). */
  optimize?: boolean;
  /** Opt-in extensions outside DMN 1.5, e.g. `["has key"]`. */
  extensions?: string[];
  /** Validate inputs against declared schema types at the boundary (default `true`). */
  validateInputs?: boolean;
  /** Extra (multi-word) names the parser should recognise. */
  knownNames?: string[];
  /** Extra context field names recognised after `.`. */
  knownFields?: string[];
}

/** Static dependency report (for incremental recalculation). */
export interface Dependencies {
  inputs: { name: string; whole: boolean; paths: string[][] }[];
  clock: boolean;
  hostFunctions: string[];
  exact: boolean;
  uncertain: string[];
}

/** Metadata of an emitted module (also embedded in its `feel.meta` custom section). */
export interface ModuleMetadata {
  format: string;
  abiVersion: number;
  abiModule: string;
  compilerVersion: string;
  runtimeVersion: string;
  dialect: string;
  entry: 'expression' | 'unaryTests';
  extensions: string[];
  extensionFlags: number;
  /** Input slots in order (`?` is the unary-test candidate). */
  inputs: { name: string; slot: number; type: string | null }[];
  candidateSlot: number | null;
  /** Inferred result type (when checking was on). */
  resultType: string | null;
  constCount: number;
  functionCount: number;
  /** Runtime ABI functions the module imports (without the `f_` prefix). */
  imports: string[];
  /** Source sites referenced by evaluation diagnostics (byte offsets). */
  sites: { start: number; end: number; what: string }[];
  dependencies: Dependencies;
  optimized: boolean;
  foldedConstants: number;
  fusedFilters: number;
  sourceSha256: string;
  cacheKey: string;
  validateInputs: boolean;
}

export interface ParseResult {
  ok: boolean;
  /** The AST (JSON), when parsing succeeded. */
  ast?: unknown;
  diagnostics: Diagnostic[];
  /** Set when the options themselves were invalid. */
  error?: string;
}

export interface CheckResult {
  ok: boolean;
  /** Inferred FEEL type of the result, e.g. `"number"`, `"list<string>"`. */
  resultType: string | null;
  diagnostics: Diagnostic[];
  error?: string;
}

export interface CompileResult {
  ok: boolean;
  /** The emitted Wasm module (empty when `ok` is false). */
  bytes: Uint8Array;
  diagnostics: Diagnostic[];
  metadata?: ModuleMetadata;
  /** The constant pool (FEEL-JSON array; also in the `feel.consts` section). */
  constants?: string;
  /** Stable key over source, options, compiler/runtime/ABI versions (SHA-256 hex). */
  cacheKey: string;
  /** Set when the options were invalid (also reported as a `FEEL-B501` diagnostic). */
  error?: string;
  /** Wall time of the `fw_compile` call in milliseconds. */
  compileMs: number;
}

export interface VersionInfo {
  compiler: string;
  runtime: string;
  abiModule: string;
  abiVersion: number;
  dialect: string;
  tzdb: string;
}

/** An evaluation diagnostic, produced alongside a value (usually `null`). */
export interface EvalDiagnostic {
  /** `FEEL-E3xx` (runtime semantics) — never aborts evaluation. */
  code: string;
  message: string;
  /** Source span (UTF-8 byte offsets) of the operation, `0..0` when unknown. */
  start: number;
  end: number;
}

/**
 * Why an evaluation produced no value. This is distinct from a FEEL `null`.
 * - `limit`: a resource limit was exceeded (`FEEL-L4xx`, e.g. `FEEL-L404` steps);
 * - `boundary`: bad inputs / clock / ABI misuse (`FEEL-B5xx`);
 * - `cancelled`: a {@link FeelWorker} evaluation exceeded its `timeoutMs`
 *   and the worker was terminated (`FEEL-B590`).
 */
export interface AbortInfo {
  kind: 'limit' | 'boundary' | 'cancelled';
  code: string;
  message: string;
}

/** Raw evaluation outcome (FEEL-JSON text, no JS decoding). */
export interface RawEvalResult {
  /** `true` when a value was produced (it may be FEEL `null`). */
  ok: boolean;
  /** Exact canonical FEEL-JSON of the result (`"null"` for FEEL null); `null` when aborted. */
  valueJson: string | null;
  diagnostics: EvalDiagnostic[];
  abort?: AbortInfo;
}

/** Evaluation outcome with the decoded value. */
export interface EvalResult extends RawEvalResult {
  /** The decoded result (numbers as {@link FeelDecimal}); absent when aborted. */
  value?: FeelValue;
}

/** Phase timings of one evaluation (milliseconds), recorded when {@link FeelInstance.profile} is on. */
export interface EvalProfile {
  /** UTF-16 length of the FEEL-JSON inputs / result text. */
  inputChars: number;
  outputChars: number;
  /** JS values → FEEL-JSON text (only for `evaluate`/`evaluateUnary`). */
  jsEncodeMs: number;
  /** Copy the text into Wasm memory + runtime FEEL-JSON decode (inputs and candidate). */
  decodeMs: number;
  /** Bind input slots (and boundary validation). */
  bindMs: number;
  /** The generated module's `evaluate()` call. */
  evaluateMs: number;
  /** Runtime FEEL-JSON encode of the result + UTF-8 decode into a JS string, + diagnostics. */
  encodeMs: number;
  /** FEEL-JSON text → JS values (only for `evaluate`/`evaluateUnary`). */
  jsDecodeMs: number;
}

/** Explicit evaluation limits for untrusted expressions. */
export interface Limits {
  /** Loop iterations + function entries per evaluation (default 50 000 000). */
  maxSteps: number | bigint;
  /** User-function call depth (default 512). */
  maxDepth: number;
  /** Values allocated per evaluation, inputs included (default 4 000 000). */
  maxValues: number;
  /** Items in any list, incl. `for` ranges (default 1 000 000). */
  maxCollection: number;
  /** String length in bytes (default 16 MiB). */
  maxString: number;
  /** Evaluation diagnostics retained (default 64). */
  maxDiags: number;
}

/** Default limits (same as the native host). */
export const DEFAULT_LIMITS: Readonly<Limits> = Object.freeze({
  maxSteps: 50_000_000,
  maxDepth: 512,
  maxValues: 4_000_000,
  maxCollection: 1_000_000,
  maxString: 16 * 1024 * 1024,
  maxDiags: 64,
});

/** A host function: receives decoded FEEL arguments, returns a FEEL value. Throwing yields `null` + `FEEL-E362`. */
export type HostFunction = (args: FeelValue[]) => FeelValue;

/**
 * The clock that `now()` / `today()` observe:
 * - `"system"` (default): the current UTC time, sampled at the start of each evaluation;
 * - `"none"`: `now()`/`today()` yield `null` with a diagnostic;
 * - a FEEL date-and-time string or a JS `Date`: a fixed instant (reproducible runs).
 */
export type ClockOption = 'system' | 'none' | string | Date;

export interface InstanceOptions {
  limits?: Partial<Limits>;
  clock?: ClockOption;
  /** Host functions by name (declared in `schema.functions`). */
  hostFunctions?: Record<string, HostFunction>;
  /** How results are decoded into JS values. */
  decode?: DecodeOptions;
  /** Runtime to use (defaults to the one loaded by {@link loadRuntime}). */
  runtime?: FeelRuntime;
}

/** Codes produced by these JS bindings (not by the Rust runtime). */
export const BINDING_CODES = {
  /** A trap without abort information (engine stack overflow, bug). Same as the native host. */
  TRAP: 'FEEL-B503',
  /** Invalid clock instant. Same as the native host. */
  CLOCK: 'FEEL-B504',
  /** Invalid FEEL-JSON inputs or candidate. Same as the native host. */
  INPUTS: 'FEEL-B505',
  /** Inputs violate declared schema types. Same as the native host. */
  BIND: 'FEEL-B506',
  /** Worker evaluation cancelled (timeout); worker terminated and restarted. */
  CANCELLED: 'FEEL-B590',
  /** Worker failed (crashed or could not start). */
  WORKER: 'FEEL-B591',
  /** Invalid module / incompatible ABI at instantiation. */
  MODULE: 'FEEL-B592',
} as const;

// ======================================================================
// Helpers
// ======================================================================

const utf8enc = new TextEncoder();
const utf8dec = new TextDecoder('utf-8', { fatal: false });
const now = (): number => (typeof performance !== 'undefined' ? performance.now() : Date.now());

function defaultUrl(file: string): URL {
  // dist/src/feel.js → dist/<file>
  return new URL(`../${file}`, import.meta.url);
}

async function toModule(src: WasmSource | undefined, file: string): Promise<WebAssembly.Module> {
  if (src instanceof WebAssembly.Module) return src;
  if (src === undefined) src = defaultUrl(file);
  if (typeof src === 'string' || src instanceof URL) {
    const res = await fetch(src);
    if (!res.ok) throw new Error(`failed to fetch ${String(src)}: HTTP ${res.status}`);
    src = res;
  }
  if (typeof Response !== 'undefined' && src instanceof Response) {
    if (typeof WebAssembly.compileStreaming === 'function' && (src.headers.get('content-type') ?? '').includes('application/wasm')) {
      return WebAssembly.compileStreaming(src);
    }
    return WebAssembly.compile(await src.arrayBuffer());
  }
  const bytes = ArrayBuffer.isView(src) ? new Uint8Array(src.buffer, src.byteOffset, src.byteLength) : new Uint8Array(src as ArrayBuffer);
  return WebAssembly.compile(bytes as BufferSource);
}

/** Convert a UTF-8 byte offset in `source` to a JS (UTF-16) string index. */
export function byteToCharOffset(source: string, byteOffset: number): number {
  let bytes = 0;
  for (let i = 0; i < source.length; i++) {
    if (bytes >= byteOffset) return i;
    const c = source.codePointAt(i)!;
    bytes += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
    if (c >= 0x10000) i++;
  }
  return source.length;
}

/** Order-preserving canonical JSON used for cache lookup keys. */
function stableStringify(v: unknown): string {
  if (v === null || typeof v !== 'object') return JSON.stringify(v) ?? 'null';
  if (Array.isArray(v)) return `[${v.map(stableStringify).join(',')}]`;
  const o = v as Record<string, unknown>;
  return `{${Object.keys(o)
    .filter((k) => o[k] !== undefined)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${stableStringify(o[k])}`)
    .join(',')}}`;
}

// ======================================================================
// Compiler
// ======================================================================

interface CompilerExports {
  memory: WebAssembly.Memory;
  fw_alloc(len: number): number;
  fw_free(ptr: number, len: number): void;
  fw_compile(s: number, sl: number, o: number, ol: number): number;
  fw_check(s: number, sl: number, o: number, ol: number): number;
  fw_parse(s: number, sl: number, o: number, ol: number): number;
  fw_version(): number;
  fw_out_ptr(): number;
  fw_out_len(): number;
  fw_wasm_ptr(): number;
  fw_wasm_len(): number;
}

interface CacheEntry {
  result: CompileResult;
  module?: WebAssembly.Module;
}

/**
 * The FEEL compiler running as WebAssembly (`compiler.wasm`). All methods
 * are synchronous; compile-time work for a typical expression is well
 * under a millisecond to a few milliseconds.
 */
export class FeelCompiler {
  /** Compiler/runtime/ABI versions reported by `fw_version()`. */
  readonly version: VersionInfo;
  /** Maximum number of entries kept by {@link compileCached} (LRU). */
  cacheLimit = 256;
  private ex: CompilerExports;
  private restarting: Promise<void> | undefined;
  private readonly cache = new Map<string, CacheEntry>();
  private cacheHits = 0;
  private cacheMisses = 0;

  /** @internal use {@link loadCompiler} */
  constructor(
    /** The compiled `compiler.wasm` (can be posted to workers). */
    readonly module: WebAssembly.Module,
    instance: WebAssembly.Instance,
  ) {
    this.ex = instance.exports as unknown as CompilerExports;
    this.ex.fw_version();
    this.version = JSON.parse(this.readOut()) as VersionInfo;
  }

  /** Resolves once the compiler is usable (after a restart following an internal error). */
  async ready(): Promise<void> {
    while (this.restarting) await this.restarting;
  }

  private restart(): void {
    try {
      this.ex = new WebAssembly.Instance(this.module, {}).exports as unknown as CompilerExports;
    } catch {
      // synchronous instantiation not allowed here: restart in the background
      this.restarting = WebAssembly.instantiate(this.module, {}).then((i) => {
        this.ex = i.exports as unknown as CompilerExports;
        this.restarting = undefined;
      });
    }
  }

  private readOut(): string {
    const ex = this.ex;
    return utf8dec.decode(new Uint8Array(ex.memory.buffer, ex.fw_out_ptr(), ex.fw_out_len()));
  }

  private put(s: string): [number, number] {
    const b = utf8enc.encode(s);
    const p = this.ex.fw_alloc(b.length);
    new Uint8Array(this.ex.memory.buffer, p, b.length).set(b);
    return [p, b.length];
  }

  /** Run one `fw_*` entry point; returns the parsed report and module bytes. */
  private call(fn: 'fw_compile' | 'fw_check' | 'fw_parse', source: string, options: CompileOptions | undefined) {
    if (this.restarting) throw new Error('the compiler is restarting after an internal error: await compiler.ready()');
    const opts = options ? JSON.stringify(options) : '';
    const [sp, sl] = this.put(source);
    const [op, ol] = this.put(opts);
    let report: Record<string, unknown>;
    let bytes = new Uint8Array(0);
    const t0 = now();
    let ms: number;
    try {
      this.ex[fn](sp, sl, op, ol);
      ms = now() - t0;
      report = JSON.parse(this.readOut()) as Record<string, unknown>;
      if (fn === 'fw_compile') {
        const len = this.ex.fw_wasm_len();
        if (len > 0) bytes = new Uint8Array(this.ex.memory.buffer, this.ex.fw_wasm_ptr(), len).slice();
      }
      this.ex.fw_free(sp, sl);
      this.ex.fw_free(op, ol);
    } catch (e) {
      // A trap (e.g. an internal panic) leaves the compiler instance in an
      // unknown state: replace it.
      ms = now() - t0;
      this.restart();
      const message = `internal compiler error: ${e instanceof Error ? e.message : String(e)}`;
      report = {
        ok: false,
        diagnostics: [{ code: BINDING_CODES.TRAP, severity: 'error', message, span: { start: 0, end: 0 }, line: 1, column: 1 }],
      };
    }
    if (typeof report.error === 'string') {
      // invalid options JSON / schema shape
      report.diagnostics = [
        { code: 'FEEL-B501', severity: 'error', message: report.error, span: { start: 0, end: 0 }, line: 1, column: 1 },
      ];
    }
    return { report, bytes, ms };
  }

  /** Parse only. */
  parse(source: string, options?: CompileOptions): ParseResult {
    const { report } = this.call('fw_parse', source, options);
    return {
      ok: report.ok === true,
      ast: report.ast ?? undefined,
      diagnostics: (report.diagnostics as Diagnostic[]) ?? [],
      ...(typeof report.error === 'string' ? { error: report.error } : {}),
    };
  }

  /** Parse and type-check (never emits code). */
  check(source: string, options?: CompileOptions): CheckResult {
    const { report } = this.call('fw_check', source, options);
    return {
      ok: report.ok === true,
      resultType: (report.resultType as string | null) ?? null,
      diagnostics: (report.diagnostics as Diagnostic[]) ?? [],
      ...(typeof report.error === 'string' ? { error: report.error } : {}),
    };
  }

  /** Parse, check (unless `check: "off"`), lower, optimize and emit a Wasm module. */
  compile(source: string, options?: CompileOptions): CompileResult {
    const { report, bytes, ms } = this.call('fw_compile', source, options);
    const metadata = (report.metadata as ModuleMetadata | null) ?? undefined;
    return {
      ok: report.ok === true && bytes.length > 0,
      bytes,
      diagnostics: (report.diagnostics as Diagnostic[]) ?? [],
      metadata,
      constants: (report.constants as string | null) ?? undefined,
      cacheKey: (report.cacheKey as string) ?? metadata?.cacheKey ?? '',
      compileMs: ms,
      ...(typeof report.error === 'string' ? { error: report.error } : {}),
    };
  }

  /**
   * Like {@link compile}, memoized. The lookup key covers the source, the
   * (canonicalized) options and the compiler/runtime/ABI versions, so a new
   * compiler build never serves stale modules; the result's `cacheKey` is
   * the compiler's own SHA-256 key over the same data (suitable for
   * persistent caches such as IndexedDB). The returned object is shared:
   * do not mutate it. The compiled `WebAssembly.Module` is cached alongside,
   * so repeated {@link FeelRuntime.instantiate} calls skip Wasm compilation.
   */
  compileCached(source: string, options?: CompileOptions): CompileResult {
    const key = `${this.version.compiler}\u0000${this.version.runtime}\u0000${this.version.abiVersion}\u0000${stableStringify(options ?? {})}\u0000${source}`;
    const hit = this.cache.get(key);
    if (hit) {
      this.cacheHits++;
      this.cache.delete(key);
      this.cache.set(key, hit); // LRU refresh
      return hit.result;
    }
    this.cacheMisses++;
    const result = this.compile(source, options);
    this.cache.set(key, { result });
    if (result.ok) moduleCache.set(result, undefined);
    while (this.cache.size > this.cacheLimit) this.cache.delete(this.cache.keys().next().value as string);
    return result;
  }

  /** Cache statistics for {@link compileCached}. */
  get cacheStats(): { size: number; hits: number; misses: number } {
    return { size: this.cache.size, hits: this.cacheHits, misses: this.cacheMisses };
  }

  clearCache(): void {
    this.cache.clear();
  }
}

/** Compiled `WebAssembly.Module`s of cached compile results (filled lazily). */
const moduleCache = new WeakMap<CompileResult, WebAssembly.Module | undefined>();

/**
 * Load `compiler.wasm` (default: `dist/compiler.wasm` next to this module).
 * Accepts a URL, a `Response`, raw bytes or an already compiled module.
 */
export async function loadCompiler(source?: WasmSource): Promise<FeelCompiler> {
  const module = await toModule(source, 'compiler.wasm');
  const instance = await WebAssembly.instantiate(module, {});
  return new FeelCompiler(module, instance);
}

// ======================================================================
// Runtime and instances
// ======================================================================

interface RuntimeExports {
  memory: WebAssembly.Memory;
  rt_alloc(len: number): number;
  rt_free(ptr: number, len: number): void;
  rt_out_len(): number;
  rt_out_ptr(): number;
  rt_load_consts(ptr: number, len: number): number;
  rt_set_limits(maxValues: number, maxCollection: number, maxString: number, maxDiags: number, maxDepth: number): void;
  /** Step limit as reported in FEEL-L404 messages (fuel is enforced by the module). */
  rt_set_max_steps?(steps: bigint): void;
  rt_set_extensions(flags: number): void;
  rt_set_clock(ptr: number, len: number): number;
  rt_begin_eval(): void;
  rt_decode(ptr: number, len: number): number;
  rt_bind_inputs(inputs: number, candidate: number, names: number, types: number, validate: number): number;
  rt_encode(h: number): number;
  rt_diags(): number;
  rt_abort(): number;
  rt_clear_abort(): void;
  rt_eval_values(): number;
  rt_abi_version(): number;
  [name: string]: unknown;
}

interface ExprExports {
  evaluate(): number;
  __feel_apply(f: number, args: number): number;
  __feel_set_limits(maxFuel: bigint, maxDepth: number): void;
}

/** A loaded `feel_rt_v1` module, ready to back expression instances. */
interface LoadedProgram {
  module: WebAssembly.Module;
  metadata: ModuleMetadata;
  consts: Uint8Array;
}

const programCache = new WeakMap<WebAssembly.Module, LoadedProgram>();

/**
 * The shared runtime (`runtime.wasm`), compiled once. Each
 * {@link FeelRuntime.instantiate} call creates a fresh, isolated runtime
 * instance for one expression-module instance.
 */
export class FeelRuntime {
  /** `rt_abi_version()` of this runtime build. */
  readonly abiVersion: number;
  private readonly exportNames: Set<string>;

  /** @internal use {@link loadRuntime} */
  constructor(
    readonly module: WebAssembly.Module,
    abiVersion: number,
  ) {
    this.exportNames = new Set(WebAssembly.Module.exports(module).map((e) => e.name));
    this.abiVersion = abiVersion;
  }

  /** Validate an emitted module against this runtime (imports, ABI version, custom sections). */
  async load(compiled: CompileResult | Uint8Array | ArrayBuffer | WebAssembly.Module): Promise<LoadedProgram> {
    let module: WebAssembly.Module | undefined;
    if (compiled instanceof WebAssembly.Module) module = compiled;
    else if (compiled instanceof Uint8Array || compiled instanceof ArrayBuffer) module = await WebAssembly.compile(compiled as BufferSource);
    else {
      if (!compiled.ok) throw new FeelModuleError('cannot instantiate a failed compilation');
      module = moduleCache.get(compiled);
      if (!module) {
        module = await WebAssembly.compile(compiled.bytes as BufferSource);
        if (moduleCache.has(compiled)) moduleCache.set(compiled, module);
      }
    }
    const cached = programCache.get(module);
    if (cached) return cached;
    const meta = WebAssembly.Module.customSections(module, 'feel.meta');
    const consts = WebAssembly.Module.customSections(module, 'feel.consts');
    if (meta.length !== 1 || consts.length !== 1) throw new FeelModuleError('not a FEEL module: missing feel.meta / feel.consts section');
    const metadata = JSON.parse(utf8dec.decode(meta[0])) as ModuleMetadata;
    if (metadata.abiModule !== 'feel_rt_v1' || metadata.abiVersion !== this.abiVersion) {
      throw new FeelModuleError(
        `incompatible module: compiled for runtime ABI ${metadata.abiModule} v${metadata.abiVersion}, this runtime provides feel_rt_v1 v${this.abiVersion}`,
      );
    }
    for (const imp of WebAssembly.Module.imports(module)) {
      if (imp.module !== 'feel_rt_v1') throw new FeelModuleError(`unexpected import module '${imp.module}'`);
      if (imp.kind !== 'function' || !imp.name.startsWith('f_') || !this.exportNames.has(imp.name)) {
        throw new FeelModuleError(`unknown runtime import '${imp.name}'`);
      }
    }
    const p: LoadedProgram = { module, metadata, consts: new Uint8Array(consts[0]) };
    programCache.set(module, p);
    return p;
  }

  /**
   * Create an isolated instance of a compiled expression: a fresh runtime
   * instance (own memory) + a fresh expression-module instance.
   */
  async instantiate(compiled: CompileResult | Uint8Array | ArrayBuffer | WebAssembly.Module, options: InstanceOptions = {}): Promise<FeelInstance> {
    const program = await this.load(compiled);
    const inst = new FeelInstance(this, program, options);
    await inst.ready();
    return inst;
  }
}

/** Thrown when a module cannot be instantiated against this runtime. */
export class FeelModuleError extends Error {
  readonly code = BINDING_CODES.MODULE;
  constructor(message: string) {
    super(message);
    this.name = 'FeelModuleError';
  }
}

let defaultRuntime: FeelRuntime | undefined;

/**
 * Load `runtime.wasm` (default: `dist/runtime.wasm` next to this module) and
 * make it the default runtime for {@link instantiate}.
 */
export async function loadRuntime(source?: WasmSource): Promise<FeelRuntime> {
  const module = await toModule(source, 'runtime.wasm');
  // probe the ABI version with a throwaway instance (no evaluation)
  const probe = await WebAssembly.instantiate(module, { feel_host_v1: { apply: () => 0, call_host: () => 0 } });
  const rt = new FeelRuntime(module, (probe.exports as unknown as RuntimeExports).rt_abi_version());
  defaultRuntime = rt;
  return rt;
}

/**
 * Instantiate a compiled expression with the default runtime (the last one
 * loaded by {@link loadRuntime}) or `options.runtime`.
 */
export async function instantiate(
  compiled: CompileResult | Uint8Array | ArrayBuffer | WebAssembly.Module,
  options: InstanceOptions = {},
): Promise<FeelInstance> {
  const rt = options.runtime ?? defaultRuntime;
  if (!rt) throw new Error('no runtime loaded: call loadRuntime() first or pass options.runtime');
  return rt.instantiate(compiled, options);
}

/** One runtime instance + one expression-module instance. */
interface Pair {
  rt: RuntimeExports;
  expr: ExprExports;
}

/**
 * A ready-to-run compiled FEEL expression. Evaluations are synchronous and
 * reuse the instance (no recompilation, no re-instantiation); the runtime
 * releases each evaluation's values at the start of the next one.
 *
 * After an evaluation is aborted by a trap (resource limit), the instance
 * transparently replaces its runtime/module pair with a fresh one, because
 * a trap abandons runtime frames (and their shadow-stack space) mid-call.
 */
export class FeelInstance {
  /** Metadata of the compiled module. */
  readonly metadata: ModuleMetadata;
  /** Number of evaluations performed. */
  evaluations = 0;
  /** Number of times the runtime/module pair was replaced after a trap. */
  recycles = 0;
  /** When `true`, {@link FeelInstance.lastProfile} records phase timings (small overhead). */
  profile = false;
  /** Phase timings of the last evaluation (when {@link FeelInstance.profile} is on). */
  lastProfile: EvalProfile | undefined;
  private pair: Pair | undefined;
  private pending: Promise<void> | undefined;
  private dirty = false;
  private trapsSinceRecycle = 0;
  private busy = false;
  private readonly limits: Limits;
  private readonly clock: ClockOption;
  private readonly needsClock: boolean;
  private readonly host: Record<string, HostFunction>;
  private readonly decodeOptions: DecodeOptions;
  private hostDiags: EvalDiagnostic[] = [];

  /** @internal use {@link FeelRuntime.instantiate} */
  constructor(
    private readonly runtime: FeelRuntime,
    private readonly program: LoadedProgram,
    options: InstanceOptions,
  ) {
    this.metadata = program.metadata;
    this.limits = { ...DEFAULT_LIMITS, ...(options.limits ?? {}) };
    this.clock = options.clock ?? 'system';
    const deps = program.metadata.dependencies;
    // set the system clock only when the expression may read it
    this.needsClock = deps.clock || deps.uncertain.length > 0 || !deps.exact;
    this.host = { ...(options.hostFunctions ?? {}) };
    this.decodeOptions = options.decode ?? {};
  }

  /** Resolves when the instance (or its replacement after a trap) is ready. */
  async ready(): Promise<void> {
    if (!this.pair && !this.pending) this.pending = this.createAsync();
    while (this.pending) await this.pending;
  }

  private imports(cell: { rt?: RuntimeExports; expr?: ExprExports }) {
    return {
      feel_host_v1: {
        apply: (f: number, args: number): number => cell.expr!.__feel_apply(f, args),
        call_host: (f: number, args: number): number => this.callHost(cell.rt!, f, args),
      },
    };
  }

  private configure(rt: RuntimeExports, expr: ExprExports): void {
    const L = this.limits;
    const u32 = (n: number) => Math.max(0, Math.min(Math.floor(Number(n)), 0xffffffff)) | 0;
    rt.rt_set_limits(u32(L.maxValues), u32(L.maxCollection), u32(L.maxString), u32(L.maxDiags), u32(L.maxDepth));
    rt.rt_set_max_steps?.(BigInt(L.maxSteps));
    rt.rt_set_extensions(this.metadata.extensionFlags | 0);
    const c = this.program.consts;
    const p = rt.rt_alloc(c.length);
    new Uint8Array(rt.memory.buffer, p, c.length).set(c);
    const n = rt.rt_load_consts(p, c.length);
    rt.rt_free(p, c.length);
    if (n < 0) throw new FeelModuleError(`invalid constant pool: ${readRtOut(rt, rt.rt_out_ptr())}`);
    expr.__feel_set_limits(BigInt(L.maxSteps), u32(L.maxDepth));
    const clk = this.clock;
    if (clk !== 'system' && clk !== 'none') {
      const s = clk instanceof Date ? FeelDateTime.fromDate(clk).value : clk;
      if (!writeAndCall(rt, s, (ptr, len) => rt.rt_set_clock(ptr, len))) {
        throw new FeelModuleError(`${BINDING_CODES.CLOCK}: invalid clock instant '${s}'`);
      }
    }
  }

  private buildSync(): Pair {
    const cell: { rt?: RuntimeExports; expr?: ExprExports } = {};
    const rtInst = new WebAssembly.Instance(this.runtime.module, this.imports(cell));
    const rt = (cell.rt = rtInst.exports as unknown as RuntimeExports);
    const exprInst = new WebAssembly.Instance(this.program.module, { feel_rt_v1: this.importObject(rt) });
    const expr = (cell.expr = exprInst.exports as unknown as ExprExports);
    this.configure(rt, expr);
    return { rt, expr };
  }

  private async createAsync(): Promise<void> {
    try {
      const cell: { rt?: RuntimeExports; expr?: ExprExports } = {};
      const rtInst = await WebAssembly.instantiate(this.runtime.module, this.imports(cell));
      const rt = (cell.rt = rtInst.exports as unknown as RuntimeExports);
      const exprInst = await WebAssembly.instantiate(this.program.module, { feel_rt_v1: this.importObject(rt) });
      const expr = (cell.expr = exprInst.exports as unknown as ExprExports);
      this.configure(rt, expr);
      if (this.pair) this.recycles++;
      this.pair = { rt, expr };
      this.dirty = false;
      this.trapsSinceRecycle = 0;
    } finally {
      this.pending = undefined;
    }
  }

  private importObject(rt: RuntimeExports): Record<string, WebAssembly.ImportValue> {
    const m: Record<string, WebAssembly.ImportValue> = {};
    for (const imp of WebAssembly.Module.imports(this.program.module)) m[imp.name] = rt[imp.name] as WebAssembly.ImportValue;
    return m;
  }

  /** Replace the pair after a trap: synchronously when the engine allows it, else in the background. */
  private recycle(): void {
    if (!this.dirty || this.pending) return;
    try {
      this.pair = this.buildSync();
      this.recycles++;
      this.dirty = false;
      this.trapsSinceRecycle = 0;
    } catch (e) {
      if (e instanceof FeelModuleError) throw e;
      // e.g. a main-thread policy forbidding synchronous instantiation
      this.pending = this.createAsync();
      this.pending.catch(() => undefined);
    }
  }

  /** Values held by the current evaluation (`rt_eval_values`); for leak tests. */
  evalValues(): number {
    return this.pair ? this.pair.rt.rt_eval_values() : 0;
  }

  /** Bytes of linear memory of the current runtime instance. */
  get memoryBytes(): number {
    return this.pair ? this.pair.rt.memory.buffer.byteLength : 0;
  }

  /**
   * Evaluate with inputs (a context: plain object or `Map`). Inputs are
   * encoded as FEEL-JSON: JS numbers via `String(n)`; use `FeelDecimal`,
   * decimal strings in `FeelDecimal`, or `bigint` for exact values.
   */
  evaluate(inputs: FeelContext | null = {}): EvalResult {
    if (!this.profile) return this.decorate(this.run(encodeFeelJson(inputs ?? {}), null));
    const t0 = now();
    const text = encodeFeelJson(inputs ?? {});
    const t1 = now();
    return this.decorate(this.run(text, null), t1 - t0);
  }

  /** Evaluate a unary-tests module against `candidate` (bound to `?`). */
  evaluateUnary(candidate: FeelValue, inputs: FeelContext | null = {}): EvalResult {
    const t0 = this.profile ? now() : 0;
    const text = encodeFeelJson(inputs ?? {});
    const cand = encodeFeelJson(candidate);
    return this.decorate(this.run(text, cand), this.profile ? now() - t0 : 0);
  }

  /** Evaluate many input sets in turn (same instance, no recompilation). */
  evaluateBatch(inputsArray: (FeelContext | null)[]): EvalResult[] {
    return inputsArray.map((i) => this.evaluate(i));
  }

  /**
   * Evaluate with raw FEEL-JSON text in and out (no JS value encoding or
   * decoding). `candidateJson` binds `?` for unary tests.
   */
  evaluateJson(inputsJson: string, candidateJson?: string | null): RawEvalResult {
    return this.run(inputsJson.trim() === '' ? '{}' : inputsJson, candidateJson ?? null);
  }

  private decorate(r: RawEvalResult, jsEncodeMs = 0): EvalResult {
    if (r.valueJson === null) return r;
    if (!this.profile || !this.lastProfile) return { ...r, value: parseFeelJson(r.valueJson, this.decodeOptions) };
    const t0 = now();
    const value = parseFeelJson(r.valueJson, this.decodeOptions);
    this.lastProfile.jsDecodeMs = now() - t0;
    this.lastProfile.jsEncodeMs = jsEncodeMs;
    return { ...r, value };
  }

  private run(inputsJson: string, candidateJson: string | null): RawEvalResult {
    if (this.busy) throw new Error('FeelInstance is not re-entrant (evaluate called from a host function of the same instance)');
    if (this.dirty) this.recycle();
    const pair = this.pair;
    if (!pair) throw new Error('instance not ready: await instance.ready()');
    if (this.dirty && this.trapsSinceRecycle > 64) {
      return boundary(BINDING_CODES.TRAP, 'instance is being re-initialised after aborted evaluations; await instance.ready()');
    }
    this.busy = true;
    this.evaluations++;
    this.hostDiags = [];
    const rt = pair.rt;
    const prof = this.profile;
    let t = prof ? now() : 0;
    const lap = (): number => {
      const n = now();
      const d = n - t;
      t = n;
      return d;
    };
    const p: EvalProfile | undefined = prof
      ? { inputChars: inputsJson.length + (candidateJson?.length ?? 0), outputChars: 0, jsEncodeMs: 0, decodeMs: 0, bindMs: 0, evaluateMs: 0, encodeMs: 0, jsDecodeMs: 0 }
      : undefined;
    this.lastProfile = p;
    try {
      rt.rt_begin_eval();
      if (this.clock === 'system' && this.needsClock) {
        const s = new Date().toISOString();
        writeAndCall(rt, s, (p, l) => rt.rt_set_clock(p, l));
      } else if (this.clock === 'none') {
        rt.rt_set_clock(0, 0);
      }
      const inputs = writeAndCall(rt, inputsJson, (p, l) => rt.rt_decode(p, l));
      if (inputs === -2) return this.decodeFailure(rt, 'invalid inputs');
      let cand = -1;
      if (candidateJson !== null) {
        cand = writeAndCall(rt, candidateJson, (p, l) => rt.rt_decode(p, l));
        if (cand === -2) return this.decodeFailure(rt, 'invalid candidate');
      }
      if (p) p.decodeMs = lap();
      if (rt.rt_bind_inputs(inputs, cand, 3, 4, this.metadata.validateInputs ? 1 : 0) === 0) {
        return boundary(BINDING_CODES.BIND, readRtOut(rt, rt.rt_out_ptr()));
      }
      if (p) p.bindMs = lap();
      let h: number;
      try {
        h = pair.expr.evaluate();
      } catch (e) {
        const abortJson = readRtOut(rt, rt.rt_abort());
        const diagnostics = this.diagnostics(rt);
        this.dirty = true;
        this.trapsSinceRecycle++;
        const abort: AbortInfo =
          abortJson !== 'null'
            ? (JSON.parse(abortJson) as AbortInfo)
            : { kind: 'boundary', code: BINDING_CODES.TRAP, message: `trap: ${e instanceof Error ? e.message : String(e)}` };
        return { ok: false, valueJson: null, diagnostics, abort };
      }
      if (p) p.evaluateMs = lap();
      const diagnostics = this.diagnostics(rt);
      const abortJson = readRtOut(rt, rt.rt_abort());
      if (abortJson !== 'null') {
        this.dirty = true;
        return { ok: false, valueJson: null, diagnostics, abort: JSON.parse(abortJson) as AbortInfo };
      }
      const valueJson = readRtOut(rt, rt.rt_encode(h));
      if (p) {
        p.encodeMs = lap();
        p.outputChars = valueJson.length;
      }
      return { ok: true, valueJson, diagnostics };
    } finally {
      this.busy = false;
    }
  }

  private decodeFailure(rt: RuntimeExports, what: string): RawEvalResult {
    const message = readRtOut(rt, rt.rt_out_ptr());
    const abortJson = readRtOut(rt, rt.rt_abort());
    if (abortJson !== 'null') return { ok: false, valueJson: null, diagnostics: [], abort: JSON.parse(abortJson) as AbortInfo };
    return boundary(BINDING_CODES.INPUTS, `${what}: ${message}`);
  }

  private diagnostics(rt: RuntimeExports): EvalDiagnostic[] {
    const raw = JSON.parse(readRtOut(rt, rt.rt_diags())) as { code: string; site: number; message: string }[];
    const sites = this.metadata.sites;
    const out: EvalDiagnostic[] = raw.map((d) => {
      const s = sites[d.site];
      return { code: d.code, message: d.message, start: s ? s.start : 0, end: s ? s.end : 0 };
    });
    return this.hostDiags.length ? out.concat(this.hostDiags) : out;
  }

  /** `feel_host_v1.call_host`: invoke a host function by name with FEEL-JSON arguments. */
  private callHost(rt: RuntimeExports, f: number, args: number): number {
    const fJson = readRtOut(rt, rt.rt_encode(f));
    const m = /^\{"@function":"host function (.*)"\}$/s.exec(fJson);
    const name = m ? (JSON.parse(`"${m[1]}"`) as string) : '';
    const fn = name ? this.host[name] : undefined;
    if (!fn) {
      const what = name || (fJson.includes('external function') ? 'external function (java/pmml mappings are not supported by the JS binding)' : fJson);
      this.hostDiags.push({ code: 'FEEL-E360', message: `no host binding for external function '${what}'`, start: 0, end: 0 });
      return 0;
    }
    const argv = parseFeelJson(readRtOut(rt, rt.rt_encode(args)), this.decodeOptions) as FeelValue[];
    let resultJson: string;
    try {
      resultJson = encodeFeelJson(fn(argv));
    } catch (e) {
      this.hostDiags.push({ code: 'FEEL-E362', message: `host function '${name}' failed: ${e instanceof Error ? e.message : String(e)}`, start: 0, end: 0 });
      return 0;
    }
    const h = writeAndCall(rt, resultJson, (p, l) => rt.rt_decode(p, l));
    if (h === -2) {
      this.hostDiags.push({
        code: 'FEEL-E361',
        message: `host function '${name}' returned invalid FEEL-JSON: ${readRtOut(rt, rt.rt_out_ptr())}`,
        start: 0,
        end: 0,
      });
      return 0;
    }
    return h;
  }
}

function boundary(code: string, message: string): RawEvalResult {
  return { ok: false, valueJson: null, diagnostics: [], abort: { kind: 'boundary', code, message } };
}

/** Read the runtime's output buffer (pointer from the last `set_out`, length `rt_out_len`). */
function readRtOut(rt: RuntimeExports, ptr: number): string {
  return utf8dec.decode(new Uint8Array(rt.memory.buffer, ptr, rt.rt_out_len()));
}

/** Write a string into runtime memory, call `fn(ptr, len)`, free the buffer. */
function writeAndCall(rt: RuntimeExports, s: string, fn: (ptr: number, len: number) => number): number {
  const cap = Math.max(1, s.length * 3);
  const p = rt.rt_alloc(cap);
  const { written } = utf8enc.encodeInto(s, new Uint8Array(rt.memory.buffer, p, cap));
  try {
    return fn(p, written);
  } finally {
    rt.rt_free(p, cap);
  }
}

// ======================================================================
// Convenience
// ======================================================================

/** Compile (cached) + instantiate in one step; throws with diagnostics on compile errors. */
export async function compileAndInstantiate(
  compiler: FeelCompiler,
  source: string,
  compileOptions?: CompileOptions,
  options?: InstanceOptions,
): Promise<FeelInstance> {
  const out = compiler.compileCached(source, compileOptions);
  if (!out.ok) {
    const msg = out.diagnostics.map((d) => `${d.code} ${d.line}:${d.column} ${d.message}`).join('\n');
    throw new Error(`compilation failed:\n${msg}`);
  }
  return instantiate(out, options);
}

export { FeelWorker } from './feel-worker.js';
export type { WorkerProgram, WorkerEvalOptions, FeelWorkerOptions } from './feel-worker.js';
