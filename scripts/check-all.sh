#!/usr/bin/env bash
# Everything CI runs, in order.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
cargo build --profile wasm --target wasm32-unknown-unknown -p feel-web -p feel-runtime-wasm
scripts/run-tck.sh
scripts/build-web.sh
node web/tests/e2e.mjs
