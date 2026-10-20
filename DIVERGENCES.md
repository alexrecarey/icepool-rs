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

## Where the port is not bit-exact

### Floats under WASM are within 2 ULP of Python's (B9)

CPython's `float ** float`, `math.log`, `math.log10` and complex powers/`abs` end in the platform libm (`pow`,
`log`, `exp`, `sin`, `cos`, `atan2`, `hypot`). The port calls Rust's `f64` methods, which natively are that same
platform libm, so native builds and the Python facade agree with the oracle bit for bit. Compiled to WASM, they are
Rust's own libm, which can round the last bit differently (e.g. `7.0 ** -2.0` was `0.020408163265306124`, not
`0.02040816326530612`). Alex accepted that (2026-10-17) over keeping line-by-line ports of glibc: WASM results are
checked within 2 ULP (`ICEPOOL_FLOAT_ULPS=2`). Not a Python bug, so no upstream test.

Functions that amplify rounding can exceed 2 ULP: a complex power `z ** w` with a non-real `w` computes its
phase from `atan2` and `log`, then takes `len * sin(phase)`; when `sin(phase)` is small relative to `cos(phase)`,
one ULP of phase is many ULPs of the imaginary part (`complex(-0.00024988505290350285, -0.5) ** complex(-5.0,
2.5)`: 23 ULP). dev/QUESTIONS.md L202610172222 asks how the WASM checks should treat these.

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

### `quantities('>=')` of an empty population is `(0,)`, so `probabilities('>=')` raises (S2a)

`Population.quantities('>=')` (population/base.py) is `itertools.accumulate(self.values()[:-1], operator.sub,
initial=self.denominator())`: with no outcomes that still yields the initial `0`, so an empty die has one `'>='`
(and one `'<'`) quantity but zero outcomes, and `probabilities('>=')` divides it by the zero denominator
(`ZeroDivisionError`) where `probabilities('==')` and `probabilities('<=')` return `()`. The port copies it
(`population.rs` `quantities`, fixtures in fixtures/s2a/statistics.jsonl). One element per outcome looks intended.

Drafted upstream test:

```python
def test_empty_quantities_ge():
    assert Die([]).quantities('>=') == ()
    assert Die([]).quantities('<') == ()
    assert Die([]).probabilities('>=') == ()
```

### `pad_to_denominator` raises `KeyError` for an absent outcome when nothing needs padding (S2a)

`Population.pad_to_denominator(denominator, outcome)` deletes the outcome when its new quantity is zero (`del
data[outcome]`), even when it was never there: `d6.pad_to_denominator(6, 7)` raises `KeyError: 7` instead of
returning `d6` unchanged. The port copies it (`population.rs` `pad_to_denominator`).

Drafted upstream test:

```python
def test_pad_to_same_denominator_absent_outcome():
    assert d6.pad_to_denominator(6, 7).equals(d6)
```


### Formatting a die with no columns raises `IndexError` (S2b)

**Fixed upstream in https://github.com/HighDiceRoller/icepool/pull/272 (pending merge); the port now follows the fix: one row per outcome. Old IndexError lines in s2b/format.jsonl are superseded by s2b/format-no-columns.jsonl.** The text below describes the old behaviour.

population/format.py `compute_alignments(rows)` starts with `['>'] * len(rows[0])`. When the column spec selects
no columns (`f'{d6:md:}'`, `html:`, `bbcode:`), or `*o` unpacks outcomes of length 0 (`Die(['']).format('md:*o')`,
`Die([()]).format('md:*o')`), `zip(*cols)` yields no rows and `rows[0]` raises `IndexError` instead of printing a
table without columns. `csv` does not compute alignments, so `f'{d6:csv:}'` is `'\r\n'`. Also, because
`quantities('>=')` of an empty die is `(0,)` (see above), `Die([]).format('csv:q>=')` has one data row `0`. The
port copies both (format.rs).

Drafted upstream test:

```python
def test_format_no_columns():
    assert f'{icepool.d6:md:}' == 'Die with denominator 6\n\n|\n|\n' + '|\n' * 6 + '\n'
```

### `d6 @ Again` is additive but every `@` raises `TypeError` in `again_count` mode (S3a)

`AgainExpression.__rmatmul__` marks `d6 @ Again` additive (upstream's own `test_is_additive` asserts it), but
`_again_count` replaces every argument that is not an `Again` with `0` (`_again_count_arg`), so it computes
`operator.matmul(0, 1)`: `TypeError: unsupported operand type(s) for @: 'int' and 'int'`. `2 @ Again` fails the
same way. Presumably the non-Again argument was meant to be kept as-is (`d6 @ 1`). The port copies the TypeError.

Drafted upstream test:

```python
def test_again_count_matmul():
    result = Die([1, 2, d(2) @ Again], again_count=1)
    expected = Die([1, 2, d(2) @ Die([1, 2])])  # or whatever again_count should give
    assert result == expected
```

### `Reroll` and dice among `again_count` outcomes count as rolls that end the chain (S3a)

`evaluate_agains_using_count` leaves `Reroll` (and empty dice) out of `not_again_die`, but `make_step_outcome`
treats every outcome that is not an `AgainExpression`, `Reroll` included, as `tupleize(zero, 1, -1)`: a terminal
roll, later replaced by a roll of `not_again_die`. So `Die([Reroll, 2, Again + 1], again_count=2)` gives `Reroll`
the weight of a terminal roll instead of rerolling it. In depth mode `Reroll` drops out of each level as usual. The
port copies it.

Drafted upstream test:

```python
def test_again_count_reroll_outcome():
    result = Die([Reroll, 2, Again + 1], again_count=2)
    assert result == Die([2, Again + 1], again_count=2)
```

### `Break(Reroll)` in `map(repeat)` raises; `mean_time_to_absorb` does not expand multiset arguments (S3b)

Two small inconsistencies in map_tools, both copied:

- `TransitionCache.step_state` turns `Break(Reroll)` into the tuple `(TransitionType.BREAK, Reroll)`, which hits
  the tuple-holding-`Reroll` problem above (`AttributeError`), while `is_self_loop` unwraps it and treats `Reroll`
  as a state different from the current one (so the state counts as transient). Without `repeat`, `map_simple`
  unwraps `Break(Reroll)` to `Reroll` and the outcome drops out. The port raises (`map_tools/core_impl.rs`,
  `Target::Reroll`).
- `map` and `map_and_time` replace a `MultisetExpression` among the extra arguments by `arg.expand()`;
  `mean_time_to_absorb` passes them to `TransitionCache` unexpanded (the facade keeps that Python code as is).

Drafted upstream tests:

```python
def test_break_reroll_repeat():
    # Like map without repeat, the Break(Reroll) branch should drop out rather than raise AttributeError:
    # 0 -> 1 -> Break(Reroll) is conditioned away, leaving nothing.
    result = icepool.map(lambda x: icepool.Break(icepool.Reroll) if x == 1 else x + 1, 0, repeat=3)
    assert result.is_empty()

def test_mean_time_to_absorb_expands_multisets():
    f = lambda s, t: min(s + sum(t), 6)
    assert icepool.mean_time_to_absorb(f, 0, icepool.d(2).pool(2)) == \
        icepool.mean_time_to_absorb(f, 0, icepool.d(2).pool(2).expand())
```

### `Deck.sequence(0)` raises `UnboundLocalError` (S4a)

`Deck.sequence(deals)` (population/deck.py) fills `_sequence_cache` in `for i in range(len(self._sequence_cache),
deals + 1): ... result = ...; self._sequence_cache.append(result)` and returns `result`, which is only bound inside
the loop. With `deals == 0` (the cache already holds `Die([()])`), and with any `deals` no larger than an earlier
call on the same deck, the loop never runs and `return result` raises `UnboundLocalError`. The intent is surely
`return self._sequence_cache[deals]`. The port raises for `deals == 0` (program/deck.rs `deck_sequence`; one call
per program, so the second case is not reachable there). The facade runs upstream's code unchanged.

Drafted upstream test:

```python
def test_sequence_zero_and_repeated():
    deck = Deck([1, 2, 2])
    assert deck.sequence(0).equals(Die([()]))
    assert deck.sequence(2).equals(deck.sequence(2))
    assert deck.sequence(1).equals(Die([(1,), (2,), (2,)]))
```

### A tuple holding `Reroll` in `Deck(...)` raises `AttributeError` unless its duplicates are 0 (S4a)

Same root as the `Die(...)` entry above: `expand_arg` hands `Reroll` itself on as the subdata, and
`merge_duplicates` calls `subdata.items()` on it, but only after `if subdup == 0: continue`. So
`Deck([(1, Reroll)], times=[0])` is an empty deck while `times=[1]` raises. Copied (program/deck.rs `new_deck`,
creation_args.py `expand_args_for_deck_rust`). The same test as above, for decks:

```python
def test_deck_tuple_with_reroll_drops_out():
    assert Deck([(1, Reroll), 2]) == Deck([2])
```

### `MultiDeal.denominator()` is not the total weight of its evaluations for three or more hands (S4b)

`MultiDeal._denominator` (generator/multi_deal.py, also `MultiDealSource`) is `comb(deck.size(), total) *
prod(comb(total, h) for h in hand_sizes()[1:])`. The ways to split `total` cards into hands of sizes
`h1, h2, ..., hn` are the multinomial `total! / (h1! h2! ... hn!)` = `comb(total, h1) * comb(total - h1, h2) * ...`,
which is what `pop` weighs with (`iter_hypergeom`), so an evaluation's result has that denominator. The two agree
for one or two hands but not beyond: `Deck(range(5)).deal((2, 2, 1)).denominator()` is 50 while evaluating it gives
denominator 30; `deal((1, 1, 1))`: 90 against 60. Evaluations are unaffected (the skip branch only multiplies by the
denominator of a source whose hands are all empty, which is 1). Copied (core `multi_deal.rs denominator`).

Drafted upstream test:

```python
def test_multi_deal_denominator_three_hands():
    class Counts(MultisetEvaluator):
        def next_state(self, state, order, outcome, counts):
            return (state or ()) + (counts,)
    for sizes in [(2, 2), (2, 2, 1), (1, 1, 1)]:
        deal = Deck(range(5)).deal(sizes)
        assert deal.denominator() == Counts().evaluate(deal).denominator()
```

### Hands of different multi-deals are equal expressions (S4d)

`MultisetTupleIndex.hash_key` (expression/multiset_tuple_expression.py) is `(type(self), self._index)`, and
`MultisetTupleSlice`'s is the type and the slice bounds: the child expression is left out. `equals` (and the truth
value of `==`, which a dict uses) compares hash keys, so `Deck([1, 2]).deal((1, 1))[0].equals(Deck([5,
6]).deal((2,))[0])`, and a `MultisetMixture` of two such hands counts them as one inner with weight 2, evaluating only
the first:
`MultisetMixture([Deck([1, 2]).deal((1, 1))[0], Deck([5, 6]).deal((1, 1))[0]]).sum()` is `Die({1: 1, 2: 1})`
instead of a mixture of 1, 2, 5 and 6. Copied (program/expression.rs `same_expression`); the facade runs upstream's
mixture code unchanged.

Drafted upstream test:

```python
def test_tuple_index_hash_key_includes_child():
    a = Deck([1, 2]).deal((1, 1))[0]
    b = Deck([5, 6]).deal((1, 1))[0]
    assert not a.equals(b)
    assert MultisetMixture([a, b]).sum().equals(Die([1, 2, 5, 6]))
```

### `Symbols.isdisjoint` returns whether the two *intersect* (S5a)

`Symbols.isdisjoint(other)` (collection/symbols.py) is `any(self[s] > 0 and other[s] > 0 for s in self)`: True when
some symbol is positive in both, the opposite of what the name and the set API mean (`{1}.isdisjoint({1})` is
False). Copied (core `Symbols::isdisjoint`).

Drafted upstream test:

```python
def test_symbols_isdisjoint():
    assert not Symbols('ab').isdisjoint('bc')
    assert Symbols('ab').isdisjoint('c')
```

### `Symbols.additive_union`, `difference`, `intersection`, `union` always raise before Python 3.14 (S5a)

They are `functools.reduce(operator.add, args, initial=self)`, and `reduce` only accepts `initial` as a keyword
from Python 3.14: on the oracle's Python (3.13) every call raises `TypeError: reduce() takes no keyword arguments`,
even with no arguments. Copied (program/collection.rs); the facade runs the same Python, so it raises the same way.
`reduce(operator.add, args, self)` works everywhere.

Drafted upstream test:

```python
def test_symbols_multi_argument_methods():
    assert Symbols('a').additive_union('b', 'b') == Symbols('abb')
    assert Symbols('abc').difference('a', 'b') == Symbols('c')
    assert Symbols('aab').intersection('ab') == Symbols('ab')
    assert Symbols('a').union() == Symbols('a')
```

### `Deck.additive_union`, `difference`, `intersection`, `union` always raise before Python 3.14 (S4)

The same `functools.reduce(..., initial=self)` as `Symbols`' (above), in population/deck.py: on the oracle's
Python 3.13 every call raises `TypeError: reduce() takes no keyword arguments`. Copied (program/deck.rs
`deck_method`); the facade runs upstream's Python here, so it raises the same way.

Drafted upstream test:

```python
def test_deck_multi_argument_methods():
    deck = Deck([1, 2, 2])
    assert deck.additive_union([3], [3]) == Deck([1, 2, 2, 3, 3])
    assert deck.difference([2], [1]) == Deck([2])
    assert deck.intersection([2, 2, 3]) == Deck([2, 2])
    assert deck.union() == deck
```

### A die of nested vectors cannot be sorted (S5a)

A comparison of two vectors of the same length takes its truth value from `op(self._data, other._data)`, a tuple
comparison, and Python's tuple comparison returns the comparison of the first differing elements *as is*. For
vectors of vectors that is again a `Vector`, so `bool()` of the outer result raises `TypeError: __bool__ should
return bool, returned Vector`. Hence `vectorize(die_of_vectors)`, `Die([Vector((Vector((1,)),)),
Vector((Vector((2,)),))])` and `d4 ** Die([Vector((-1,))])` vectorized all raise "Items do not appear to be
sortable". Copied (core `Vector`'s truth value is the raw object, `Value::binary` on tuples returns the element
comparison's object).

Drafted upstream test:

```python
def test_nested_vectors_sort():
    die = Die([Vector((Vector((2,)),)), Vector((Vector((1,)),))])
    assert die.outcomes() == (Vector((Vector((1,)),)), Vector((Vector((2,)),)))
```

### `==` between dice of vectors of different lengths only has a truth value (S5a)

`Vector((1,)) == Vector((1, 2))` is a `VectorWithTruthOnly`, whose hash raises `ValueError` (it reads `_data`), so
the data of `Die([Vector((1,))]) == Die([Vector((1, 2))])` raises, while its truth value (`equals`) works. The port
keeps the error for whoever reads the data (`PValue::DieWithTruthError`).

Drafted upstream test:

```python
def test_die_of_vectors_eq_different_lengths():
    result = Die([Vector((1,))]) == Vector((1, 2))
    assert not result
    assert result.equals(Die([False]))
```

### `pointwise_max(die)` / `pointwise_min(die)` with a single die iterate its outcomes (S5b)

function.py `pointwise_max(arg0, *more_args)` treats a lone argument as the iterable of dice, so
`pointwise_max(d6)` is the pointwise max over `Die([1])` ... `Die([6])` (each outcome made a die by
`harmonize_denominators`), i.e. `Die([6])`, and `pointwise_min(d6)` is `Die([1])`. The docstring says "Either an
iterable of dice, or two or more dice as separate arguments", so this is documented, but a single die is surely
meant as itself (as `icepool.lowest(d6)` / `highest(d6)` would suggest). The port copies it (program/function.rs
`pointwise`, fixtures/s5b/function).

Drafted upstream test:

```python
def test_pointwise_max_single_die():
    assert icepool.pointwise_max(icepool.d6).equals(icepool.d6)
    assert icepool.pointwise_min(icepool.d6).equals(icepool.d6)
```

### `consecutive(iterator)` raises ValueError (S5b)

function.py `consecutive(*args)` chains its arguments twice (`min(itertools.chain(*args), default=None)`, then
`max(itertools.chain(*args))`), so an iterator argument is exhausted by the `min` and the `max` raises `ValueError:
max() iterable argument is empty`. The facade copies it by materialising both chains (python/icepool/function.py);
the program language passes lists only.

Drafted upstream test:

```python
def test_consecutive_iterator():
    assert icepool.consecutive(iter([1, 3])) == (1, 2, 3)
```

### `d` / `z` are `@cache`d and keyed by `==`, so `z(Fraction(1))` succeeds after `z(True)` (S5)

function.py `d(sides)` and `z(sides)` are wrapped in `functools.cache`, which looks arguments up by hash and `==`
before the body runs. A lone `int` argument is its own key, but any other type is keyed as a one-element list, and
`[True] == [1.0] == [Fraction(1)]` (equal hashes too). So once `z(True)` is cached, `z(1.0)` and `z(Fraction(1))`
return that die instead of raising `TypeError('sides must be an int.')`; in a fresh process (or after only `z(1)`)
they raise. Results then depend on what the process computed before (the fresh-seed fuzz met it: a long-lived
oracle). The port has no cross-call cache and gives what a fresh process gives; the `d`, `z`, `dz_attr`, `d_pool`
and `z_pool` nodes call `fresh_dz()` (harness/grammar/basic.py: `d.cache_clear()`, `z.cache_clear()`) first so the
oracle does too (fixtures/s5/dz-cache runs the sequence in one process). The facade keeps upstream's `@cache`.

Drafted upstream test:

```python
def test_d_rejects_non_int_after_cache():
    icepool.d(True)
    with pytest.raises(TypeError):
        icepool.d(1.0)  # fails: returns the cached d(True)
```

### Values nested ~10000 deep raise `RecursionError` where CPython may still answer (R1)

CPython recurses on nested tuples in C: comparison and `repr` raise `RecursionError` from its C recursion limit
(`Py_C_RECURSION_LIMIT`, 10000 in 3.13: `t == u` for tuples nested 9998 deep, `str(Die([t]))` from 9993,
`Die([t, u])` from 9996), and hashing has no limit at all, so `Die([t])` works for a tuple nested 100000 deep
(and crashes the interpreter somewhere beyond). The port recurses natively and must not overflow its stack, so
every recursion on a nested value takes a level (`value/recursion.rs`) and raises `RecursionError` past 10000
levels *or* 6 MiB of stack (1000 levels / 512 KiB in WASM). So: anything on a tuple nested more than 10000 deep
raises (`Die([t])`, `tupleize` chains, `map(lambda s: (s,), repeat=n)` states), and near 10000 the exact level
differs from CPython's by a few (each CPython operation adds its own C levels; the port counts tuple levels
only). With `sys.setrecursionlimit` raised far, Vector operators and evaluations over many outcomes (one level
per outcome) also raise from the stack budget, where Python continues (or crashes: `Die(range(20000)).pool(1)
.sum()` at limit 100000 segfaults the oracle). Not a Python bug; listed so the threshold differences are known.

### Vector comparisons take exponential time in Python (R1)

`Vector.binary_operator` with `compare_for_truth` computes `op(self._data, other._data)` (a tuple comparison,
recursing into the elements' own comparison operators) and then the elementwise results again, so `t == u` or
`t < u` for vectors nested `n` deep takes `2**n` steps; from about 30 levels Python effectively hangs, from ~250
(`==`: ~500) it raises `RecursionError` after exploring. The port computes `==`'s truth value with tuple equality
(linear) but follows the same order otherwise, so it gives the same results and raises where Python eventually
would (from 250 levels at top level), immediately.

Drafted upstream test:

```python
def test_nested_vector_comparison_is_not_exponential():
    t = 1
    for _ in range(40):
        t = Vector([t])
    assert bool(t == t)  # currently ~2**40 steps
```

### `x in symbols` is True for anything with a `len()` (C6g)

`Symbols` is a `Mapping[str, int]` without its own `__contains__`, so `in` is `Mapping.__contains__`: `self[key]`,
False only on `KeyError`. But `Symbols.__getitem__` never raises `KeyError`: a one-character key missing from the
symbols counts 0, any other key gives a (sub-)Symbols. So `'z' in Symbols('ab')` is True, as is `(1, 2) in
Symbols('ab')`, and `1 in Symbols('ab')` raises `TypeError` (`len(1)`). It shows wherever a Symbols is the
collection of a selection (`die.reroll(Symbols('ab'), ...)`, `die.is_in(...)`, `to_one_hot(...)`, where
`symbols.index` is then `symbols['index']`, a Symbols, and calling it raises `TypeError`). The port copies it
(harness map.rs `contains`, js die.ts `memberships`).

Drafted upstream test:

```python
def test_symbols_contains_only_present_symbols():
    assert 'a' in Symbols('ab')
    assert 'z' not in Symbols('ab')  # fails: __getitem__ returns 0 instead of raising KeyError
```
