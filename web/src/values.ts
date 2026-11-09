/**
 * FEEL values in JavaScript and the precision-preserving FEEL-JSON
 * interchange format used between JavaScript and the Wasm runtime.
 *
 * FEEL-JSON (see `crates/feel-runtime/src/json.rs`):
 * - `null`, `true`, `false` and strings are plain JSON;
 * - numbers are JSON number tokens whose *text* is the exact decimal
 *   (`1.10` keeps its scale), or `{"@number": "1.10"}`;
 * - lists are arrays, contexts are objects (entry order preserved);
 * - tagged values are objects with exactly one key starting with `@`:
 *   `@date`, `@time`, `@dateTime`, `@duration`, `@range`, `@function`
 *   (output only) and `@context` (escape for a context whose only key
 *   starts with `@`).
 *
 * Numbers never pass through binary floating point in this module: output
 * number tokens are kept as strings inside {@link FeelDecimal}, and
 * {@link parseFeelJson} is a small hand-written JSON parser (results are
 * never handed to `JSON.parse`).
 *
 * @module
 */

const NUMBER_TOKEN = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$/;
let trustedToken = false;

/**
 * An exact FEEL number (decimal128 semantics in the runtime), backed by its
 * decimal text. Scale is preserved: `new FeelDecimal("1.10").toString()` is
 * `"1.10"`.
 *
 * Construct from a string (a JSON number token, e.g. `"12345678901234567890.25"`),
 * a `bigint` (exact) or a JS `number` (converted with `String(n)`, i.e. the
 * shortest decimal that round-trips the binary double — `0.1` becomes
 * `"0.1"`, but `0.1 + 0.2` becomes `"0.30000000000000004"`).
 */
export class FeelDecimal {
  /** The decimal text (a JSON number token). */
  readonly text: string;

  constructor(value: string | number | bigint) {
    if (trustedToken && typeof value === 'string') {
      this.text = value;
      return;
    }
    let text: string;
    if (typeof value === 'bigint') text = value.toString();
    else if (typeof value === 'number') text = numberToken(value);
    else {
      text = String(value).trim();
      if (text.startsWith('+')) text = text.slice(1);
      if (!NUMBER_TOKEN.test(text)) throw new RangeError(`not a decimal number: ${JSON.stringify(value)}`);
    }
    this.text = text;
  }

  /** The exact decimal text, e.g. `"0.3"`. */
  toString(): string {
    return this.text;
  }
  /** `JSON.stringify` writes the decimal as a string (exact). */
  toJSON(): string {
    return this.text;
  }
  /** Nearest binary double (lossy). */
  toNumber(): number {
    return Number(this.text);
  }
  /** `true` when the text has no fractional part and no exponent. */
  get isInteger(): boolean {
    return /^-?[0-9]+$/.test(this.text);
  }
  /** Exact conversion of an integral value to `bigint` (throws otherwise). */
  toBigInt(): bigint {
    if (!this.isInteger) throw new RangeError(`${this.text} is not an integer`);
    return BigInt(this.text);
  }
  valueOf(): number {
    return this.toNumber();
  }
}

/** @internal Build a FeelDecimal from a token already validated by a parser. */
function trustedDecimal(text: string): FeelDecimal {
  trustedToken = true;
  try {
    return new FeelDecimal(text);
  } finally {
    trustedToken = false;
  }
}

function numberToken(n: number): string {
  if (!Number.isFinite(n)) throw new RangeError(`FEEL has no ${n}: only finite numbers can be passed`);
  // String(n) is the shortest round-trip decimal of the double; "1e+21"
  // style exponents are valid JSON number tokens and decoded exactly.
  return Object.is(n, -0) ? '0' : String(n);
}

/** Base class of the string-backed temporal wrappers. */
abstract class FeelTemporalBase {
  /** The FEEL lexical form (as in `date("…")`, `time("…")` …). */
  readonly value: string;
  constructor(value: string) {
    this.value = String(value);
  }
  toString(): string {
    return this.value;
  }
  toJSON(): string {
    return this.value;
  }
}

/** A FEEL `date` (`"2020-01-31"`), FEEL-JSON `{"@date": …}`. */
export class FeelDate extends FeelTemporalBase {
  readonly kind = 'date' as const;
}
/** A FEEL `time` (`"10:30:00"`, `"10:30:00+01:00"`, `"10:30:00@Europe/Paris"`), FEEL-JSON `{"@time": …}`. */
export class FeelTime extends FeelTemporalBase {
  readonly kind = 'time' as const;
}
/** A FEEL `date and time` (`"2020-01-31T10:30:00Z"`), FEEL-JSON `{"@dateTime": …}`. */
export class FeelDateTime extends FeelTemporalBase {
  readonly kind = 'dateTime' as const;
  /** Convert a JS `Date` (an instant) to a UTC FEEL date and time. */
  static fromDate(d: Date): FeelDateTime {
    return new FeelDateTime(d.toISOString());
  }
}
/** A FEEL duration (`"P1DT2H"` days and time, `"P1Y2M"` years and months), FEEL-JSON `{"@duration": …}`. */
export class FeelDuration extends FeelTemporalBase {
  readonly kind = 'duration' as const;
}

/**
 * A FEEL range. `start`/`end` are `undefined` for an unbounded side.
 * `notEqual`/`equal` mark the unary-test forms `!= x` / `= x` that the
 * runtime represents as ranges.
 */
export class FeelRange {
  constructor(
    readonly start: FeelValue | undefined,
    readonly end: FeelValue | undefined,
    readonly startIncluded: boolean = start !== undefined,
    readonly endIncluded: boolean = end !== undefined,
    readonly notEqual: boolean = false,
    readonly equal: boolean = false,
  ) {}
  toString(): string {
    const s = this.start === undefined ? '' : feelValueToString(this.start);
    const e = this.end === undefined ? '' : feelValueToString(this.end);
    return `${this.startIncluded ? '[' : '('}${s}..${e}${this.endIncluded ? ']' : ')'}`;
  }
}

/** A function value returned by an evaluation (output only; not callable). */
export class FeelFunction {
  constructor(readonly description: string) {}
  toString(): string {
    return `function(${this.description})`;
  }
}

/** A FEEL context: a `Map` (order and arbitrary keys preserved) or a plain object. */
export type FeelContext = Map<string, FeelValue> | { [key: string]: FeelValue };

/**
 * A FEEL value as seen from JavaScript. Inputs may also use JS `number`,
 * `bigint` and `Date`; outputs use {@link FeelDecimal} for every number.
 */
export type FeelValue =
  | null
  | undefined
  | boolean
  | string
  | number
  | bigint
  | Date
  | FeelDecimal
  | FeelDate
  | FeelTime
  | FeelDateTime
  | FeelDuration
  | FeelRange
  | FeelFunction
  | FeelValue[]
  | FeelContext;

/** Options for decoding FEEL-JSON results into JS values. */
export interface DecodeOptions {
  /**
   * `"object"` (default): contexts become plain objects (note: JS orders
   * integer-like keys first). `"map"`: contexts become `Map`s, preserving
   * entry order exactly.
   */
  contexts?: 'object' | 'map';
  /**
   * `"decimal"` (default): numbers become exact {@link FeelDecimal}s.
   * `"number"`: numbers become JS doubles (lossy).
   */
  numbers?: 'decimal' | 'number';
}

// ---------------------------------------------------------------- encoding

/**
 * Encode a JS value as FEEL-JSON text.
 *
 * - `number` → `String(n)` (shortest round-trip decimal of the double; use
 *   {@link FeelDecimal}, a decimal string wrapped in `FeelDecimal`, or a
 *   `bigint` for exact values); `NaN`/`±Infinity` throw.
 * - `bigint`, {@link FeelDecimal} → exact number tokens.
 * - JS `Date` → `{"@dateTime": "<ISO-8601 UTC>"}`.
 * - `undefined` → `null`.
 * - plain objects and `Map`s → contexts (a context with a single `@…` key is
 *   escaped as `{"@context": …}`).
 */
export function encodeFeelJson(v: FeelValue): string {
  return enc(v, 0);
}

const SAFE_STRING = /^[\x20\x21\x23-\x5b\x5d-\x7e]*$/;

function str(s: string): string {
  // fast path for plain ASCII without quotes/backslashes/control characters
  return s.length < 64 && SAFE_STRING.test(s) ? `"${s}"` : JSON.stringify(s);
}

function enc(v: FeelValue, depth: number): string {
  if (v === null || v === undefined) return 'null';
  switch (typeof v) {
    case 'boolean':
      return v ? 'true' : 'false';
    case 'string':
      return str(v);
    case 'number':
      return numberToken(v);
    case 'bigint':
      return v.toString();
    case 'object':
      break;
    default:
      throw new TypeError(`cannot pass a ${typeof v} to FEEL`);
  }
  if (depth > 256) throw new RangeError('value nested too deeply for FEEL-JSON');
  if (v instanceof FeelDecimal) return v.text;
  if (Array.isArray(v)) {
    let out = '[';
    for (let i = 0; i < v.length; i++) {
      if (i > 0) out += ',';
      out += enc(v[i], depth + 1);
    }
    return out + ']';
  }
  if (v instanceof Map) {
    let out = '{';
    let n = 0;
    let first = '';
    for (const [k, x] of v) {
      if (n > 0) out += ',';
      else first = k;
      out += str(k) + ':' + enc(x, depth + 1);
      n++;
    }
    out += '}';
    return n === 1 && first.charCodeAt(0) === 64 ? `{"@context":${out}}` : out;
  }
  if (v instanceof FeelTemporalBase) {
    const tag = v instanceof FeelDate ? '@date' : v instanceof FeelTime ? '@time' : v instanceof FeelDateTime ? '@dateTime' : '@duration';
    return `{"${tag}":${str(v.value)}}`;
  }
  if (v instanceof Date) {
    if (Number.isNaN(v.getTime())) throw new RangeError('invalid Date');
    return `{"@dateTime":"${v.toISOString()}"}`;
  }
  if (v instanceof FeelFunction) return `{"@function":${str(v.description)}}`;
  if (v instanceof FeelRange) {
    let out = '{"@range":{';
    if (v.start !== undefined) out += `"start":${enc(v.start, depth + 1)},`;
    if (v.end !== undefined) out += `"end":${enc(v.end, depth + 1)},`;
    out += `"startIncluded":${v.startIncluded},"endIncluded":${v.endIncluded}`;
    if (v.notEqual) out += ',"notEqual":true';
    if (v.equal) out += ',"equal":true';
    return out + '}}';
  }
  const o = v as Record<string, FeelValue>;
  const keys = Object.keys(o);
  let out = '{';
  for (let i = 0; i < keys.length; i++) {
    if (i > 0) out += ',';
    out += str(keys[i]) + ':' + enc(o[keys[i]], depth + 1);
  }
  out += '}';
  return keys.length === 1 && keys[0].charCodeAt(0) === 64 ? `{"@context":${out}}` : out;
}

// ---------------------------------------------------------------- decoding

/** Thrown by {@link parseFeelJson} on malformed input. */
export class FeelJsonError extends SyntaxError {
  constructor(
    message: string,
    readonly offset: number,
  ) {
    super(`invalid FEEL-JSON at offset ${offset}: ${message}`);
    this.name = 'FeelJsonError';
  }
}

/**
 * Parse FEEL-JSON text into JS values without ever converting number
 * tokens through binary floating point (unless `numbers: "number"`).
 */
export function parseFeelJson(text: string, options: DecodeOptions = {}): FeelValue {
  const p = new Parser(text, options.contexts === 'map', options.numbers === 'number');
  p.ws();
  const v = p.value(0);
  p.ws();
  if (p.i !== text.length) p.fail('trailing characters');
  return v;
}

/** Validate FEEL-JSON syntax (cheaply) without building values. Returns an error message or `null`. */
export function checkFeelJson(text: string): string | null {
  try {
    parseFeelJson(text);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
}

class Parser {
  i = 0;
  constructor(
    readonly s: string,
    readonly maps: boolean,
    readonly doubles: boolean,
  ) {}

  fail(m: string): never {
    throw new FeelJsonError(m, this.i);
  }

  ws(): void {
    const s = this.s;
    let i = this.i;
    for (;;) {
      const c = s.charCodeAt(i);
      if (c === 32 || c === 10 || c === 13 || c === 9) i++;
      else break;
    }
    this.i = i;
  }

  value(depth: number): FeelValue {
    if (depth > 600) this.fail('nesting too deep');
    const s = this.s;
    const c = s.charCodeAt(this.i);
    switch (c) {
      case 34: // "
        return this.string();
      case 123: // {
        return this.object(depth);
      case 91: {
        // [
        this.i++;
        const items: FeelValue[] = [];
        this.ws();
        if (s.charCodeAt(this.i) === 93) {
          this.i++;
          return items;
        }
        for (;;) {
          this.ws();
          items.push(this.value(depth + 1));
          this.ws();
          const d = s.charCodeAt(this.i++);
          if (d === 44) continue;
          if (d === 93) return items;
          this.i--;
          this.fail("expected ',' or ']'");
        }
      }
      case 116: // t
        return this.lit('true', true);
      case 102: // f
        return this.lit('false', false);
      case 110: // n
        return this.lit('null', null);
      default:
        if (c === 45 || (c >= 48 && c <= 57)) return this.number();
        return this.fail(Number.isNaN(c) ? 'unexpected end of input' : 'unexpected character');
    }
  }

  lit<T>(word: string, v: T): T {
    if (this.s.startsWith(word, this.i)) {
      this.i += word.length;
      return v;
    }
    return this.fail('invalid literal');
  }

  number(): FeelValue {
    const s = this.s;
    const start = this.i;
    let i = start;
    if (s.charCodeAt(i) === 45) i++;
    const digits = () => {
      const d0 = i;
      while (i < s.length) {
        const c = s.charCodeAt(i);
        if (c < 48 || c > 57) break;
        i++;
      }
      if (i === d0) {
        this.i = i;
        this.fail('invalid number');
      }
    };
    if (s.charCodeAt(i) === 48) i++;
    else digits();
    if (s.charCodeAt(i) === 46) {
      i++;
      digits();
    }
    const e = s.charCodeAt(i);
    if (e === 101 || e === 69) {
      i++;
      const sg = s.charCodeAt(i);
      if (sg === 43 || sg === 45) i++;
      digits();
    }
    this.i = i;
    const tok = s.slice(start, i);
    return this.doubles ? Number(tok) : trustedDecimal(tok);
  }

  string(): string {
    const s = this.s;
    let i = ++this.i;
    // fast path: no escapes
    for (;;) {
      const c = s.charCodeAt(i);
      if (c === 34) {
        const r = s.slice(this.i, i);
        this.i = i + 1;
        return r;
      }
      if (c === 92 || Number.isNaN(c)) break;
      i++;
    }
    let out = s.slice(this.i, i);
    for (;;) {
      const c = s.charCodeAt(i);
      if (Number.isNaN(c)) {
        this.i = i;
        this.fail('unterminated string');
      }
      if (c === 34) {
        this.i = i + 1;
        return out;
      }
      if (c === 92) {
        const e = s.charAt(i + 1);
        i += 2;
        switch (e) {
          case '"':
            out += '"';
            break;
          case '\\':
            out += '\\';
            break;
          case '/':
            out += '/';
            break;
          case 'b':
            out += '\b';
            break;
          case 'f':
            out += '\f';
            break;
          case 'n':
            out += '\n';
            break;
          case 'r':
            out += '\r';
            break;
          case 't':
            out += '\t';
            break;
          case 'u': {
            const h = s.slice(i, i + 4);
            if (!/^[0-9a-fA-F]{4}$/.test(h)) {
              this.i = i;
              this.fail('bad \\u escape');
            }
            out += String.fromCharCode(parseInt(h, 16));
            i += 4;
            break;
          }
          default:
            this.i = i - 1;
            this.fail('bad escape');
        }
        continue;
      }
      const j = i;
      while (i < s.length) {
        const d = s.charCodeAt(i);
        if (d === 34 || d === 92) break;
        i++;
      }
      out += s.slice(j, i);
    }
  }

  object(depth: number): FeelValue {
    const s = this.s;
    this.i++;
    const keys: string[] = [];
    const vals: FeelValue[] = [];
    this.ws();
    if (s.charCodeAt(this.i) === 125) {
      this.i++;
    } else {
      for (;;) {
        this.ws();
        if (s.charCodeAt(this.i) !== 34) this.fail('expected object key');
        keys.push(this.string());
        this.ws();
        if (s.charCodeAt(this.i) !== 58) this.fail("expected ':'");
        this.i++;
        this.ws();
        vals.push(this.value(depth + 1));
        this.ws();
        const d = s.charCodeAt(this.i++);
        if (d === 44) continue;
        if (d === 125) break;
        this.i--;
        this.fail("expected ',' or '}'");
      }
    }
    if (keys.length === 1 && keys[0].charCodeAt(0) === 64) {
      const t = this.tagged(keys[0], vals[0]);
      if (t !== NOT_TAGGED) return t;
    }
    if (this.maps) {
      const m = new Map<string, FeelValue>();
      for (let k = 0; k < keys.length; k++) m.set(keys[k], vals[k]);
      return m;
    }
    const o: { [key: string]: FeelValue } = {};
    for (let k = 0; k < keys.length; k++) {
      if (keys[k] === '__proto__') {
        Object.defineProperty(o, '__proto__', { value: vals[k], enumerable: true, writable: true, configurable: true });
      } else o[keys[k]] = vals[k];
    }
    return o;
  }

  tagged(tag: string, v: FeelValue): FeelValue | typeof NOT_TAGGED {
    const str = typeof v === 'string' ? v : undefined;
    switch (tag) {
      case '@number':
        if (str !== undefined) return this.doubles ? Number(str) : new FeelDecimal(str);
        if (v instanceof FeelDecimal || typeof v === 'number') return v;
        break;
      case '@date':
        if (str !== undefined) return new FeelDate(str);
        break;
      case '@time':
        if (str !== undefined) return new FeelTime(str);
        break;
      case '@dateTime':
      case '@date and time':
        if (str !== undefined) return new FeelDateTime(str);
        break;
      case '@duration':
        if (str !== undefined) return new FeelDuration(str);
        break;
      case '@function':
        return new FeelFunction(str ?? '');
      case '@context':
        if (v !== null && typeof v === 'object' && !Array.isArray(v)) return v;
        break;
      case '@range': {
        const get = (k: string): FeelValue | undefined =>
          v instanceof Map ? v.get(k) : v && typeof v === 'object' && k in v ? (v as Record<string, FeelValue>)[k] : undefined;
        if (v !== null && typeof v === 'object') {
          const start = get('start');
          const end = get('end');
          const si = get('startIncluded');
          const ei = get('endIncluded');
          return new FeelRange(
            start,
            end,
            typeof si === 'boolean' ? si : start !== undefined,
            typeof ei === 'boolean' ? ei : end !== undefined,
            get('notEqual') === true,
            get('equal') === true,
          );
        }
        break;
      }
      default:
        return NOT_TAGGED;
    }
    return this.fail(`invalid payload for ${tag}`);
  }
}

const NOT_TAGGED: unique symbol = Symbol('not tagged');

// ---------------------------------------------------------------- display

/** A compact FEEL-ish rendering of a value (for UIs and logs). */
export function feelValueToString(v: FeelValue): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'string') return JSON.stringify(v);
  if (typeof v === 'number' || typeof v === 'bigint' || typeof v === 'boolean') return String(v);
  if (v instanceof FeelDecimal) return v.text;
  if (v instanceof FeelDate) return `date("${v.value}")`;
  if (v instanceof FeelTime) return `time("${v.value}")`;
  if (v instanceof FeelDateTime) return `date and time("${v.value}")`;
  if (v instanceof FeelDuration) return `duration("${v.value}")`;
  if (v instanceof Date) return `date and time("${v.toISOString()}")`;
  if (v instanceof FeelRange || v instanceof FeelFunction) return v.toString();
  if (Array.isArray(v)) return `[${v.map(feelValueToString).join(', ')}]`;
  const entries = v instanceof Map ? [...v.entries()] : Object.entries(v);
  return `{${entries.map(([k, x]) => `${/^[A-Za-z_?][\w ?]*$/.test(k) ? k : JSON.stringify(k)}: ${feelValueToString(x)}`).join(', ')}}`;
}
