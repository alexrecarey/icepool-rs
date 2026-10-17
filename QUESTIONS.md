# Questions for Alex

Genuine questions about intended behaviour that the oracle cannot answer. The loop works around them and carries on.

## V1: LGPL-derived libm ports in an MIT crate (resolved 2026-10-17)

Resolved by B9: Alex accepts Rust's built-in float math, so the glibc ports (`value/trig.rs`, `trig_tables.rs`,
`libm.rs`, `pow.rs`) are deleted and no LGPL-derived code is left. Native builds still call the platform libm;
WASM results are within 2 ULP of the oracle's (DIVERGENCES.md).

## S6c: put the JS suite in the long gate (harness/core/gate.sh is the loop's)

The JS facade's suite, `crates/icepool-wasm/js/build.sh --test`, runs every fixture through the JS API (the
idiomatic methods, JS callbacks and evaluators, JS Maps as dicts), the API tests and a `tsc` type check. It is GREEN
at S6c (all fixtures, 0 failing; ~40 s with a warm cargo cache) but no gate runs it, so a core change can break the
JS facade unnoticed. Please add it to the long gate, next to "fixtures through wasm" (it needs `wasm-pack` and
`node` >= 22; `tsc` is optional, the script skips it when missing):

```bash
  if command -v wasm-pack > /dev/null && command -v node > /dev/null; then
    step "JS suite" crates/icepool-wasm/js/build.sh --test
  fi
```
- S6c question: done 2026-10-16, JS suite added to the long gate (node 22 in ~/.local/node22).

## B9: compare.py's ULP tolerance should cover complex values (done 2026-10-17: complex repr leaves compared part by part)

With the glibc ports gone, 7 complex powers in `fixtures/v1/complex-edge-cases.jsonl` (lines 2, 5, 13, 15, 63, 76,
79) come out of WASM 1-2 ULP off in a part, e.g. `(0.6236155388561803-0.6236155388561802j)` for Python's
`...803j)`. A complex has no wire type: results encode it as `{"repr": "complex: (a+bj)"}`, which `compare.py`
compares as text even with `ICEPOOL_FLOAT_ULPS=2`, so `gate.sh long`'s "fixtures through wasm" step fails on them
(`ICEPOOL_FLOAT_ULPS=2 ICEPOOL_RUN=crates/icepool-wasm/run-wasm.sh harness/core/tool fixtures verify --oracle-sample 0`:
11555 checked, these 7 failures). Its "fuzz through wasm" step will too: `fuzz --ops pow,truediv,entropy,
standardized_moment,die,d,weights,die_typed --seed 21 --count 2000` through WASM gives 5 failures, all complex
results of `standardized_moment` with a fractional k, each 1 ULP off in one part (no plain float fails).
compare.py is protected, so: please extend `_eq` to compare two complex repr
leaves part by part, e.g.

```python
_PART = r"(?:inf|nan|[\d.]+(?:e[+-]\d+)?)"
_COMPLEX = re.compile(rf"^complex: (?:\((-?{_PART})([+-]{_PART})j\)|(-?{_PART})j)$")

def _complex_parts(r):
    m = isinstance(r, str) and _COMPLEX.match(r)
    return m and ((m[1], m[2]) if m[3] is None else ("0", m[3]))

# in _eq, next to the {"float"} case:
        if set(a) == {"repr"} and set(b) == {"repr"} and a != b:
            pa, pb = _complex_parts(a["repr"]), _complex_parts(b["repr"])
            return bool(pa and pb) and all(_float_close(x, y) for x, y in zip(pa, pb))
```

The JS fixture suite does exactly this already (`crates/icepool-wasm/js/test/fixtures.mjs` `complexParts`) and passes.

## D1: two protected-path moves the library design needs (done 2026-10-17: design doc in docs/design/, fixtures.rs in crates/icepool-harness/tests/, guard and mutants updated, C5b requeued)

1. **The design document.** D1 asks for `docs/design/library-architecture.md`, but `docs/` is protected, so it is
   at `docs/design/library-architecture.md` (as FACADE-AUDIT.md went to the root). Please `git mv` it to
   `docs/design/library-architecture.md`; tasks.json's C details already say "to move to docs/design/".
2. **The fixtures test** (needed by C5b, not before). The JSON program runner leaves icepool-core (design section 5),
   but the protected `crates/icepool-core/tests/fixtures.rs` calls `icepool_core::program::run`. Until it moves, C5
   keeps the runner in core behind a non-default `runner` feature that the new `icepool-harness` crate turns on, so
   `cargo test --workspace` still compiles and runs it. Please, once C5 is merged: move it to
   `crates/icepool-harness/tests/fixtures.rs` with one change, `icepool_core::program::run` ->
   `icepool_harness::run` (C5 makes that function exist), keep it in guard.sh's protected list under its new path,
   and update its `root()` (`../..` stays right: the crate is also two levels down). C5b then moves `program/`.

   **C5 status (2026-10-17): ready to move.** `crates/icepool-harness` exists with `pub use ... run` and a
   build.rs identical to icepool-core's (the `env!("CARGO_MANIFEST_DIR")` rebuild guard), so the move is exactly:

   ```bash
   git mv crates/icepool-core/tests/fixtures.rs crates/icepool-harness/tests/fixtures.rs
   sed -i 's/icepool_core::program::run/icepool_harness::run/' crates/icepool-harness/tests/fixtures.rs
   # guard.sh: protect crates/icepool-harness/tests/fixtures.rs instead of the old path
   ```

   (`serde_json` is already a normal dependency of icepool-harness; no Cargo.toml change is needed.) One knock-on
   in the protected gate.sh: its long-gate mutants step runs `cargo mutants ... -p icepool-core`, which tests only
   icepool-core's package; once the fixtures test lives in icepool-harness, mutants in core would no longer be
   checked against the fixtures. Please add `--test-package icepool-core,icepool-harness` (or
   `--test-workspace true`) to that line when you move the file. Until then core's dev-dependency on
   icepool-harness keeps `cargo test -p icepool-core` running every fixture.

   **C5b status (2026-10-17): waiting on this move.** C5b was scheduled twice with fixtures.rs still in
   icepool-core, so both attempts could only record that they are blocked. Please hold C5b until the move is done.

## L202610172222: the long gate's "wasm build" step no longer warms run-wasm.sh's build (gate.sh is the loop's)

Since C5 (2026-10-17 20:34) icepool-core's `khepri` engine is behind a feature. The long gate's "wasm build" step,
`cargo build --release -p icepool-wasm --target wasm32-unknown-unknown`, builds icepool-core for wasm32 with
`runner` only (icepool-wasm's default), but run-wasm.sh builds `--features runner,khepri`, which compiles
icepool-core again: ~25 s here (`touch crates/icepool-core/src/lib.rs; cargo build --lib --release --target
wasm32-unknown-unknown -p icepool-wasm --features runner,khepri`). The harness starts `$ICEPOOL_RUN` lazily on the
first program and gives it 20 s, so whenever the node package is stale (after any merge that touches core) the
first fixture of "fixtures through wasm" times out (long-20261017-2209: `s1b/die-methods-edge-cases.jsonl:1`, a
plain `d6.zero()`; it passes once built). Earlier long gates only passed because some worktree had happened to
build the package after main's files were written. Making `runner`/`khepri` default would put test code in a plain
`wasm-pack build`, so the fix belongs in the gate. Please change the step to build what run-wasm.sh builds:

```bash
  step "wasm build" cargo build --release -p icepool-wasm --target wasm32-unknown-unknown --features runner,khepri --quiet
```

(check-production.sh already builds the npm package's feature set just before, so nothing else loses its warm
cache.) With that, run-wasm.sh's own build is wasm-bindgen only (~1 s), as its header comment says.

## L202610172222: complex `**` under WASM can be more than 2 ULP off (seed 21 of "fuzz through wasm")

`complex(-0.00024988505290350285, -0.5) ** complex(-5.0, 2.5)` is `(1605.4982660299447-258.3902050878144j)` in
Python and natively, `(1605.4982660299454-258.3902050878131j)` through WASM: 3 ULP off in the real part, 23 in the
imaginary. Not a port bug: CPython's `_Py_c_pow` computes `phase = atan2(b, a) * br + bi * log(hypot(a, b))` and
returns `len * cos(phase)`, `len * sin(phase)`. Rust's libm (what WASM gets) is 1 ULP off glibc in `hypot`, `pow`,
`atan2`, `exp` and `log` for these inputs, and a 1-ULP change in `phase` (6.12) moves `len * sin(phase)` by about
`len * cos(phase) * ulp(6.12)` = 1.4e-12 = 23 ULP of 258. No order of operations avoids it: only bit-exact
transcendentals would, and B9 decided against porting libm again. The long gate's "fuzz through wasm" (seed 21,
3000 programs) generates exactly this program, so that step stays red until one of:

1. a looser tolerance for complex parts (e.g. relative error ~1e-14 of the complex's magnitude rather than ULPs of
   each part) in compare.py and the JS suite's `complexParts`;
2. accepting it as a known failure of the long gate (no fixture added: natively it already matches);
3. bringing back exact libm ports for wasm32 only.

The divergence is recorded in DIVERGENCES.md ("Floats under WASM").
