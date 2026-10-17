#!/usr/bin/env bash
# wasm-pack builds of the JS facade with khepri's `calculate` (feature `khepri`): for the web (ES module + .wasm,
# what khepri's worker imports) and for node.
#   crates/icepool-wasm/build.sh [OUT]    OUT defaults to $CARGO_TARGET_DIR/icepool-wasm; writes OUT/web and OUT/node
# run-wasm.sh builds OUT/node by itself when it is stale.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="${1:-$CARGO_TARGET_DIR/icepool-wasm}"
for target in web nodejs; do
  dir="$OUT/${target%js}"
  wasm-pack --quiet build "$HERE" --release --target "$target" --out-dir "$dir" -- --features khepri
  echo "$dir: $(wc -c < "$dir/icepool_wasm_bg.wasm") bytes wasm, $(gzip -9c "$dir/icepool_wasm_bg.wasm" | wc -c) gzipped"
done
