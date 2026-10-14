#!/usr/bin/env bash
# The gate. PROTECTED. The loop script runs this itself after every iteration; the agent runs it too while working.
#
#   harness/core/gate.sh fast    every iteration (target: a few minutes)
#   harness/core/gate.sh long    every 10 merges: big fuzz, full fixture re-check, khepri sweep, wasm, mutants
#
# Exit 0 = green. Any red step makes the whole gate red; every step still runs so the log shows everything.
set -uo pipefail
MODE="${1:-fast}"
cd "$(dirname "$0")/../.."
ROOT="$PWD"
export ICEPOOL_ORACLE="${ICEPOOL_ORACLE:-$HOME/code/alexrecarey/icepool-oracle}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
H="uv run --quiet python -m harness.core.tool"
FAILED=()

step() {  # step NAME CMD...
  local name="$1"; shift
  echo "=== $name"
  local t0=$SECONDS
  if "$@"; then
    echo "--- $name: ok ($((SECONDS - t0))s)"
  else
    echo "--- $name: FAILED ($((SECONDS - t0))s)"
    FAILED+=("$name")
  fi
}

oracle_pinned() {
  local want have
  want="$(cut -d' ' -f1 oracle.lock)"
  have="$(git -C "$ICEPOOL_ORACLE" rev-parse HEAD)"
  [ "$want" = "$have" ] || { echo "oracle at $have, oracle.lock says $want"; return 1; }
  [ -z "$(git -C "$ICEPOOL_ORACLE" status --porcelain)" ] || { echo "oracle checkout has local changes"; return 1; }
}

grammar_complete() { $H ops > /dev/null || { echo "a node is missing from harness/grammar/status.json"; return 1; }; }

step "oracle pinned and clean" oracle_pinned
step "grammar status complete" grammar_complete
step "cargo build" cargo build --release --workspace --quiet
step "cargo test" cargo test --release --workspace --quiet
step "cargo clippy" cargo clippy --release --workspace --quiet -- -D warnings

if [ "$MODE" = fast ]; then
  step "fixtures (port) + 5% oracle re-check" $H fixtures verify --oracle-sample 0.05
  step "fuzz: fixed seeds" $H fuzz --seed 1 --count 2000
  step "fuzz: fresh seed" $H fuzz --fresh --count 1000
  step "khepri sweep: fixed" $H fuzz --ops khepri --seed 7 --count 150
  step "khepri sweep: fresh" $H fuzz --ops khepri --fresh --count 50
else
  step "fixtures (port) + full oracle re-check" $H fixtures verify --oracle-sample 1.0
  for s in 11 12 13 14 15; do step "fuzz: seed $s" $H fuzz --seed "$s" --count 10000 --depth 4; done
  step "fuzz: fresh seed" $H fuzz --fresh --count 20000 --depth 4
  step "khepri sweep: 5000" $H fuzz --ops khepri --seed 70 --count 5000
fi

# Checks that switch on as the port grows: each runs only once the thing it checks exists.
if [ -f crates/icepool-py/Cargo.toml ] && [ -x harness/core/upstream_suite.sh ]; then
  step "upstream test suite through the facade" harness/core/upstream_suite.sh "$MODE"
fi
if [ "$MODE" = long ] && [ -f crates/icepool-wasm/Cargo.toml ]; then
  step "wasm build" cargo build --release -p icepool-wasm --target wasm32-unknown-unknown --quiet
  # crates/icepool-wasm/run-wasm.sh speaks the icepool-run protocol (stdin/stdout JSON lines) through the WASM build
  # under node. Once it exists, every fixture and a fuzz run go through WASM too.
  if [ -x crates/icepool-wasm/run-wasm.sh ]; then
    step "fixtures through wasm" env ICEPOOL_RUN="$ROOT/crates/icepool-wasm/run-wasm.sh" $H fixtures verify --oracle-sample 0
    step "fuzz through wasm" env ICEPOOL_RUN="$ROOT/crates/icepool-wasm/run-wasm.sh" $H fuzz --seed 21 --count 3000
    step "khepri sweep through wasm" env ICEPOOL_RUN="$ROOT/crates/icepool-wasm/run-wasm.sh" $H fuzz --ops khepri --seed 71 --count 500
  fi
fi
if [ "$MODE" = long ] && command -v cargo-mutants > /dev/null && [ -n "${MUTANTS_BASE:-}" ]; then
  # Mutants in code changed since MUTANTS_BASE. Survivors are reported, not failed: the loop turns them into tasks.
  echo "=== mutants (since $MUTANTS_BASE)"
  mkdir -p harness-out
  git diff "$MUTANTS_BASE" -- crates > harness-out/mutants.diff
  cargo mutants --in-diff harness-out/mutants.diff -p icepool-core --timeout 300 --output harness-out \
    > harness-out/mutants.log 2>&1 || true
  tail -5 harness-out/mutants.log
fi

echo
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "GATE $MODE: GREEN"
  exit 0
fi
echo "GATE $MODE: RED (${FAILED[*]})"
exit 1
