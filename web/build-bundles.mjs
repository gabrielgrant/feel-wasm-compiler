// Bundles produced after `tsc` (see scripts/build-web.sh):
// - dist/worker.bundle.js: the worker entry with its imports inlined, so
//   FeelWorker can start it from a blob: URL (restarts work offline);
// - dist/vendor/feelin.js: feelin (benchmark comparison only) as a single
//   ES module that the benchmark page can import without a CDN.
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const common = { bundle: true, format: 'esm', target: 'es2022', logLevel: 'warning', legalComments: 'none' };

await build({ ...common, entryPoints: [path.join(here, 'dist/src/worker.js')], outfile: path.join(here, 'dist/worker.bundle.js'), sourcemap: true });
await build({ ...common, entryPoints: [path.join(here, 'bench/feelin-entry.mjs')], outfile: path.join(here, 'dist/vendor/feelin.js'), minify: true });
