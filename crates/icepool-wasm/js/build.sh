#!/usr/bin/env bash
# Builds the WASM for the npm package (crates/icepool-wasm/js): wasm-pack's nodejs and web targets into js/pkg-node
# and js/pkg-web, then optionally runs the JS tests or packs the package.
#
#   js/build.sh            optimised build (wasm-opt -O3; slow under load), what `npm pack` should ship
#   js/build.sh --dev      release code without wasm-opt (under a second once cargo has built the wasm32 target)
#   js/build.sh --test     the --dev build with the `runner` and `khepri` features (the `run` and `calculate` exports,
#                          which the package leaves out), then the JS tests (js/test/: the fixture suite, the API
#                          tests, the types)
#   js/build.sh --pack     the optimised build, then `npm pack` in js/
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/.." && pwd)"
ROOT="$(cd "$CRATE/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
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
case "${1:-}" in
  --test)
    cd "$HERE"
    node --test "test/*.test.mjs"
    if command -v tsc > /dev/null; then tsc --noEmit --strict --target es2022 --module nodenext test/types-check.ts; fi
    ;;
  --pack) cd "$HERE" && exec npm pack ;;
esac
