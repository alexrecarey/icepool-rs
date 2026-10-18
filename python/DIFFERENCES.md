# Differences from upstream icepool on the Rust backend

`python/icepool` is upstream icepool with its modules switched to the Rust core (`icepool_rs`) under
`ICEPOOL_BACKEND=rust`. Results are meant to be upstream's exactly, down to the order of outcomes, the types and the
integer quantities, and the port copies upstream's bugs rather than fixing them (DIVERGENCES.md at the repository
root lists those). This page lists what still differs, and why. Written by hand (task C8, 2026-10-18) from
DIVERGENCES.md, dev/NOTES.md and [FACADE-AUDIT.md](../docs/FACADE-AUDIT.md), each claim checked against the code and the oracle; keep it current
when a difference goes or a new one is found.

## Which icepool this is

The facade is a copy of the oracle pinned in `oracle.lock` (commit 70168f56): upstream **v2.2.2** plus upstream's
next commit (`any()` evaluation, the opposite of `empty()`) plus three bug fixes on their way upstream:

- a joint `@multiset_function` dropping all but the last outer preparation (evaluator/multiset_function.py);
- `additive_union` with more than two arguments (operator/binary.py);
- `Symbols` lookups adding missing symbols (collection/symbols.py).

Both backends have these, so both differ from the released v2.2.2 there. `icepool.__version__` still says `2.2.2`.

## The backend switch

`ICEPOOL_BACKEND` is read once, when `icepool` is imported (`python/icepool/backend.py`):

- `python`, **the default**: upstream's pure-Python code everywhere, exactly as upstream ships it.
- `rust`: each module keeps its Python API and argument handling, and its computational work runs in the core.
  Python callables (map functions, user evaluators, user sources and operators, `outcome_type`, ...) are called back.
- anything else: `ImportError`.

There is no silent fallback: with `rust`, if `icepool_rs` cannot be imported, `import icepool` fails, and what the
core cannot take raises `NotImplementedError` (below). `icepool.backend.RUST` is the extension module, or `None`.

The default, with `ICEPOOL_BACKEND` unset, is `rust` in the wheel (scripts/build-wheel.sh, which ships the facade
with the extension) and `python` in a checkout (see python/README.md).

## Caches live for one call

The core caches within one computation and never across calls, so results never depend on what the process
computed before. Upstream keeps several caches across calls, and under `rust` these are not used or not filled:
the global `PoolSource` cache, `Die._sum_cache` (behind `n @ die`), and the evaluators' room caches (a dungeon's
`ascending_cache` / `descending_cache` stay empty; the `_cache` of dungeons itself is kept). `icepool.d` and
`icepool.z` keep upstream's `@cache`, so `d(6) is d(6)` holds on both backends.

What that changes:

- **Repeated identical calls are not faster the second time.** Upstream answers a warm repeat from its caches;
  the core recomputes it.
- **No history-dependent answers.** Upstream's global `PoolSource` cache is keyed by `==`, so after a pool popped
  down to `(1,)`, a pool whose outcomes are `(True,)` gets the cached `(1,)` (DIVERGENCES.md K5). The Rust backend
  always gives what a fresh process gives.
- **`n @ die` near Python's recursion limit.** Upstream's `Die._sum_all` recurses once per roll not yet in the die's
  `_sum_cache`, so whether `n @ die` raises `RecursionError` depends on which sums that die (and `icepool.d`'s
  cached dice) computed before: after `500 @ d6`, upstream answers `1000 @ d6`. The Rust backend has no such
  history: for steps from 800 up to about 1000 rolls it raises `NotImplementedError("icepool_rs: n @ die close to
  Python's recursion limit")`, beyond that `RecursionError` (DIVERGENCES.md K4). Below 800 rolls both answer.
- **The recursion limit of evaluations** is calibrated to upstream's for a fresh evaluator and cold dice (an
  evaluation recurses once per outcome: `RecursionError` near 980 outcomes at top level). Upstream's real limit
  also depends on its warm caches, which the facade does not reproduce.

## `NotImplementedError("icepool_rs: ...")`

Where upstream answers and the core does not model the case, the Rust backend raises `NotImplementedError` with a
message starting `icepool_rs:`. Known cases:

- `n @ die` with a step between 800 and about 1000 rolls (above);
- counts that are not ints: a keep count or keep index of another type (`d6.pool(3).keep([1.0, 1, 1])`),
  `multiply_counts` by a non-int, operator constants (`unique(1.5)`, `keep_counts('>', 0.5)`), a `map_counts`
  function returning a non-int (a `bool` too: upstream keeps bool counts), `MultisetKeep` indices other than a slice,
  list or tuple of ints and `...`, slice bounds that are not ints;
- counts or sizes from a user's `MultisetSource` or `MultisetOperator` that are not ints, or tuples of ints, within
  64 bits;
- an evaluation prepared into dungeon classes the core does not know (a user's own `_prepare`, which is internal
  API);
- a `MultiDeal` hand size that is not an int, or is negative (upstream raises `TypeError` or `ValueError` later);
- a deck operation whose other operand has counts that are not ints (`Deck([1, 2]) + {1: 1.5}`; upstream raises
  `ValueError` in `Deck(...)`).

A user operator also gets no source or argument counts from the core (an empty iterator and an empty tuple): an
operator should only read its children's counts.

## Other small differences

- **Same NaN object twice.** Python's dict merges the very same NaN object (identity), so upstream's `x =
  float('nan'); Die([x, x])` has one outcome; the core never merges NaNs and gives two.
- **Exception types.** `truncate` and `clip` hash their bounds on entry, so an unhashable bound raises `TypeError`
  even on an empty die (upstream returns the empty die). Weights with `__index__` that are not ints (numpy ints)
  raise `TypeError` where upstream raises `ValueError` later.
- **`int` subclasses in `sum`'s float path** go the generic way; the result is the same unless a compensation
  was pending.
- **Identity.** Dice the core makes are new facade objects: `pool._dice` holds dice equal to, not identical to,
  the dice passed in. Private attributes (`_data`, `_rs`, the caches above) differ; code that reads upstream's
  internals may see something else. (The few upstream tests that read internals run with shims,
  `harness/upstream_shims/`.)

## Deep values and recursion limits

The core recurses natively and must not overflow its stack, so every recursion on a nested value counts a level
(`crates/icepool-core/src/value/recursion.rs`) and raises `RecursionError` past 10000 levels (CPython's C
recursion limit) or 6 MiB of stack. So anything on a tuple nested more than 10000 deep raises, where CPython may
still answer (it hashes nested tuples without a limit), and near 10000 the exact level can differ from CPython's by
a few. `Vector` operators count the Python frames upstream's methods would use, against `sys.getrecursionlimit()`.
With the recursion limit raised far, `Vector` operators and evaluations over very many outcomes raise from the
stack budget where Python continues (or crashes). See DIVERGENCES.md, "Values nested ~10000 deep".

`==` and `<` between deeply nested `Vector`s take exponential time in Python (from about 30 levels upstream
effectively hangs); the core computes the same results, and raises where Python eventually would, at once.

## Ctrl-C and threads

The core checks for signals at most every 50 ms in its long loops (evaluations, sums, `@`, Markov chains, Cartesian
products), so Ctrl-C raises `KeyboardInterrupt` as it does in pure Python; dropping a big evaluation's memo after
the interrupt can take a moment (half a second for `d(30).pool(60).expand()`).

The core releases the GIL during built-in evaluations, `n @ die`, and the built-in operators on dice without
Python-object outcomes, so other Python threads keep running, in parallel, while it computes. Paths driven by
Python callbacks (`map`, user evaluators, `map(repeat=...)`) hold the GIL between callbacks. Using icepool from
several threads at once is safe.

## Speed and memory

Measured at 1b5764c (docs/reviews/2026-10-17-library-review.md), cold: 1.2 to 10 times faster than upstream
(median about 2.5). Slower where most of the time is Python-side argument handling: building a big die from Python
values (`Die(range(10000))`, 0.6x), `map` with a Python callback (0.8x), and tiny calls such as `quantity()`
(0.4x). Warm repeats lose to upstream's caches (above). A die takes about twice upstream's memory per outcome,
and big evaluations peak at 3 to 4 times upstream's.

## Floats

On the Rust backend floats are bit-exact: the core calls the same platform math library CPython does. (Only the
WASM build, the npm package, differs in the last bits of some results; DIVERGENCES.md, "Floats under WASM".)
