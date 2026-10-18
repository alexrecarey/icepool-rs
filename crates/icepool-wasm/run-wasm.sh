#!/usr/bin/env bash
# The icepool-run protocol (one JSON program per stdin line, one JSON result per stdout line) through the WASM
# build under node. The harness uses it as ICEPOOL_RUN (see harness/core/gate.sh long).
#
#   run-wasm.sh           the protocol, for ICEPOOL_RUN
#   run-wasm.sh --test    calculate() against f2f.calculate's recorded outputs (test-calculate.mjs)
#   run-wasm.sh --bench   khepri timings (bench.mjs)
#
# Builds the node package with wasm-pack first if any source is newer than it, before reading stdin, so that the
# harness's 20 s per-program timeout never counts a build. The protocol uses wasm-bindgen's output as it is
# (--no-opt: under a second once cargo has built the wasm32 target, which the long gate does just before), since
# wasm-opt -O3 alone took 6 to 45 s here. It is the same code rustc generated for wasm32, which is what can differ
# from native; wasm-opt preserves semantics. --test and --bench use the optimised build that ships.
# A lock keeps parallel starts from building at once. Built with the `runner` (the `run` export) and `khepri` (the
# `calculate` export) features, which the npm package leaves out.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

case "${1:-}" in
  --test | --bench) PKG="$CARGO_TARGET_DIR/icepool-wasm/node"; OPT=() ;;
  *) PKG="$CARGO_TARGET_DIR/icepool-wasm/node-noopt"; OPT=(--no-opt) ;;
esac
STAMP="$PKG/icepool_wasm_bg.wasm"

stale() {
  [ -f "$STAMP" ] || return 0
  # The runner (the `run` export) is icepool-harness's since C5b: its sources count too.
  [ -n "$(find "$ROOT/crates/icepool-core/src" "$ROOT/crates/icepool-harness/src" "$HERE/src" "$ROOT/Cargo.lock" \
    "$HERE/Cargo.toml" "$ROOT/crates/icepool-core/Cargo.toml" "$ROOT/crates/icepool-harness/Cargo.toml" \
    -newer "$STAMP" -print -quit)" ]
}

if stale; then
  mkdir -p "$CARGO_TARGET_DIR"
  (
    flock 9
    if stale; then
      wasm-pack --quiet build "$HERE" --release --target nodejs --out-dir "$PKG" --no-pack "${OPT[@]}" \
        -- --features runner,khepri >&2
      touch "$STAMP"
    fi
  ) 9> "$CARGO_TARGET_DIR/icepool-wasm.lock"
fi

case "${1:-}" in
  --test) exec node "$HERE/test-calculate.mjs" "$PKG" ;;
  --bench) exec node "$HERE/bench.mjs" "$PKG" ;;
  *) exec node "$HERE/run-wasm.mjs" "$PKG" ;;
esac
