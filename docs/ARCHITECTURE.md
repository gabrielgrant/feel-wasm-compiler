# Architecture

```
FEEL source ──► lexer ──► parser ──► AST ──► [checker] ──► lowering ──► IR ──► [optimizer] ──► Wasm emitter ──► module.wasm
 (text)        tokens   scope-aware  spans   types,        names,       typed                    code + feel.meta +
                        multi-word           diagnostics   closures,    (handle vs               feel.consts
                        names                              constants    tri-state)
                                                                                                   │ imports feel_rt_v1.*
DMN XML ──► model reader ──► DMN lowering (decision tables, hit policies, DRG, imports) ──► AST ──┘
                                                                                                   ▼
                                                     shared runtime (feel-runtime): values, decimals,
                                                     temporal, builtins, FEEL-JSON, limits, diagnostics
                                                     ├── natively: bound as wasmi host functions (feel-host)
                                                     └── as runtime.wasm (feel-runtime-wasm) for browsers
```

The same crates build natively and for `wasm32-unknown-unknown`. `compiler.wasm` (`feel-web`) is `feel-compiler` plus a C ABI, so native and in-browser compilation share one front end and one back end.

## Front end

**Lexer** (`lexer.rs`, rules 25–37 and 61–65). It produces word, number, string and symbol tokens with byte spans and records whether white space came before each token. String escapes follow rule 64: `\uXXXX`, `\UXXXXXX`, surrogate pairs. An unknown escape such as `\d` is kept verbatim, as the reference implementations do for regular-expression patterns.

**Parser** (`parser.rs`) is recursive descent with precedence climbing, ordered per rule 2 and its additional rules:

* `for` / `if` / quantified / `function` are lowest, then `or`, `and`, comparison (`= != < <= > >=`, `between`, `in`, `instance of`), `+ -`, `* /`, then `**`;
* `**` is left-associative and binds looser than unary `-`, so `-4 ** 2 = 16`. The parser warns (`FEEL-W201`);
* postfix path, filter and invocation bind tightest.

*Names* are token sequences (10.3.1.4). At each name position the parser takes the **longest match against the names in scope** (10.3.1.6): context entries defined so far, function parameters, `for`/`some`/`every` variables, `partial`, `item`, `?`, schema inputs and host functions, DRG element names from the DMN layer, and the builtins. If nothing matches, it falls back to a run of plain (non-keyword) words. So `Monthly Salary * 12` works without a schema, and `Profit and loss` works when the name is known. After `.`, field names are matched against known fields from the schema or item definitions. Unary tests (rule 15), intervals with all four bracket forms (`]a..b[` included) and range values (`< e`, `= e`, `!= e`, intervals) are first-class nodes.

**Checker** (`check.rs`, optional) runs on the AST with a schema environment and works like CEL's `check`:

* it resolves names and field paths;
* it validates operators against Tables 57–60 and builtin calls against the catalog's signatures (arity, named parameters, argument types);
* it infers result types;
* it narrows nullability through guards: `x != null`, `x = null` in else branches, conjunctions, and true comparisons;
* it checks the expected result type and the unary-test candidate type.

Findings have stable codes and spans; suggestions use edit distance. A **strict** policy makes `C1xx` findings errors; a **permissive** policy makes them warnings. With **off**, the checker does not run. The checker never feeds the code generator, so it cannot change the values a program produces (`checker_never_changes_values` test).

## IR and lowering

`lower.rs` resolves every name and turns the AST into the IR (`ir.rs`).

* **Inputs** become *slots*: the module's input list is fixed at compile time and the host binds them by name. This is "typed slot access": there is no name lookup at run time.
* **Locals** are let-bound values: context entries (evaluated in order, 10.3.2.6), iteration variables, parameters and the candidate `?`.
* **Closure conversion.** Each FEEL function literal becomes a separate Wasm function with signature `(env, args) -> result`. Free variables of enclosing functions are captured into an environment list, transitively through nested functions. Function-valued context entries can refer to themselves: the closure is created first and its environment filled afterwards, which allows recursion.
* **Filter scopes.** Inside a filter, a name that is not bound lexically resolves to `ItemField{item, key, fallback}`, because the item's context entries shadow outer names (10.3.2.5). `item` itself resolves to the item unless the item has an `item` entry.
* **Builtins** are called through `call_builtin(id, args)`. `not(x)` is lowered to inline tri-state logic.
* **Constants** (strings, exact numbers, temporal literals, keys, type and signature descriptors) go into a constant pool. Slot names and types are reserved constants 0–2.
* **Sites.** Every operation that can produce an evaluation diagnostic gets a site id that maps to its source span.

IR nodes produce either a **value handle** or a **tri-state boolean** (0 false, 1 true, 2 null). Comparisons, `and`, `or`, `not`, `in`, `instance of` and quantifiers stay tri-state, so conditions never allocate a boolean value.

## Code generation

`codegen.rs` emits a module with **no linear memory**. It contains:

* imports of only the runtime helpers the expression uses;
* `evaluate`, `__feel_apply`, `__feel_invoke` and `__feel_set_limits`, plus one function per FEEL function;
* a funcref table for closures;
* four globals (fuel, depth and their maxima);
* two custom sections: `feel.meta` (metadata JSON) and `feel.consts` (the constant pool as FEEL-JSON).

The following are generated Wasm code, not runtime calls:

* `if` / `then` / `else`;
* `and` / `or` per Table 50, short-circuiting;
* `not`, `x = null`, and the conversions between tri-state and handle;
* loops for filters (including the numeric-index case of Table 68), multi-variable `for` with `partial`, and `some`/`every` with early exit;
* `in` and unary-test disjunctions;
* closure creation and dispatch (`call_indirect`);
* step and depth accounting.

Decimal arithmetic, comparisons of non-trivial values, collection construction, temporal operations and builtins are calls to runtime helpers.

## Runtime

`feel-runtime` keeps values in a per-instance **arena**, addressed by `i32` handles.

* **Fixed region:** null, true, false and the constant pool. It survives across evaluations.
* **Evaluation region:** truncated by `begin_eval`, so repeated evaluations do not accumulate memory. Capacity is also released after unusually large evaluations.

Strings are `Rc<str>`, lists are vectors of handles, and contexts are ordered key/handle vectors, so key presence is distinct from a null value. Closures hold their table index, an environment handle and a signature constant.

The runtime implements:

* the operator semantics (Tables 50–69);
* the full builtin catalog (10.3.4), described by a parsable signature table that the checker also uses;
* XPath regular expressions on top of `fancy-regex` (flags `s m i x q`, back-references, `\p{IsBlock}` for common blocks);
* runtime types for `instance of` and conformance, plus the implicit conversions of 10.3.2.9.4;
* FEEL-JSON;
* limits, diagnostics and the clock capability.

Higher-order builtins (`sort`, `list replace` with a function) call back into the generated module through a `Cx` trait. Natively this is a wasmi re-entrant call; in the browser it is the `feel_host_v1.apply` import, routed to the module's `__feel_apply`.

## Optimizations

`opt.rs` is selectable with `CompileOptions::optimize`; the TCK and the vectors run both ways.

* **Constant folding** applies the runtime's own operation to constant operands, one operation at a time in source order, so rounding order is identical. A fold is applied only if the operation produced no diagnostic and a scalar result, which preserves diagnostics and null behaviour. Constant conditions in `if`/`and`/`or` are simplified only when the result mode is preserved.
* **Filter/projection fusion** turns `list[pred].key` into one loop with a non-mapping `select_or_null`. The result is identical to filtering and then projecting, including nested lists and missing keys.
* **Typed slot access / slot caching:** inputs are read once into locals.
* **Tri-state lowering of boolean logic**, as described above.

Equivalence is checked by differential tests: 407 vectors, comparing results *and* diagnostic codes, and the full TCK with `--no-opt`.

## DMN integration layer

`feel-dmn` reads DMN 1.1–1.5 XML and lowers a model, with its imports, to **one FEEL context**. Its entries are:

* imported models: nested under the import name, or merged for unnamed imports;
* input data: slots converted to their typeRef, with allowed-value checks;
* BKMs: functions, whose results are converted to the return type of their function item;
* decision services: functions whose parameters are the input data followed by the input decisions (10.4);
* decisions, in requirement order, each converted to its typeRef (10.3.2.9.4, "conforms to").

Inputs of imported models are keyed by namespace, so one binding serves every import path (DMN16-50). The whole model compiles to one Wasm module, which is instantiated once and evaluated for every test case.

**Decision tables** are lowered in the DMN layer (hit-policy orchestration is not part of the FEEL core):

1. input expressions are bound once;
2. each input entry becomes a unary-tests check of its column's value; a `-` with input values tests those values;
3. matched rule outputs are collected in rule order;
4. U/A/P/F/R/O/C and C+/C#/C</C> are expressed with FEEL builtins.

Boxed conditional, filter, for and some/every follow the "MUST resolve to a boolean / collection" rules of 10.2.1.8–10.

`host_dt.rs` evaluates the same tables a second way. Every cell is compiled separately: input expressions as expressions, input entries as **unary-test modules** fed the column's value as the candidate, and outputs as expressions. The hit policy is applied in Rust. The two strategies agree on every TCK decision table (`feel-tck --host-dt`).
