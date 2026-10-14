# Porting icepool to Rust/WASM: analysis

2026-10-13. Analysed against **upstream icepool v2.2.2** (the version khepri-f2f ships), not the local fork at
`~/code/alexrecarey/khepritools/icepool`, which is a stale v1.5.0a0 from June 2024.

## TL;DR

**It will work, and you do not have to change the Python API.** The dynamism in icepool lives almost entirely in its
*Python surface*: `==` returning a die-with-a-truth-value, `__getattr__` proxies, signature-sniffing `star`, Fraction
returns, `Die(die)` returning itself, operator-overloaded `Again`. None of it touches the algorithms that do the work:
the outcome-by-outcome dynamic programme over pools, binomial popping of identical dice, LCM mixing, the absorbing
Markov-chain solver. Those are plain, static, Rust-friendly code.

So the design that gives both "no API changes" and "simple Rust" is **two layers**:

1. **A static Rust core**: generic over an `Outcome` trait and a host-callback trait. No truth-value magic, no
   reflection. Compiles to WASM *and* to a native Python extension.
2. **Thin host facades**: a Python facade over PyO3 that keeps every dunder trick in Python (so upstream's test suite
   runs unchanged against the Rust core), and a JS facade over wasm-bindgen with an idiomatic method-based API. There
   is no existing JS API to preserve, so "API change" doesn't apply there.

The facade can't be paper-thin: ~530 of the test suite's 659 assert lines go through `DieWithTruth` (`==` on dice).
That behaviour must stay in the facade, never in Rust.

## What the code is

| | |
|---|---|
| Source | 14,689 lines, 61 modules, **zero runtime dependencies** (stdlib only; `inspect` used once) |
| Tests | 47 files, 1,712 passing (+8 skipped), **83% line / 75% branch coverage** |
| Maintainer | essentially one person (2,801 of 2,821 commits), still active, internals reworked often (e.g. issue "MultisetExpression rework, again") |
| Public surface | 62 top-level exports; Die: 38 methods + 42 dunders; Population: 50 methods |

The four subsystems and how each ports:

| Subsystem | Lines (approx.) | What it is | Port difficulty |
|---|---|---|---|
| Populations (`Die`, `Deck`, `Counts`) | ~4k | sorted outcome→quantity maps, arithmetic as Cartesian product, LCM mixing | easy–medium |
| `map` / `explode` / `reroll` / `Again` / Markov chains | ~1.5k | callables over outcomes, sentinels, closed-form rerolls, exact Gaussian elimination | medium |
| Multiset machinery (`Pool`, keep tuples, `MultisetEvaluator`, expressions, deals) | ~6k | the paper's DP: pop one outcome at a time from every source, memoised by "room" | medium (core) to hard (`@multiset_function`, tuple expressions) |
| Collections (`Vector`, `Symbols`) and formatting | ~1.5k | outcome types with custom ordering; md/csv/bbcode/html tables | easy |

## Measurements (khepri-f2f's real workload)

| Case | CPython 3.12 native | Pyodide 0.26.3 in browser |
|---|---|---|
| typical, 3 vs 1 | 4 ms | 12 ms |
| mid, 4 vs 2 DA | 10 ms | 29 ms |
| heavy, 6+2 vs 6+2 EXP + Continuous | 140 ms | **380 ms** |
| Pyodide boot + `micropip install icepool` | n/a | **~1.4 s, 5.6 MB transferred** (fast Mac, possibly warm cache; cold mobile will be several seconds) |

Workload shape for the multiset DP (FaceToFace): 210–1,817 memoised rooms, 3k–330k `next_state` calls, max 131
states per room.

**Numbers do not fit in machine integers.** Probabilities are exact fractions:

| Case | Bits |
|---|---|
| Heavy case, roll alone (20¹⁶) | 70 bits |
| Heavy case, final result denominator | **258 bits** |

u64 holds 64 bits and u128 holds 128, so quantities **must be big integers**. Use a crate with inline small-value
storage (`ibig`/`dashu`, or `num-bigint` behind a small-int fast path) so the common case stays fast.

**What a WASM port would surely win:** startup and download (a Rust core should be well under 1 MB of WASM against
5.6 MB plus an interpreter boot), and memory (no Python heap, no global caches growing forever in the worker).
**What it would probably win:** per-calculation speed. Room bookkeeping, hashing and popping move to native code. But
big integers in WASM are software arithmetic, so I won't promise a multiplier until a prototype is measured.

## The design decisions to make now

These are the real trade-offs, the places where the facade cannot fully absorb Python's dynamism.

### 1. Outcome type: one dynamic `Value` enum, generic core

The core is generic (`Die<O: Outcome>`), so Rust users get typed `Die<i64>` with fast dense convolution. Both bindings
instantiate it with a single `Value` enum:

```rust
enum Value { Int(i64), BigInt(..), Float(TotalF64), Fraction(..), Str(Rc<str>),
             Tuple(Rc<[Value]>), Vector(..), Symbols(..), Inf(Sign), Host(HostRef) }
```

Python's semantics go *inside* `Value`, not dropped, because real code depends on them:

- `bool` is an int: `True * 2 == 2`, `True == 1` hash-merge. khepri relies on this: `(d20 > save) * woundsPerFailure`
  and `crit_saves @ (d20 > save)`.
- Tuple `+` concatenates while Vector `+` is elementwise; `zero()` is `x * 0` (gives `''` for str, `()` for tuple).
- Mixed int/float/Fraction compare and sort together.
- `Host(HostRef)` is the escape hatch for namedtuples and user classes in Python: comparisons and hashing call back
  into Python. Slow, but correct and rare.

### 2. User-defined evaluators (`next_state`) need a callback trait

- **Python:** `next_state` is called through PyO3 with the GIL held. That is correct, and still a net win because
  all the room/memo/pop machinery around it moves to Rust.
- **JS:** JS has no value hashing, so **evaluator state must be serialisable to a hashable key** (numbers, strings,
  arrays of those). This is the one genuine API constraint, and it applies to JS only. Batch the callback per room
  (all states, the outcome and the count vector in, a typed array out) so crossings are rooms × branches, not one
  per state.
- Ship `FaceToFace`, `sum`, `size`, keep-index and the other built-in evaluators **prebuilt in Rust**. khepri then
  never crosses the boundary in its hot loop.

### 3. `@multiset_function` and `Again`: medium, not hard, under two layers

- `@multiset_function` *traces*: it calls the user's function once with placeholder expressions and records the
  graph. Tracing stays in the facade (Python, or JS with method calls instead of operators). Rust only executes the
  resulting expression DAG.
- `Again` becomes a closed Rust enum over a fixed op set (`Again | Const | Unary(op) | Binary(op)`), with depth mode
  and count mode ported as written.

### 4. `map(repeat=…)` and Markov chains

These discover states dynamically, so they need incremental callbacks. That is what the callback trait is for. Exact
integer elimination is easy once big integers are in place.

### 5. Tests that pin heuristics, not results

`order_test.py`, `pool_direction_test.py` and `evaluator_cache_test.py` check the auto-order choice and the cache
keying. **Recommendation:** match the order heuristic exactly (it's a small, well-defined function and matching it
keeps results identical). Use **per-call caches** instead of upstream's global, ever-growing ones, and shim
`evaluator_cache_test.py` (it reads `evaluator._cache` directly). Write these semantics down before porting, so the
pinned tests don't look like false failures.

### 6. JS API shape

JS has no operator overloading, so arithmetic is methods: `d(6).add(d(6))`, `die.gt(10)`, `Die.sum(n, die)` for `@`.
`map(fn, ...extra, {star})` takes a JS callable: map makes only 12–42 callback calls per khepri calculation, so the
boundary cost doesn't matter there. Quantities are `BigInt`; probabilities are `{num, den}` or a float.

## Test suite: how far it carries an automated port

| Test kind | Share of cases | Ports how |
|---|---|---|
| Brute-force oracle (pool result vs a hand-written function `map`ped over plain dice) | ~51% | excellent: record the oracle table once, as a golden fixture |
| Hardcoded literal tables | ~29% | excellent: direct fixtures |
| Two icepool APIs compared to each other | ~15% | only meaningful once both APIs exist; runs free under PyO3 |
| Smoke-only (formatting must not raise), exceptions, internals | ~5% | formatting is unconstrained (no output is checked) |

**About 85% of cases are declarative on the side under test** (the lambdas are on the oracle side), so they convert
to JSON fixtures that also run in WASM CI. About 10% need host callbacks: evaluator subclasses, `map` with lambdas,
`@multiset_function` bodies.

### Gaps worth closing *before* porting (and sending upstream)

- `operator/keep.py` at **35%**: keeping on non-generator expressions (`(a|b)[1:]`, mixed-sign slices, `...`) is
  barely tested. This is the most likely place for a port to diverge silently.
- **Never executed at all:**
  - Population: `skewness`, `excess_kurtosis`, `correlation`, `quantity_where`, `split`, `quantile`
  - Die: `ceil`, `floor`, `round`, `trunc`, `middle`
  - Multiset expressions: `isdisjoint`, `issubset`, `modulo_counts`, `count_subset`
  - Tuple-expression slicing, operations on mixed pools, most of Symbols' set operations, `3 - Vector(...)`
- **Docstring examples:** 31 code examples, none run. There are no doctests.
- **Notebooks:** 39 notebooks with 137 stored-output cells of real game workloads (Cortex, BitD, L5R, Neon City,
  Risk…), never re-executed in CI. With nbval/nbmake they become free regression fixtures.

### Upstream bugs found along the way (send as PRs now, don't port them)

- `multiset_function.py:363-390` `prepare_multiset_joint_function`: inner loop dedented outside the outer `for`, so
  only the last preparation survives.
- `operator/binary.py:109` `MultisetAdditiveUnion._initial_state` unpacks exactly two sizes, though it accepts any
  number of children.
- `expression/multiset_tuple_expression.py:118` `MultisetTupleIndex._initial_state` returns `None` instead of
  `(None, None)`.
- `collection/symbols.py:125` `Symbols.__getitem__` on a `defaultdict` inserts missing keys.
- `order.py:118-131` `pool_pop_order` contradicts `PoolSource.order_preference` and is dead code.
- `collection/vector.py`: `cartesian_product` / `tupleize` / `vectorize` duplicate `expand.py` and are dead.

## Harness for automating the port

1. **PyO3 strangler: the automation engine.** Build the Rust core as a Python extension. Replace one module at a time
   (pool popping → evaluator driver → Die arithmetic → …) and rerun upstream's full 1,712 tests after each swap.
   About 40 of 47 test files should run unchanged; the rest need small shims for private internals. An agent loop
   ("make test file X pass against the Rust core") has a hard, objective oracle at every step.
2. **Golden JSON fixtures** harvested from the Python run (a flag-gated `conftest.py` that records both sides of
   every equality) form the WASM build's own test suite. Store quantities as strings and outcomes typed.
3. **Differential fuzzing.** Generate a grid of expressions (die sizes, pool shapes, keep tuples with negatives and
   `...`, decks/deals, mixed dice), run them through Python icepool and the port, and compare tables exactly. Then
   add a few hypothesis properties, e.g. `highest(n,k) + lowest(n,n-k) == n @ die`, and ascending/descending forced
   orders must agree.

## About "contributing it back"

Be clear-eyed about this. icepool's README sells "pure Python, no dependencies, runs in Pyodide", and the author's
apps and JupyterLite site depend on that. A compiled core *replacing* pure Python is unlikely to be accepted. Shapes
that could be:

- an **optional accelerated backend** (`pip install icepool[fast]`, same API, falls back to pure Python), or
- a **sister project** (`icepool-rs` plus an npm package) that upstream links to and shares test fixtures with.

Either way, **open a GitHub discussion with the maintainer before writing Rust.** The test-hardening work below is
welcome upstream regardless, and it's a good way to start that conversation.

## Options

| | What | Effort | Upstream-able |
|---|---|---|---|
| **A** | Two-layer full-parity port: Rust core + PyO3 facade + JS facade | months of focused work; heavily automatable thanks to the strangler | yes, as backend or sister project |
| **B** | khepri-only Rust engine: Die/map/explode/`@` + Pool/keep/DP + FaceToFace | ~1–1.5k lines of Rust, a couple of weeks | no |
| **C** | Stay on Pyodide; cheap fixes (reuse the evaluator between calls; preload/cache the Pyodide bundle) | days | n/a |

**Recommendation: A, phased so that B falls out as phase 1.** If the maintainer isn't interested, you've lost nothing:
phase 1 ships khepri on WASM either way.

### Phases

0. **Talk to the maintainer** (GitHub discussion). Pin the port to v2.2.2 and track upstream releases.
1. **Test hardening, sent upstream** (~1.5 weeks): close the coverage gaps above, run the notebooks in CI, turn the
   docstring examples into tests, send the bug PRs.
2. **Core that is enough for khepri** (confident estimate, ~1–1.5k lines): `Counts`/`Die` with big integers, arithmetic,
   `map` + mixture (`weighted_lcm` ported exactly), `reroll`/`explode`/`@`, Pool with keep tuples and skip collapse,
   backward/forward DP with compact index-window room keys, prebuilt evaluators. **Ship khepri on it**, verified
   against `test_f2f.py` outcome by outcome.
3. **PyO3 facade + strangler.** Swap modules in behind the unchanged Python API until the upstream suite is green.
4. **Remaining surface for full parity:** multiset expression operators, `Deck`/`Deal`/`MultiDeal`, tuple
   expressions, `@multiset_function`, full `Again`, `map(repeat)`/Markov, statistics, `Vector`/`Symbols`,
   formatting.
5. **JS facade + npm package**, with the golden fixtures as its test suite.
