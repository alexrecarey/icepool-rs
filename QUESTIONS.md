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

## B9: compare.py's ULP tolerance should cover complex values (the long gate's WASM step needs it)

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

## D1: two protected-path moves the library design needs (loop owner)

1. **The design document.** D1 asks for `docs/design/library-architecture.md`, but `docs/` is protected, so it is
   at `design/library-architecture.md` (as FACADE-AUDIT.md went to the root). Please `git mv` it to
   `docs/design/library-architecture.md`; tasks.json's C details already say "to move to docs/design/".
2. **The fixtures test** (needed by C5b, not before). The JSON program runner leaves icepool-core (design section 5),
   but the protected `crates/icepool-core/tests/fixtures.rs` calls `icepool_core::program::run`. Until it moves, C5
   keeps the runner in core behind a non-default `runner` feature that the new `icepool-harness` crate turns on, so
   `cargo test --workspace` still compiles and runs it. Please, once C5 is merged: move it to
   `crates/icepool-harness/tests/fixtures.rs` with one change, `icepool_core::program::run` ->
   `icepool_harness::run` (C5 makes that function exist), keep it in guard.sh's protected list under its new path,
   and update its `root()` (`../..` stays right: the crate is also two levels down). C5b then moves `program/`.
