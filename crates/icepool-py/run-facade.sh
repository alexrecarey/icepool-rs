#!/usr/bin/env bash
# The icepool-run protocol through the Python facade on the Rust backend (see run-facade.py), for ICEPOOL_RUN:
#
#   ICEPOOL_RUN=crates/icepool-py/run-facade.sh uv run python -m harness.core.tool fuzz --count 2000
#
# Rebuilds the extension into the harness venv (maturin develop) first if any Rust source is newer than the
# installed one, before reading stdin, so the harness's per-program timeout never counts a build.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"
SO="$(ls .venv/lib/python3*/site-packages/icepool_rs/icepool_rs*.so 2>/dev/null | head -1 || true)"
if [ -z "$SO" ] || [ -n "$(find crates/icepool-core/src crates/icepool-py/src Cargo.lock -newer "$SO" -print -quit)" ]; then
  uv run --quiet maturin develop --release -m crates/icepool-py/Cargo.toml > /dev/null 2>&1 \
    || { echo "maturin develop failed" >&2; exit 1; }
fi
exec uv run --quiet python "$HERE/run-facade.py"
