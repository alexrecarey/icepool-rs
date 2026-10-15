# Questions for Alex

Genuine questions about intended behaviour that the oracle cannot answer. The loop works around them and carries on.

## V1: LGPL-derived libm ports in an MIT crate

To match the oracle bit for bit under WASM, complex powers need glibc's own `hypot`, `sin`, `cos`, `atan2`
(wasm32's libm rounds differently in the last bit). `crates/icepool-core/src/value/trig.rs` (+ `trig_tables.rs`)
and `glibc_hypot` in `value/libm.rs` are line-by-line ports of glibc 2.43 (IBM Accurate Mathematical Library and
glibc's own hypot), which is LGPL-2.1-or-later, while the workspace is `license = "MIT"`. (`pow.rs`, `libm.rs`'s
`log` and `exp` come from Arm's optimized-routines, MIT, so they are fine.) The files say where they come from.
Is LGPL-derived code acceptable here (e.g. relicense those files, or the WASM package, as LGPL), or should they be
replaced? The alternative is an independent correctly-rounded implementation, which would match glibc in nearly
all cases but not provably all. Until you decide, the ports stay in: natively they are unused (the platform glibc
is called), only the wasm32 build runs them.

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
