# Runtime ABI (`feel_rt_v1`) and module format

## Generated module

| Item | Contents |
|---|---|
| imports | functions from module **`feel_rt_v1`** named `f_<name>`, only those used; no memory, table or global imports |
| memory | none (the module never touches linear memory) |
| table 0 | funcref table: one entry per FEEL function literal, in definition order |
| globals | 0 `fuel: i64`, 1 `depth: i32`, 2 `max_fuel: i64`, 3 `max_depth: i32` (all mutable, not exported) |
| export `evaluate() -> i32` | resets fuel and depth, then evaluates and returns a value handle |
| export `__feel_apply(f, args) -> i32` | calls the user function `f` with an already-bound argument list (used by runtime callbacks) |
| export `__feel_set_limits(max_fuel: i64, max_depth: i32)` | call once after instantiation |
| custom section `feel.meta` | JSON metadata (below) |
| custom section `feel.consts` | the constant pool, a FEEL-JSON array; constant *i* has handle `3 + i` |

User functions have the type `(env: i32, args: i32) -> i32`. In the prologue the function decrements fuel, increments depth and checks it, then loads parameters from `args` and captures from `env`.

### Metadata (`feel.meta`)

```json
{"format":"feel-wasm-module","abiVersion":1,"abiModule":"feel_rt_v1",
 "compilerVersion":"0.1.0","runtimeVersion":"0.1.0","dialect":"feel-1.5","entry":"expression",
 "extensions":[],"extensionFlags":0,
 "inputs":[{"name":"applicant","slot":0,"type":"context<age: number>"}],"candidateSlot":null,
 "resultType":"boolean","constCount":5,"functionCount":0,
 "imports":["slot","path","ge"],
 "sites":[{"start":0,"end":0,"what":"module"},{"start":0,"end":13,"what":"path"}],
 "dependencies":{"inputs":[{"name":"applicant","whole":false,"paths":[["age"]]}],"clock":false,"hostFunctions":[],"exact":true,"uncertain":[]},
 "optimized":true,"foldedConstants":0,"fusedFilters":0,
 "sourceSha256":"…","cacheKey":"…","validateInputs":true}
```

Hosts **must** reject a module whose `abiModule`/`abiVersion` they do not provide. `feel_compiler::read_module_sections` and the TypeScript loader both fail with "incompatible module: compiled for runtime ABI feel_rt_v1 v9, this runtime provides feel_rt_v1 v1". An import that is not a known `feel_rt_v1` function is also rejected. Any incompatible change to a helper's meaning or signature bumps the version, which also renames the import module (`feel_rt_v2`).

## Conventions

* **Handles** are `i32` indices into the instance's value arena. `0` is null, `1` true, `2` false, `3..` constants, then evaluation values. Null is always handle 0, so `x = null` compiles to `i32.eqz`.
* **`-1` (ABSENT)** means "no value", distinct from null: a missing key in `item_field`, a null iteration source in `as_iter`/`filter_list`, an invocation that must yield null in `bind_args`.
* **Tri-state:** `0` false, `1` true, `2` null.
* **`site`:** index into `meta.sites`, attached to evaluation diagnostics.
* **Ownership:** all values belong to the runtime instance. The generated module holds only handles, in locals. Lists and contexts are built through `list_new`/`list_push` and `ctx_new`/`ctx_put` on freshly allocated handles that nothing else references yet. Published values are immutable. `begin_eval` (host side) releases everything allocated by the previous evaluation; constants are never released.
* **Decimals and temporal values** never cross the ABI as numbers. They exist only as handles; hosts exchange them as FEEL-JSON text.

## Functions (generated code → runtime)

| Function | Signature | Meaning |
|---|---|---|
| `slot(i)` | → h | value bound to input slot *i* |
| `truth(h)` / `bool_of(t)` | → t / → h | handle ↔ tri-state (normally inlined) |
| `add sub mul div pow (a,b,site)` | → h | Tables 57, 59, 60 |
| `neg(a,site)` | → h | Table 62 |
| `eq ne lt le gt ge (a,b,site)` | → t | Tables 52–54 |
| `in_value(c,v,uses_q,site)` | → t | `c in v` (Table 55): range containment, list membership, boolean result of a `?` test, otherwise `=` |
| `range(s,e,flags,site)` | → h | range value; flags: 1 start incl., 2 end incl., 4/8 start/end unbounded, 16 `!= e`, 32 `= e` |
| `list_new(cap)`, `list_push(l,v)`, `list_len(l)`, `list_get(l,i)`, `list_snapshot(l)` | | list building and access (`list_len` returns -1 for a non-list) |
| `ctx_new(cap)`, `ctx_put(c,k,v)` | | context building (k = string constant handle) |
| `path(v,k,site)` | → h | selection, properties, projection over lists (Tables 64–67) |
| `select_or_null(v,k)` | → h | selection without list projection or diagnostics (fused filters) |
| `item_field(item,k)` | → h / -1 | implicit names inside filters |
| `index(l,n,site)` | → h | `l[n]`, Table 68 |
| `as_iter(v,site)`, `iter_range(a,b,site)`, `filter_list(v)` | → h / -1 | iteration sources (10.3.2.14) and the singleton conversion for filters |
| `is_number(h)` | → 0/1 | numeric filter = index |
| `make_closure(idx,env,sig)` | → h | closure value |
| `fn_kind(f)`, `fn_index(f)`, `fn_env(f)` | | closure dispatch |
| `bind_args(f,args,named,site)` | → h / -1 | arity, names, implicit conversions, parameter types (Table 63) |
| `call_value(f,args,named,site)` | → h | builtin, external or host function values |
| `call_builtin(id,args,site)`, `call_builtin_named(id,ctx,site)` | → h | builtin catalog (10.3.4) |
| `builtin_fn(id)`, `host_fn(name,sig)`, `external_fn(mapping,sig)` | → h | function values |
| `instance_of(v,ty)` | → t | Table 61 (`ty` = type-string constant) |
| `coerce(v,ty,site)` | → h | binding conversion: implicit conversions, then "conforms to" |
| `not_bool(h,site)` | → t | `not()` of a handle |
| `cond_diag(h,site)` | | diagnostic hook (reserved) |
| `fuel_exhausted()`, `depth_exceeded()` | | record a limit abort (then trap) |

## Host-facing functions (`runtime.wasm` exports)

| Export | Use |
|---|---|
| `rt_alloc(len)`, `rt_free(ptr,len)` | buffers for text written into runtime memory |
| `rt_load_consts(ptr,len)` → n / -1 | load the module's `feel.consts` once per instance |
| `rt_set_limits(values, collection, string, diags, depth)`, `rt_set_max_steps(i64)` | resource limits |
| `rt_set_extensions(flags)` | `meta.extensionFlags` |
| `rt_set_clock(ptr,len)` → 1/0 | the instant `now()`/`today()` observe (length 0 = no clock) |
| `rt_begin_eval()` | release the previous evaluation and clear diagnostics and abort |
| `rt_decode(ptr,len)` → h / -2 | FEEL-JSON → handle |
| `rt_bind_inputs(inputs, candidate_or_-1, 3, 4, validate)` → 1/0 | bind slots by name; validate declared types at the boundary |
| `rt_encode(h)` → ptr | FEEL-JSON of a value (with `rt_out_len()`) |
| `rt_diags()`, `rt_abort()` → ptr | diagnostics JSON `[{code,site,message}]`; abort JSON `{kind,code,message}` or `null` |
| `rt_out_ptr()`, `rt_out_len()` | the last output buffer (also holds error messages) |
| `rt_eval_values()` | values held by the current evaluation (leak tests) |
| `__stack_pointer` (global) | shadow-stack pointer, so a host can restore it after a trap |

`runtime.wasm` imports **`feel_host_v1.apply(f,args)`**, which the host routes to the paired module's `__feel_apply`, and **`feel_host_v1.call_host(f,args)`**, which resolves external and host functions. The binding key is the host function name, or `java:<class>.<signature>` / `pmml:<document>#<model>`. `rt_encode(f)` reveals the key.

## Failure propagation

| Outcome | Mechanism | Host sees |
|---|---|---|
| FEEL null with evaluation diagnostics | ordinary return; diagnostics recorded (bounded by `max_diags`) | `result: "null"`, `diagnostics: [FEEL-E3xx]` |
| Resource limit (`FEEL-L401` values, `L402` collection, `L403` string, `L404` steps, `L405` depth) | the runtime records an `Abort`; the binding layer traps immediately (`unreachable` in runtime.wasm, a wasmi error natively) | `abort: {kind: "limit", code}`, no result |
| Boundary error (`FEEL-B5xx`): bad FEEL-JSON, a declared input type violated, an unbound slot, a trap in a callback | `Abort::Boundary` or a host-side check | `abort: {kind: "boundary", code}` |
| Cancellation (browser) | the Web Worker is terminated on timeout and restarted | `abort: {kind: "cancelled", code: "FEEL-B590"}` |

An instance stays usable after an abort, because the next `begin_eval` clears it. The browser binding nevertheless replaces the runtime/module pair after a trap (about 1 ms). A trap unwinds runtime frames without restoring the shadow stack; exporting `__stack_pointer` makes in-place recovery possible too.

## Isolation and reproducibility

Each `instantiate` creates its own runtime instance with its own arena, limits, clock and host bindings, paired with its own module instance. Nothing is shared between instances except the immutable compiled code.

Evaluation is deterministic given four things: the source and options (captured by the cache key), the inputs, the **clock** (passed per evaluation; `System` samples it once per evaluation), and the **timezone data** (chrono-tz with IANA tzdb **2025b**, embedded at build time and reported by `fw_version().tzdb`). Host callbacks are the only other source of variation, and they are explicit capabilities.
