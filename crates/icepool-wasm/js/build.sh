#!/usr/bin/env bash
# Builds the npm package (crates/icepool-wasm/js): wasm-pack's nodejs and web targets into js/pkg-node and
# js/pkg-web, then the TypeScript API (js/src/*.ts) into js/dist with tsc, then optionally runs the JS tests or packs
# the package.
#
#   js/build.sh            optimised build (wasm-opt -O3; slow under load), what `npm pack` should ship
#   js/build.sh --dev      release code without wasm-opt (under a second once cargo has built the wasm32 target)
#   js/build.sh --test     the --dev build with the `runner` and `khepri` features (the `run` and `calculate` exports,
#                          which the package leaves out), then the JS tests (js/test/: the fixture suite, the API
#                          tests, the types)
#   js/build.sh --pack     the optimised build, then `npm pack` in js/
#
# tsc (TypeScript >= 5.2) comes from js/node_modules (`npm install`), else from PATH or the usual global installs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/.." && pwd)"
ROOT="$(cd "$CRATE/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

tsc_path() {
  local c
  for c in "$HERE/node_modules/.bin/tsc" "$(command -v tsc || true)" "$HOME/.bun/bin/tsc" \
    "$(npm root -g 2> /dev/null || true)/typescript/bin/tsc"; do
    if [ -n "$c" ] && [ -x "$c" ]; then
      echo "$c"
      return
    fi
  done
  echo "js/build.sh: tsc not found: run 'npm install' in crates/icepool-wasm/js (or install typescript)" >&2
  return 1
}
TSC="$(tsc_path)"

OPT=()
FEATURES=()
case "${1:-}" in --dev | --test) OPT=(--no-opt) ;; esac
case "${1:-}" in --test) FEATURES=(-- --features runner,khepri) ;; esac
for target in nodejs web; do
  dir="$HERE/pkg-${target%js}"
  wasm-pack --quiet build "$CRATE" --release --target "$target" --out-dir "$dir" --no-pack "${OPT[@]}" "${FEATURES[@]}"
  rm -f "$dir/.gitignore"
done
# The nodejs target is CommonJS, inside a "type": "module" package.
echo '{"type": "commonjs"}' > "$HERE/pkg-node/package.json"
# The API, compiled against this build's generated types (pkg-node/icepool_wasm.d.ts).
rm -rf "$HERE/dist"
"$TSC" -p "$HERE/tsconfig.json"
case "${1:-}" in
  --test)
    cd "$HERE"
    node --test "test/*.test.mjs"
    "$TSC" -p test/tsconfig.json
    ;;
  --pack) cd "$HERE" && exec npm pack ;;
esac
