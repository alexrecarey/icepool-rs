#!/usr/bin/env bash
# Check 1: upstream's own test suite, run against the Python facade (python/icepool) with ICEPOOL_BACKEND=rust.
# PROTECTED. Switched on by gate.sh once crates/icepool-py exists.
#
# The facade is built with maturin into the harness venv. Tests come from the oracle checkout, unmodified, except
# that shims for tests reading private internals may live in harness/upstream_shims/ (conftest.py there is loaded).
#
# Every test id in baselines/upstream-pass.txt must pass. New passes are written to harness-out/upstream-pass.txt;
# the loop script merges them into the baseline on main, so the pass list only ever grows.
set -uo pipefail
cd "$(dirname "$0")/../.."
export ICEPOOL_ORACLE="${ICEPOOL_ORACLE:-$HOME/code/alexrecarey/icepool-oracle}"
mkdir -p harness-out

uv run --quiet maturin develop --release -m crates/icepool-py/Cargo.toml > harness-out/maturin.log 2>&1 \
  || { tail -30 harness-out/maturin.log; exit 1; }

SHIMS=()
[ -f harness/upstream_shims/conftest.py ] && SHIMS=(-p harness.upstream_shims.conftest)
ICEPOOL_BACKEND=rust PYTHONPATH="$PWD/python:$PWD" uv run --quiet python -m pytest "$ICEPOOL_ORACLE/tests" \
  -q -p no:cacheprovider -W ignore --rootdir "$ICEPOOL_ORACLE" "${SHIMS[@]}" \
  --junitxml=harness-out/upstream.xml > harness-out/upstream.log 2>&1
tail -3 harness-out/upstream.log

python3 - <<'PY'
import os, sys, xml.etree.ElementTree as ET
root = ET.parse("harness-out/upstream.xml").getroot()
passed = set()
for tc in root.iter("testcase"):
    if not any(c.tag in ("failure", "error", "skipped") for c in tc):
        passed.add(f"{tc.get('classname')}::{tc.get('name')}")
with open("harness-out/upstream-pass.txt", "w") as f:
    f.write("".join(sorted(t + "\n" for t in passed)))
base = set()
if os.path.exists("baselines/upstream-pass.txt"):
    base = {l.strip() for l in open("baselines/upstream-pass.txt") if l.strip()}
lost = sorted(base - passed)
print(f"upstream suite (rust backend): {len(passed)} passing, baseline {len(base)}, newly passing {len(passed - base)}")
for t in lost[:20]:
    print("  NO LONGER PASSES:", t)
sys.exit(1 if lost else 0)
PY
