#!/usr/bin/env bash
# Builds the Python wheel that ships (design/library-architecture.md section 8): the `icepool` facade
# (python/icepool) and the `icepool_rs` extension (crates/icepool-py), with ICEPOOL_BACKEND defaulting to `rust`.
#
# maturin packs only the Python sources inside its project directory (the one holding pyproject.toml) and follows
# no symlinks out of it, so `python-source = "../../python"` in crates/icepool-py/pyproject.toml would build the
# extension alone. Instead this stages a project, in a temporary directory, from crates/icepool-py/wheel/ (its
# pyproject.toml, `icepool_rs/__init__.py`, and `icepool/_wheel.py`, the build-time default backend) plus a copy of
# python/icepool, and runs `maturin build --release` there. crates/icepool-py/pyproject.toml is left as it was: the
# harness's `maturin develop` still installs the extension alone, and chooses the backend explicitly.
#
# Usage: scripts/build-wheel.sh OUT_DIR   (the wheel is written to OUT_DIR; extra arguments go to maturin build)
set -euo pipefail
[ $# -ge 1 ] || { echo "usage: $0 OUT_DIR [maturin build args...]" >&2; exit 2; }
OUT="$(mkdir -p "$1" && cd "$1" && pwd)"
shift
cd "$(dirname "$0")/.."
ROOT="$PWD"
TEMPLATE="$ROOT/crates/icepool-py/wheel"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir "$STAGE/python"
sed "s|@MANIFEST@|$ROOT/crates/icepool-py/Cargo.toml|" "$TEMPLATE/pyproject.toml" > "$STAGE/pyproject.toml"
cp python/README.md "$STAGE/README.md"
cp -r "$TEMPLATE/icepool_rs" python/icepool "$STAGE/python/"
cp "$TEMPLATE/icepool/_wheel.py" "$STAGE/python/icepool/"

# maturin from the harness venv (uv run), building in the staged project. Run from outside the checkout, cargo
# would not reuse what `maturin develop` and the gate built, for two reasons, so both are passed on:
# - rustup finds rust-toolchain.toml from the working directory: its channel goes in RUSTUP_TOOLCHAIN, so the
#   wheel is built by the pinned toolchain, not the machine's default;
# - icepool-core's build script ties its fingerprint to $PWD, the checkout cargo runs for (see its build.rs): that
#   is still $ROOT, not the staging directory, which is new on every run.
TOOLCHAIN="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
(cd "$STAGE" && PWD="$ROOT" RUSTUP_TOOLCHAIN="$TOOLCHAIN" \
  uv run --project "$ROOT" --quiet maturin build --release --out "$OUT" "$@")
