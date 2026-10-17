# Library review, 2026-10-17

Three read-only reviews of icepool-rs at `1b5764c` (all 55 port tasks done, long gate green): robustness,
performance and memory across general workloads, and code quality as a library. Alex's decisions (2026-10-17): fix
all robustness findings (R1-R4), drop the glibc math ports (B9), do all code-quality work (D1, C1-C8); Python and
JS/TS bindings over the Rust core, the TS API idiomatic and as close to icepool as possible with its own docs;
production builds contain no tests. The performance items (P-list below) are not scheduled yet: don't regress them,
and prefer designs that make them easy.

## Robustness

The Python facade is very hardened: huge sides/pools, negative sizes, NaN/inf, int overflow (add, mul, pow, shift,
i64::MIN / -1, neg, abs), huge counts, keep indices, `n @ die` recursion, exceptions in callbacks, re-entrant
callbacks and inconsistent `__lt__` all match Python down to the exception type. No i64 truncation. JS errors are
proper exceptions, BigInts round-trip, a JS callback's exception is rethrown as itself, serde_json's 128-level limit
rejects deep JSON cleanly. 30k hostile fuzzed programs through icepool-run found no other panics.

1. **Segfault on deep Vectors (Python facade).** `t=1; for i in range(5000): t=Vector([t])`, then `t+t` -> rc=-11.
   At depth 100000 also `Die([t])`, `t==u`, `str(Die([t]))`. Python raises RecursionError. Only `hash(t)` raises.
   Recursion without depth guard: `crates/icepool-py/src/convert.rs:184` (from_py), `:233` (to_py), Value binary
   ops / eq / format in `crates/icepool-core/src/value/{ops,vector,format}.rs`. (Nested plain tuples segfault in
   CPython itself, so only Vector is a regression.)
2. **Abort / panic / trap on huge dice.** Runner `{"op":"d","sides":9223372036854775807}` -> capacity-overflow panic
   at `crates/icepool-core/src/function.rs:22`; `{"op":"d","sides":8589934592}` -> "memory allocation of
   687194767360 bytes failed", rc=134 (uncatchable); `die_sequence` with rolls=2**63-1 does `vec![die; rolls]` at
   `program/collection.rs:64`; JS `d(2n**63n-1n)` -> `RuntimeError: unreachable`. A panic while a RefCell (HANDLES,
   `program/multiset_function.rs:462-464` VARS) is borrowed can poison the instance. The Python facade pre-checks
   these (MemoryError/OverflowError) - move the checks into core; about 79 `with_capacity`/`vec![x; n]` sites to
   audit. No catch_unwind at WASM exports (`icepool-wasm/src/lib.rs:37-46`) or PyO3 entry points.
3. **Ctrl-C ignored, GIL held.** `(d(100).pool(400).highest(200).sum()).mean()`, SIGINT after 3 s: no reaction in
   60 s (oracle: KeyboardInterrupt in 0.2 s). No `check_signals`, no `allow_threads`/`detach` in
   `crates/icepool-py/src`. Threads otherwise safe (8 threads, re-entrant callbacks, no errors).
4. **Possible leak (unproven).** 20k x `d(6).map(lambda o, big=big: o)` + `pool(3).sum()`: maxrss +1.7 MB (rust) vs
   +0.77 MB (oracle). Py<PyAny> held in Rust Arcs is invisible to the cycle GC. JS HANDLES
   (`icepool-wasm/src/facade.rs:65`) freed only by FinalizationRegistry.
5. Minor: JS `new Die([undefined])` gives a NoneType '<' TypeError instead of "cannot pass"; JS `Die.sum(900, d(2))`
   says "not ported yet: n @ die close to Python's recursion limit".

## Performance and memory (28 general workloads)

Cold vs cold: native Rust 2.8-12x faster than pure Python (median ~6x); the Python facade 1.2-10x (median ~2.5x).
Slower than Python: `Die(range(10000))` 0.6x and `map` with a Python callback 0.8x (both: Python-side argument
expansion, `creation_args.expand_args_for_die_rust` isinstance checks against Mapping/Protocols, plus
`contains_again` in `population/again.py:246`), tiny calls such as `quantity()` 0.4x (~0.55 us vs 0.18 us per call).
Warm repeated identical calls: Python wins via its cross-call caches (`Die._sum_cache`, evaluator dungeon cache,
`@cache` on `d()`); icepool-rs caches per call by design. Least gain: big Value-tuple state spaces (all_straights
14d12 1.8x native).

Hotspots: generic Value arithmetic/hashing in sums and evaluators (`Value::Int` is IBig, dashu dispatch);
`binary_operator` builds all pairwise sums then hashes and merges (71% of 100d6; Python-style sort 13%); evaluator
pops allocate per call (`CompoundKeepSource::pop` `Vec<Vec<...>>`, memo key drop 10%, Arc atomics 7-14%); Value-tuple
evaluator states (all_straights: hashing 20%, eq 9%).

Memory: native Die item `(Value, UBig)` 80 B (~180 B with cumulative quantities and index map); through the facade
219 B/outcome vs Python 101 B. Peak during big evaluations 3-4x Python (per-call memo holding cloned sources,
statelets, dice). No unbounded caches; RSS flat over repeated calls. `Value` is 56 B because of inline `RBig`.

Not scheduled yet (P-list), in expected-value order: dense i64 convolution for sums of int dice (100d6 3.2x, 10d6
7x, d100@d10 2.9x); facade fast path for plain outcomes (`Die(range(10000))` 30 -> 12 ms, callback map 2.6x);
bounded LRU cross-call caches; checked-i64 fast path in `Value::binary` (-15 to -21% on sums); evaluator buffer reuse
+ borrowed memo keys (-20-30% on pool evaluators, lower peak memory); typed evaluator states (-30-40% on
straights/poker); box Fraction to shrink Value 56 -> 32 B (~30% less per Die item); pre-sized maps, cached hashes,
skip re-sorting sorted ints (-10-15%). Arc -> Rc is not worth it (Send/Sync needed by host traits and frozen
pyclasses).

## Code quality

Clean in the small: zero clippy warnings, no unsafe, one upstream TODO, tests in `#[cfg(test)]`, good doc comments on
core types, provenance comments cite Python file + function (no line numbers: keep it that way). Not yet a library to
show upstream: it reads as Python icepool re-expressed in Rust.

1. **Python semantics in the core trait.** `crates/icepool-core/src/outcome.rs`: `Outcome` requires
   `py_lt/py_le/py_gt/py_ge/py_eq/check_key` returning Result. `pysort.rs` (721 lines) is a top-level core module;
   ~50 py_* references outside program/. Every `Die<i64>` comparison returns Result.
2. **Features fixed to Value.** All built-in evaluators are `impl MultisetEvaluator<Value>` (evaluator/poker.rs,
   keep.rs, basic.rs, lexi.rs, argsort.rs, comparison.rs; poker uses `outcome.binary(BinOp::Gt, ..)?.truthy()?`).
   `impl Die<Value>` in population.rs:222, die.rs:586, map_tools/core_impl.rs, format.rs, again.rs, to_pool.rs,
   expand.rs. Value in ~30 core files outside program/. `Die<i64>` users get construction, pops, reroll, clip, sums;
   no poker/keep/map/explode/Again/formatting. Only typed evaluators: khepri.rs (application code, 646 lines; move
   out).
3. **No idiomatic Rust API.** No std::ops on Die (`binary_operator(...)`, `matmul(&[(i64, Quantity)])`); `Die::new(
   Vec<DieArg<O>>, Vec<Quantity>)` takes a Python-style argument enum; Display only on Value; all 33 modules pub
   (program, pysort, khepri, value::libm, trig_tables); crate docs point at NOTES.md and the harness.
4. **Errors named after Python exceptions.** `error.rs`: `Error::{Type, Value, Index, Key, ZeroDivision,
   Overflow,..}(String)`, `python_type()`, Display "TypeError: msg"; 116 TypeError/ValueError string sites (86 outside
   program/); `Error::Host(HostError)` in core.
5. **Tangled facades.** The JS "facade" exports `call(op, args) -> String`, `evaluate(id, args, kwargs)`,
   `from_wire`: a JSON-op RPC with Python kwargs, not an idiomatic API; uses program::{eval, call, host, wire, PValue,
   PErr} 19 times. icepool-py imports `program::PErr` and `program::matmul_dice` (lib.rs:27, :397). Host-callback
   traits spread over value/host.rs (HostObject), host_source.rs, operator/mod.rs (HostOperator), program/host.rs
   (HostFunction, HostEvaluator). Good: nothing outside program/ imports crate::program.
6. **Upstream tracking.** Module headers cite their Python source; the mapping is buried in NOTES.md. README is 10
   lines of loop links; repo root full of loop machinery.
7. **Size/cohesion.** program/ops.rs 1556, program/expression.rs 1390, value/ops.rs 909; program/ mirrors the core
   file for file, so each feature touches core + program + py + wasm.
8. **Duplication.** evaluator/poker.rs: seven evaluators with the same boilerplate and their own int/int_tuple/
   sorted_descending helpers. Facade conversions written twice (icepool-py/src/convert.rs 384 lines; JSON/JS tagged
   forms in facade.rs).
9. **Tests.** Fixtures run via cargo test without the oracle (good). No CI. 77 Rust #[test]s, mostly value/* and
   program/; die.rs, pool.rs, population.rs, evaluators, map_tools have none: all behaviour coverage goes through the
   JSON runner.
10. icepool-core declares the `icepool-run` [[bin]], pulling serde_json (arbitrary_precision) into every consumer.
    `Quantity = UBig` is a good choice: keep it.

## Size (khepri-only notes, for reference)

Full wasm 2.66 MB / 936 KB gzip; khepri-only build (no runner) 627 KB / 245 KB gzip. LTO + opt-level "s" -22% at
~25% slower heavy case; opt-level "z" 2x slower; wasm-opt -Oz vs -O3 < 0.5%. Dropping the glibc ports saves only
14-21 KB.
