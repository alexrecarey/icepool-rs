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
