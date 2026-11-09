# Conformance

## Baseline (pinned)

| Artifact | Pin |
|---|---|
| Specification | **OMG Decision Model and Notation (DMN) 1.5**, <https://www.omg.org/spec/DMN/1.5> (PDF retrieved from <https://www.omg.org/spec/DMN/1.5/PDF>, SHA-256 `196c80d0f483446a6e616ba096be5d44be0f8d1820684f59a41c80bfefc52e0f`) |
| Test suite | **DMN TCK** <https://github.com/dmn-tck/tck>, revision `5cd912f281831bc535e710aba435e16de198028a` (2026-11-04); fetched by `scripts/fetch-tck.sh` |
| TCK artifacts used | `TestCases/compliance-level-2/**` and `TestCases/compliance-level-3/**`: every `*.dmn` model and every `*-test-*.xml` file. `TestCases/non-compliant/**` is run for information only |
| Time-zone data | IANA tzdb **2025b** (chrono-tz 0.10.4) |

### Clauses implemented

* **10.3.1 Syntax:** grammar rules 1–67 (10.3.1.2), literals and datatypes (10.3.1.3), tokens, names and white space (10.3.1.4), qualified names (10.3.1.5), and ambiguity resolution (10.3.1.6).
* **10.3.2 Semantics:**
  * semantic domain; equality, identity and equivalence (10.3.2.1–2);
  * every datatype of 10.3.2.3.1–8;
  * ternary logic (10.3.2.4); lists and filters (10.3.2.5); contexts (10.3.2.6); ranges (10.3.2.7); functions (10.3.2.8);
  * type equivalence, conformance and conversions (10.3.2.9);
  * decision-table semantics (10.3.2.10), via the DMN layer;
  * scope and the context stack (10.3.2.11);
  * built-in, user-defined and externally defined functions, positional and named parameters (10.3.2.13);
  * `for` (10.3.2.14);
  * semantic mappings, Tables 48–69 (10.3.2.15); error handling (10.3.2.16).
* **10.3.4 Built-in functions:** conversion, boolean, string, list, numeric, date and time, range (Table 78), temporal, sort, context and miscellaneous functions (`now`, `today`). This is the complete Table 72–82 catalog.
* **Used inside DMN** (the integration layer, to the extent needed to give FEEL its DMN context):
  * decision tables, 8.1–8.3: unary tests on input values, input values for `-`, all hit policies and aggregations;
  * boxed expressions, 10.2.1 and 10.5: literal, context, invocation, relation, list, function definition (FEEL, Java), conditional, filter, for, some, every;
  * the DRG, 6.3 and 7.3: input data, decisions, BKMs, decision services (10.4), item definitions (typeRef, collections, components, allowed values, function items), and imports (named, unnamed, nested).

## The claim

> For the dialect `feel-1.5`, the compiler implements the full FEEL expression and unary-tests language of DMN 1.5 clause 10.3, including the complete standard builtin catalog of 10.3.4. Programs are compiled to WebAssembly, and FEEL in DMN models is evaluated through the integration layer. **All 3,511 applicable result nodes** of the pinned DMN TCK (compliance levels 2 and 3) pass when evaluated by generated Wasm, both with and without optimizations. There are **no skipped or out-of-scope cases**.

Supporting evidence:

| Check | Command | Result |
|---|---|---|
| TCK, optimized | `scripts/run-tck.sh` → `reports/tck.json` | 3,511 total, 3,511 pass, 0 fail, 0 skip, 0 out of scope |
| TCK, unoptimized emission | `feel-tck … --no-opt` → `reports/tck-unoptimized.json` | 3,511 / 3,511 |
| Decision tables as separately compiled unary-test modules vs lowered tables | `feel-tck … --host-dt` | 83 compared, 0 mismatches |
| Semantic vectors (spec examples incl. every Table 78 example, numeric boundaries, missing vs null, temporal, unary tests, closures, diagnostics, limits) | `cargo test -p feel-host --test vectors` and `node web/tests/e2e.mjs` | 412 / 412 natively (optimized + unoptimized, results *and* diagnostic codes must agree) and in Chromium |
| Decimal arithmetic vs Python `decimal` (prec 34, ROUND_HALF_EVEN) | `cargo test -p feel-decimal` | 21,738 vectors, 0 mismatches |
| Temporal library | `cargo test -p feel-temporal` | 69 tests, including 232 temporal `+`/`−` cases transcribed from TCK 0100 |
| Checker, modules, isolation, limits, decision tables, dependencies | `cargo test --workspace` | all pass |

Every TCK test case not commented out in the TCK files is run; the commented-out cases are not part of the suite. The `non-compliant` folder, which the TCK marks as not normative, gives 39 / 50 (`reports/tck-non-compliant.json`). Its failures need non-standard coercions, such as strings used as booleans or numbers.

The per-clause and per-builtin mapping is in [CONFORMANCE-MATRIX.md](CONFORMANCE-MATRIX.md), generated from the reports.

### Not claimed (separate from the FEEL claim)

* **DMN platform conformance:** DMN XML schema validation, DMN DI (diagrams), authority requirements, knowledge sources, and DRG well-formedness checking. The DMN reader is pragmatic: it reads what FEEL evaluation needs.
* **10.3.3 XML data mapping** of FEEL values (XE/XV) is not implemented. FEEL-JSON is the interchange format.
* **PMML** externally defined functions: the mapping is parsed and the binding key is `pmml:<document>#<model>`, but no PMML engine is bound. A call yields null with `FEEL-E360` unless the host binds it. **Java** externals are resolved through host bindings keyed by class and signature; the test bindings cover the `java.lang` methods used by TCK 0076. Classpath configuration is implementation-defined (10.3.2.13.3).

## Dialects and extensions

* **`feel-1.5`** is the only dialect and the conformance baseline (`CompileOptions::dialect`, `metadata.dialect`).
* **`b-feel`** (`CompileOptions.dialect = "b-feel"`, CLI `--dialect b-feel`): the Business FEEL dialect of DMN 1.6 clause 11, a **separate dialect outside the DMN 1.5 claim**. It uses the same grammar and the same generated code, with two-valued boolean logic, plus a runtime mode flag (`DIALECT_BFEEL`, recorded in `metadata.extensionFlags`):
  * comparisons, `in`, `instance of`, `and`, `or`, `not` and conditions treat a null or incomparable result as **false**;
  * `+` with a string converts the other operand to a string (null → `""`); otherwise a number operand turns the other into **0**, and a temporal or duration operand turns the other into a zero duration; `string - string` is `""`; division by zero is 0;
  * statistical builtins (`sum`, `mean`, `stddev`, `median`, `mode`, `product`; `min`/`max` ignore nulls) ignore null and non-number items;
  * a builtin that FEEL makes null returns its result type's default: 0, `""`, false, `[]`, `1970-01-01`, `00:00:00Z`, `1970-01-01T00:00:00Z`, `PT0S`/`P0M`, `(0..0)`.

  These rules were implemented from secondary sources: B. Silver, "Make Way for B-FEEL" (Trisotech, 2024), and the feel.dmn.tools B-FEEL table. All 35 of their examples are vectors (`bfeel-*`), and two controls check that the same expressions keep FEEL 1.5 semantics without the flag. The normative DMN 1.6 clause 11 text is **not yet pinned**. Where the sources are silent or conflict (`**`, `null = null`, filters, `"5" * 2`), the implementation follows the general rules above; check those cases against the spec text before relying on them.
* **Extensions** are opt-in via `extensions` and recorded in the module metadata (`extensionFlags`). Each is enforced twice: by name resolution and by the runtime.
  * `has key(context, key)` → `true` / `false`: key **presence**, independent of the value, so a present null-valued entry is reported as present. Without the extension the name is not a builtin. The standard presence idiom is `count(get entries(m)[key = k]) > 0` (`get entries` keeps present null-valued entries). Both are tested independently (`ctx-has-key-*`, `ctx-presence-idiom`).

## Semantic decisions where the text is ambiguous or the TCK is more specific

Each of these is covered by a TCK case or a vector.

1. **Numbers.** Decimal128 semantics:
   * 34 digits, half-even;
   * division keeps the preferred scale when the quotient is exact, as in Java `BigDecimal.divide(MathContext.DECIMAL128)`;
   * results needing a scale above 6176 are null (underflow), after exact trailing-zero stripping;
   * results needing a scale below −6111 are null (overflow), after zero padding;
   * division by zero is null;
   * `sqrt`, `exp`, `log`, `log10` and non-integer `**` are correctly rounded to 34 digits, never computed in `f64`;
   * `**` exponents outside ±999,999,999 are null (Table 60).
2. **Integers for positions, lengths and scales.** `substring`, `sublist`, `list replace`, `insert before`, `remove`, and the `scale` of `decimal`/`round …`/`floor`/`ceiling` truncate a fractional argument, as the TCK expects (e.g. `decimal(1/3, 2.5) = 0.33`). Rounding to a scale beyond 34 digits leaves the value unchanged (`round up(5.5, 6176) = 5.5`).
3. **Wildcard `-`.** It matches every input value, null included (8.1; the 10.3.2.10 bullet "e != null" is not followed). With input values given, it tests `e in (v)`.
4. **`instance of` precedence** is at comparison level, as in the reference implementations. The order of rule 2 differs only for ill-typed inputs.
5. **String escapes.** An escape not in rule 64 (e.g. `"\d"`) is kept verbatim, backslash included, as regex patterns in the TCK require.
6. **Temporal values:**
   * an implicit date → date and time conversion gives UTC midnight;
   * `=` and `<` on times and date-times compare at **millisecond** precision (TCK 0068), while `is()` and subtraction are exact;
   * when one operand has no zone, it takes the other's offset;
   * a named zone on a time without a date uses the zone's standard offset;
   * DST gaps and overlaps follow `java.time`;
   * month arithmetic clamps the day (`2017-01-31 + P1M = 2017-02-28`);
   * fractional seconds keep nanosecond precision;
   * mixed duration forms (`P1Y2D`) are null;
   * `years and months duration` counts whole months between local dates;
   * year range is ±999,999,999.
7. **`in` with a list** tests membership (`list contains`). Incomparable elements give false, not null (TCK 0072). Range elements test containment.
8. **Ranges.** An interval value needs ordered endpoints of the same type, otherwise it is null (`[2..1]` is null). `= e` and `!= e` are range values distinct from `[e..e]` (DMN 1.5 rule 7.a; TCK 0068, 0074). `range()` accepts only rule 66 literals.
9. **Boxed iterators and conditionals.** "MUST resolve to a boolean / collection" (10.2.1.8–10) yields null otherwise. Textual FEEL `if` treats a non-boolean condition as false (Table 49).
10. **Decision services.** Parameters are the input data followed by the input decisions (10.4). Results are converted to the return type of the service's function typeRef.
11. **Imports.** Imported input data are bound once per namespace (DMN16-50). Unnamed imports merge their elements into the importing scope.
12. **Evaluation order.** `and`, `or`, `some`, `every` and `in (…)` short-circuit. This is observable only in which diagnostics are reported.
13. **`?` in `in`.** In `e1 in e2`, `?` is bound to `e1` (Table 55, last row). In unary tests it is the supplied candidate.

## Known gaps

None affect the TCK or the vectors. They are recorded here so the claim stays exact.

* **XPath regular expressions** run on `fancy-regex`:
  * `\p{IsBlock}` supports a table of common Unicode blocks (BasicLatin, Latin-1Supplement, LatinExtended-A/B, IPAExtensions, Greek, Cyrillic, Hebrew, Arabic, GeneralPunctuation, CJKUnifiedIdeographs); other block names make the pattern invalid (null);
  * `\i`/`\c` are ASCII approximations of the XML name-character classes;
  * character-class subtraction is mapped to intersection with a negated class.
* **Time-zone rules after 2099** are extrapolated (chrono-tz has no transitions after 2099).
* **Checker precision:** a few builtins have approximate static result types (`Any`). This affects only inferred types and diagnostics, never values.
