# Benchmarks

Two sets of measurements, both reproducible:

* **Browser** (`node web/bench/run.mjs`): headless Chromium 153 (Playwright 1.63.0) on an Intel Xeon @ 2.30 GHz with 4 logical CPUs. Compared against `feelin` 7.0.2. Full tables, method and raw data: [web/bench/RESULTS.md](../web/bench/RESULTS.md) and `web/bench/results.json`.
* **Native** (`cargo run --release -p feel-host --example bench`): the emitted module executed by wasmi 2.0.0 with the runtime bound natively, on the same machine.

These numbers come from one machine and one run each. Treat differences below about 10–20 % as noise. No claims are made beyond the workloads listed.

## Browser: phases measured separately

| Phase | Result |
|---|---|
| Cold startup: fetch + compile + instantiate of `compiler.wasm` (2.83 MB) and `runtime.wasm` (2.26 MB) in parallel, fresh context, n = 25 | median **60 ms**, p90 85 ms |
| Then the first compile + instantiate + evaluate | median 11.3 ms |
| parse / check / full compile (parse + check + lower + optimize + emit), per workload, 500 iterations | parse 0.06–0.10 ms, check ≈ 0.06 ms, compile **0.125–0.20 ms** median; modules are 1.3–2.1 KB |
| Instantiation (fresh runtime instance + module instance + constant pool), 200 iterations | 0.61–0.73 ms with a precompiled `WebAssembly.Module`; 0.72–1.16 ms including module compile |
| One-shot compile + instantiate + evaluate | 1.1–1.6 ms median |

## Browser: warm evaluation per call (compiled once), vs feelin

Our `evaluate()` time includes the JS↔Wasm transfer: JS values → FEEL-JSON → runtime → FEEL-JSON → JS values with exact decimals. feelin's public `evaluate`/`unaryTest` re-parses the source on every call; it has no compile-once API.

| Workload (input) | ours median µs | feelin median µs | feelin / ours |
|---|---:|---:|---:|
| `age >= 18 and income > 50000 and status in ("A","B")` | 2.91 | 34.0 | 11.7× |
| `if score >= 700 then … else …` | 2.19 | 31.3 | 14.3× |
| `decimal((principal * rate / 12) / (1 - (1 + rate / 12) ** -term), 2)` | 36.9 | 55.8 | 1.5× |
| `sum(for x in items return x.price * x.qty)` (20 items) | 26.3 | 63.7 | 2.4× |
| `items[price > 10].name` (100 items) | 105 | 79.8 | **0.8×** (we are slower) |
| `for x in xs return x * 2` (100 numbers) | 23.8 | 47.7 | 2.0× |
| `sort(xs, function(a,b) a < b)` (100 numbers) | 126 | 207 | 1.6× |
| unary tests `[18..65)` / `"A","B","C"` / `< 1000` | 2.15–2.27 | 7.1–9.7 | 3.3–4.3× |

The unrounded loan formula is **excluded** from the comparison because the results differ: ours is `1266.713274564701728308559295569298` (decimal128), feelin's is `1266.7132745647145` (binary double).

For the 100-item filter/projection, most of our time is FEEL-JSON transfer of the 100-context input. `evaluateJson` (raw text in and out) takes 64.6 µs.

## Browser: transfer cost (identity `x` on a list of n decimals)

| n | `evaluateJson` total | `evaluate` (JS values) total | input only (`count(x)`) |
|---:|---:|---:|---:|
| 10 | 5 µs | 10 µs | 5 µs |
| 1,000 | 0.115 ms | 0.24 ms | 0.05 ms |
| 100,000 | 14.6 ms | 28.8 ms | 6.2 ms |

At n = 100,000, the runtime's FEEL-JSON decode is about 60 ns per element and its encode about 85 ns per element. The TypeScript decimal-preserving codec roughly doubles the end-to-end cost when JS values rather than text are used.

## Native (wasmi)

`cargo run --release -p feel-host --example bench`: 300 compiles, 200 instantiations, then 200 warmup and 2,000 timed evaluations per workload. Each evaluation includes decoding the FEEL-JSON inputs and encoding the result.

| workload | compile median µs | instantiate median µs | eval median µs | eval p90 µs | eval p99 µs | module bytes |
|---|---:|---:|---:|---:|---:|---:|
| `predicate` | 77.4 | 37.8 | 0.74 | 0.78 | 0.88 | 1858 |
| `loan` (unrounded, exact decimal) | 75.4 | 37.9 | 14.18 | 14.68 | 31.26 | 1929 |
| `sum-decimal` (100 items) | 88.4 | 44.1 | 55.65 | 60.09 | 87.70 | 1901 |
| `filter-project` (100 items) | 81.3 | 46.4 | 50.10 | 54.91 | 76.56 | 2035 |
| `map-double` (100 numbers) | 75.5 | 41.3 | 22.45 | 24.96 | 58.90 | 1674 |
| `sort-lambda` (100 numbers) | 87.6 | 44.0 | 175.16 | 195.78 | 248.72 | 1791 |
| `ut-range` `[18..65)` | 67.4 | 33.1 | 0.29 | 0.31 | 0.35 | 1379 |
| `ut-strings` `"A","B","C"` | 66.0 | 31.5 | 0.34 | 0.36 | 0.40 | 1475 |

The whole DMN TCK runs in about 4.5 s (`feel-tck`). That covers parsing about 150 models, compiling about 2.2 s of modules (1.07 MB of Wasm in total), and evaluating 3,511 result nodes.

## Not measured

* CEL was not benchmarked; no CEL comparison is claimed.
* There is no comparison with other FEEL engines besides feelin.
* There is no measurement on real networks or under compression of the Wasm artifacts (about 5 MB uncompressed).
