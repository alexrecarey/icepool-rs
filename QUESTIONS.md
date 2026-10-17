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
