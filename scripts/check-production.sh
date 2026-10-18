#!/usr/bin/env bash
# Production builds contain no test code (design/library-architecture.md section 8). Builds what ships, the Python
# wheel (scripts/build-wheel.sh: the icepool facade and the icepool_rs extension) and the npm package (npm pack of
# crates/icepool-wasm/js), lists their contents and symbols, and fails if any test, fixture, runner, harness or
# JSON-program code is in them, or if the wheel is not a usable package:
#
#   1. dependency graph: icepool-py and icepool-wasm (as built alone, which is how maturin and wasm-pack build them)
#      depend on neither icepool-harness, serde_json nor icepool-khepri (khepri's engine);
#   2. wheel contents: no test, fixture, harness, catalogue, run-facade or program files; both packages, `icepool`
#      and `icepool_rs`, present; installed into a fresh venv, `import icepool` (from there, never the checkout)
#      runs on the Rust backend with ICEPOOL_BACKEND unset;
#   3. the extension module holds no harness, runner (panic locations embed source paths, so any linked runner code
#      shows: `icepool-harness`, and `src/program/`, its old home in icepool-core) or serde_json strings, and no
#      runner, harness or serde_json symbols;
#   4. npm contents: no tests, fixtures, catalogue, run-wasm or bench files;
#   5. the .wasm modules hold none of those strings either, and export neither `run` (test-only) nor `calculate`
#      (khepri's), nor the JSON-op exports `call`, `evaluate`, `from_wire`, `wire_result`.
#
# INTERIM, until task C6l: the TypeScript API's bridge (crates/icepool-wasm/src/facade.rs, under js/src/rpc.ts) still
# calls the program runner's nodes by name for what is not on typed exports yet, so the npm package links the runner
# (icepool-harness, without its `khepri` feature) and serde_json and exports the JSON-op calls. While facade.rs uses
# `icepool_harness`, those (and only those) are reported, not failed, for the npm package alone; once C6l removes the
# bridge, every check above applies to the npm package too, with no edit here.
#
# The npm package is checked as `js/build.sh --dev` builds it: the same wasm-bindgen output as `--pack`, without
# wasm-opt, which only shrinks the module (and takes up to 45 s). Needs cargo, uv (maturin), wasm-pack, node, npm,
# unzip, tar, nm: a missing tool is a failure, never a silent pass. Usage: scripts/check-production.sh
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
# The node the long gate uses for the JS suite, if installed there.
[ -x "$HOME/.local/node22/bin/node" ] && PATH="$HOME/.local/node22/bin:$PATH"

for tool in cargo uv wasm-pack node npm unzip tar nm; do
  command -v "$tool" > /dev/null || { echo "check-production: $tool not found, cannot check"; exit 1; }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILED=0
fail() { echo "  FAIL: $*"; FAILED=1; }

# Test-only paths in a package: tests, fixtures, harness, catalogue functions, the program runner, the scripts
# that drive the facades with programs, benchmarks.
TEST_PATHS='(^|/)(tests?|conftest|fixtures?|harness|catalogue|program|run[-_](facade|wasm)|bench)([/._-]|$)|(^|/)test_|\.test\.'
# Strings any linked harness or runner code leaves in a binary.
HARNESS_STRINGS=(icepool-harness icepool_harness)
RUNNER_STRINGS=(src/program/ serde_json)

INTERIM=0
if grep -q 'icepool_harness' crates/icepool-wasm/src/facade.rs 2> /dev/null; then
  INTERIM=1
  echo "INTERIM (until C6l): the JS bridge calls the program runner; runner code in the npm package is reported, not failed"
fi

# check_paths WHAT < listing: fails on test-only paths.
check_paths() {
  local bad
  bad="$(grep -E "$TEST_PATHS" || true)"
  [ -z "$bad" ] || fail "$1 contains test-only files:"$'\n'"$bad"
}

# check_strings FILE INTERIM_OK STRING...: fails on any STRING found in FILE (only reports it if INTERIM_OK=1).
check_strings() {
  local file="$1" interim_ok="$2" s n
  shift 2
  for s in "$@"; do
    n="$(grep -c -a -F -- "$s" "$file" || true)"
    [ "$n" = 0 ] && continue
    if [ "$interim_ok" = 1 ]; then
      echo "  interim: $(basename "$file") holds \"$s\" ($n lines)"
    else
      fail "$(basename "$file") holds \"$s\" ($n lines)"
    fi
  done
}

echo "=== 1. dependency graph"
# check_tree PACKAGE INTERIM_OK [cargo tree args...]
check_tree() {
  local pkg="$1" interim_ok="$2" tree
  shift 2
  tree="$(cargo tree -p "$pkg" -e normal,features --prefix none "$@" 2>&1)" || { fail "cargo tree -p $pkg: $tree"; return; }
  echo "  $pkg: $(grep -c ' v[0-9]' <<< "$tree") normal dependency lines"
  grep -q 'icepool-khepri v' <<< "$tree" && fail "$pkg has icepool-khepri (khepri's engine)"
  local pattern
  for pattern in 'serde_json v' 'icepool-harness v'; do
    if grep -q "$pattern" <<< "$tree"; then
      if [ "$interim_ok" = 1 ]; then echo "  interim: $pkg has $pattern"; else fail "$pkg has $pattern"; fi
    fi
  done
}
check_tree icepool-py 0
check_tree icepool-wasm "$INTERIM" --target wasm32-unknown-unknown

echo "=== 2. Python wheel (scripts/build-wheel.sh)"
if scripts/build-wheel.sh "$WORK/wheel" > "$WORK/maturin.log" 2>&1; then
  WHEEL="$(ls "$WORK"/wheel/*.whl)"
  unzip -Z1 "$WHEEL" > "$WORK/wheel.txt"
  echo "  $(basename "$WHEEL"): $(wc -c < "$WHEEL") bytes, $(wc -l < "$WORK/wheel.txt") files:"
  sed 's/^/    /' "$WORK/wheel.txt"
  check_paths "the wheel" < "$WORK/wheel.txt"
  for f in icepool/__init__.py icepool/_wheel.py icepool_rs/__init__.py; do
    grep -qx "$f" "$WORK/wheel.txt" || fail "the wheel has no $f"
  done

  # The smoke test: the wheel alone, in a fresh venv on the harness's Python (the wheel is built for it), run from
  # $WORK with -I (no cwd, PYTHONPATH or user site on sys.path) and ICEPOOL_BACKEND unset. Installing needs no
  # network: the wheel has no dependencies.
  PY="$(uv run --quiet python -c 'import sys; print(sys.executable)')"
  if uv venv --quiet --python "$PY" "$WORK/venv" > "$WORK/venv.log" 2>&1 \
    && VIRTUAL_ENV="$WORK/venv" uv pip install --quiet --offline "$WHEEL" >> "$WORK/venv.log" 2>&1; then
    if (cd "$WORK" && env -u ICEPOOL_BACKEND "$WORK/venv/bin/python" -I -c '
import sys, icepool
assert icepool.__file__.startswith(sys.prefix), icepool.__file__
assert icepool.backend.BACKEND == "rust" and icepool.backend.RUST is not None, icepool.backend.BACKEND
die = icepool.d(6) + 1
assert die.outcomes() == (2, 3, 4, 5, 6, 7) and die.quantities() == (1,) * 6, die
assert type(die._rs).__module__ == "icepool_rs", die
print("  smoke test: `icepool.d(6) + 1` from the installed wheel, on the Rust backend by default")
' 2>&1); then :; else fail "the installed wheel's smoke test failed"; fi
  else
    cat "$WORK/venv.log"
    fail "could not install the wheel into a fresh venv"
  fi

  echo "=== 3. extension module"
  unzip -q "$WHEEL" -d "$WORK/wheel-x"
  for so in "$WORK"/wheel-x/*/*.so; do
    echo "  $(basename "$so"): $(wc -c < "$so") bytes; exported: $(nm -D --defined-only "$so" | awk '{print $3}' | tr '\n' ' ')"
    check_strings "$so" 0 "${HARNESS_STRINGS[@]}" "${RUNNER_STRINGS[@]}"
    n="$(nm -C "$so" 2> /dev/null | grep -c -E 'icepool_harness::|serde_json::' || true)"
    [ "$n" = 0 ] || fail "$(basename "$so") has $n runner, harness or serde_json symbols"
  done
else
  tail -30 "$WORK/maturin.log"
  fail "maturin build failed"
fi

echo "=== 4. npm package (js/build.sh --dev, npm pack)"
if crates/icepool-wasm/js/build.sh --dev > "$WORK/wasm-pack.log" 2>&1 \
  && (cd crates/icepool-wasm/js && npm pack --silent --pack-destination "$WORK" > "$WORK/npm-pack.log" 2>&1); then
  TGZ="$(ls "$WORK"/*.tgz)"
  tar tzf "$TGZ" > "$WORK/npm.txt"
  echo "  $(basename "$TGZ"): $(wc -c < "$TGZ") bytes, $(wc -l < "$WORK/npm.txt") files:"
  sed 's/^/    /' "$WORK/npm.txt"
  check_paths "the npm package" < "$WORK/npm.txt"

  echo "=== 5. wasm modules"
  mkdir "$WORK/npm-x" && tar xzf "$TGZ" -C "$WORK/npm-x"
  for wasm in "$WORK"/npm-x/package/pkg-*/*.wasm; do
    exports="$(node -e '
      const m = new WebAssembly.Module(require("fs").readFileSync(process.argv[1]));
      console.log(WebAssembly.Module.exports(m).filter((e) => e.kind === "function").map((e) => e.name).join(" "));
    ' "$wasm")"
    echo "  $(basename "$(dirname "$wasm")")/$(basename "$wasm"): $(wc -c < "$wasm") bytes; exports: $exports"
    check_strings "$wasm" "$INTERIM" "${HARNESS_STRINGS[@]}" "${RUNNER_STRINGS[@]}"
    for e in $exports; do
      case "$e" in
        run | calculate) fail "$(basename "$wasm") exports $e (a build feature the package leaves out)" ;;
        call | evaluate | from_wire | wire_result)
          if [ "$INTERIM" = 1 ]; then echo "  interim: exports $e"; else fail "$(basename "$wasm") exports $e"; fi ;;
      esac
    done
  done
else
  tail -30 "$WORK/wasm-pack.log" "$WORK/npm-pack.log" 2> /dev/null
  fail "the npm package build failed"
fi

echo
if [ "$FAILED" = 0 ]; then
  if [ "$INTERIM" = 1 ]; then
    echo "check-production: OK, no test code in the wheel; the npm package as INTERIM allows (until C6l)"
  else
    echo "check-production: OK, no test code in the wheel or the npm package"
  fi
  exit 0
fi
echo "check-production: FAILED"
exit 1
