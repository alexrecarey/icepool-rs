# Divergences

Python icepool behaviour the port copies but which looks wrong. Each entry: what, where in icepool, a drafted
upstream test. Alex decides what goes upstream.

## The oracle itself

The oracle is pinned (oracle.lock) to alexrecarey/icepool `fix/multiset-bugs`: upstream v2.2.2, plus upstream's
next commit (reinstating `any()` evaluation), plus three fixes Alex is sending upstream:
- joint `@multiset_function` dropping all but the last outer preparation (evaluator/multiset_function.py)
- `additive_union` with more than two arguments (operator/binary.py)
- `Symbols` lookups adding missing symbols (collection/symbols.py)

Those three are bugs, not semantics, so the port follows the fixed behaviour. When upstream releases them, re-pin.

## Found during the port

### A tuple holding `Reroll` raises `AttributeError` in `Die(...)` (K3)

`creation_args.expand_arg` sends a tuple through `cartesian_product(*arg)`, which returns `Reroll` itself if any
element is `Reroll`. `expand_arg` then returns that `Reroll` as if it were a mapping (`if type(arg) == tuple: ...
else: return arg`), and `merge_weights_lcm` raises `AttributeError: 'RerollType' object has no attribute 'values'`.
The docs say "If any argument is `icepool.Reroll`, the result is `icepool.Reroll`", so the intent is surely that the
outcome drops out. The port raises too (`program/map.rs` `tupleize`). It also means `map(lambda x: (x, Reroll) if
... else ..., die)` raises whenever any branch builds such a tuple, whatever its weight.

Drafted upstream test:

```python
def test_tuple_with_reroll_drops_out():
    assert Die([(1, Reroll), 2]).equals(Die([2]))
    assert d6.map(lambda x: (x, Reroll) if x == 6 else (x, x)).equals(d5.map(lambda x: (x, x)))
```


### `n @ die` raises RecursionError for n above ~985 (K4)

`Die._sum_all(rolls)` (population/die.py) computes `self._sum_all(rolls - 1) + self` recursively, so `Die([1000])
@ d6` (or `1000 @ d6`) raises `RecursionError` unless the die's `_sum_cache` already holds a sum within ~980
rolls (it persists on the die object, and `icepool.d` caches its dice, so the outcome depends on history). The
port copies the raise for steps of 1000 or more and reports anything between 800 and 1000 as unimplemented.
A loop over `range(max cached + 1, rolls + 1)` would fix it.

Drafted upstream test:

```python
def test_matmul_large_count():
    result = 2000 @ icepool.Die([1])
    assert result.equals(icepool.Die([2000]))
```

### The global `PoolSource` cache merges outcome spellings across pools (K5)

`PoolSource._new_raw` (generator/pool.py) is a `@cache`d classmethod keyed by `(dice, outcomes, keep_tuple)`,
compared with `==`. `True == 1`, so after any pool popped down to outcomes `(1,)`, popping a pool whose remaining
outcomes are `(True,)` returns the cached source with `(1,)` (and likewise dice spelled `Die([1])` for
`Die([True])`). Results then depend on what the process computed before. The port has no global cache and gives
what a fresh process gives; the `pool_pop` node calls `icepool.Pool.clear_cache()` first so the oracle does too.
The evaluator (K6) will meet the same thing in the outcome types of its results.

Drafted upstream test:

```python
def test_pool_source_cache_keeps_spelling():
    list(Pool([Die([1, 2])])._make_source().pop(Order.Descending, 2))
    for source, _, _ in Pool([Die([True, 2])])._make_source().pop(Order.Descending, 2):
        assert type(source.outcomes()[0]) is bool  # fails: the cached source has (1,)
```


### `middle(keep)` with `keep` above the size keeps only the top roll (K5)

`KeepGenerator.middle` (generator/keep.py) says "If this is greater than the current keep_size, all are kept", but
with the same parity it computes `start = (keep_size - keep) // 2`, which is negative, and `self[start:stop]` then
counts that start from the end: `d6.pool(3).middle(5)` has keep tuple `(0, 0, 1)`, so `d6.middle(3, 5)` is the
highest of 3d6, not 3d6. (With the other parity and a tie, `middle(4, tie='high')` on 3 dice does keep all.) The
port copies it (`keep_tuple::middle_slice`).

Drafted upstream test:

```python
def test_middle_keep_more_than_size_keeps_all():
    assert d6.pool(3).middle(5).keep_tuple() == (1, 1, 1)
    assert d6.middle(3, 5).equals(3 @ d6)
```

### Sources with opposite `KeepSkip` preferences evaluate wrongly (`Order.Any` pop order) (K6)

`merge_order_preferences` (order.py) turns two conflicting preferences of the same priority into `Order.Any` of
the next priority. Two pools whose keep tuples skip at opposite ends (`d4.pool(3)[1:]` prefers descending,
`d4.pool(3)[:2]` ascending) merge to `(Order.Any, PoolComposition)`, and `Dungeon.evaluate` pops in that order.
But `Room.pop` takes the *min* outcome for any `order >= 0` (`if order < 0: ... else: outcomes[0]`), while
`PoolSource.pop` pops its *max* unless `order > 0`. So the room offers the min outcome, every source answers "not my
outcome" (count 0, weight 1), and the sources are never popped: `d4.pool(3)[1:].issubset(d4.pool(3)[:2])` is a die
with denominator 112 (`{False: 82, True: 30}`), where brute force gives `{False: 3786, True: 310}` over 4096. The
port copies it (`evaluator/mod.rs` `Dungeon::pop`, fixtures in fixtures/k6/evaluate.jsonl). Probably `Any` should
fall back to the default order before popping.

Drafted upstream test:

```python
def test_opposite_keep_skip_preferences():
    from collections import Counter
    import itertools
    a = d4.pool(3)[1:]
    b = d4.pool(3)[:2]
    expected = Counter()
    for x in itertools.product(range(1, 5), repeat=3):
        for y in itertools.product(range(1, 5), repeat=3):
            expected[Counter(sorted(x)[1:]) <= Counter(sorted(y)[:2])] += 1
    assert a.issubset(b).equals(Die(expected), simplify=True)
```
