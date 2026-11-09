#!/usr/bin/env bash
# Build the browser artifacts:
#   web/dist/compiler.wasm   the FEEL compiler compiled to Wasm (crate feel-web)
#   web/dist/runtime.wasm    the shared FEEL runtime (crate feel-runtime-wasm)
#   web/dist/src/*.js        TypeScript bindings (ES modules, via tsc)
#   web/dist/demo/demo.js    the demo page script
#   web/dist/worker.bundle.js  self-contained worker script (esbuild)
#   web/dist/vendor/feelin.js  feelin 7.0.2 as one ES module (benchmarks only)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build --profile wasm --target wasm32-unknown-unknown -p feel-web -p feel-runtime-wasm

mkdir -p web/dist
cp target/wasm32-unknown-unknown/wasm/feel_web.wasm web/dist/compiler.wasm
cp target/wasm32-unknown-unknown/wasm/feel_runtime_wasm.wasm web/dist/runtime.wasm

cd web
if [ ! -d node_modules ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then
  npm ci --no-audit --no-fund
fi
npx tsc -p tsconfig.json
node build-bundles.mjs
ls -l dist/compiler.wasm dist/runtime.wasm dist/worker.bundle.js dist/vendor/feelin.js
