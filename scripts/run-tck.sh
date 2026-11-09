#!/usr/bin/env bash
# Run the pinned DMN TCK (compliance levels 2 and 3) against the generated Wasm
# backend, optimized and unoptimized, plus the decision-table differential.
set -euo pipefail
cd "$(dirname "$0")/.."
TCK="${TCK_DIR:-.tck}"
[ -d "$TCK/TestCases" ] || scripts/fetch-tck.sh "$TCK"
cargo build --release -p feel-dmn
mkdir -p reports
./target/release/feel-tck "$TCK/TestCases" --json reports/tck.json --md reports/tck-by-directory.md "$@"
./target/release/feel-tck "$TCK/TestCases" --no-opt --json reports/tck-unoptimized.json
./target/release/feel-tck "$TCK/TestCases" --host-dt
# informational: the TCK's non-compliant folder (not part of the conformance claim)
./target/release/feel-tck "$TCK/TestCases" --non-compliant --json reports/tck-non-compliant.json || true
