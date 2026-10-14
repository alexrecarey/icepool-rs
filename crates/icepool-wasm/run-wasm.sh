#!/usr/bin/env bash
# The icepool-run protocol (one JSON program per stdin line, one JSON result per stdout line) through the WASM
# build under node. The harness uses it as ICEPOOL_RUN (see harness/core/gate.sh long).
#
# Builds the node package with wasm-pack first if any source is newer than it, before reading stdin, so the
# harness's per-program timeout never counts a build. A lock keeps parallel starts from building at once.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
PKG="$CARGO_TARGET_DIR/icepool-wasm/node"
STAMP="$PKG/icepool_wasm_bg.wasm"

stale() {
  [ -f "$STAMP" ] || return 0
  [ -n "$(find "$ROOT/crates/icepool-core/src" "$HERE/src" "$ROOT/Cargo.lock" "$HERE/Cargo.toml" \
    "$ROOT/crates/icepool-core/Cargo.toml" -newer "$STAMP" -print -quit)" ]
}

if stale; then
  mkdir -p "$CARGO_TARGET_DIR"
  (
    flock 9
    if stale; then
      wasm-pack --quiet build "$HERE" --release --target nodejs --out-dir "$PKG" --no-pack >&2
      touch "$STAMP"
    fi
  ) 9> "$CARGO_TARGET_DIR/icepool-wasm.lock"
fi

exec node "$HERE/run-wasm.mjs" "$PKG"
