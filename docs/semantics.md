# Semantics the port must copy

Starting points found while analysing icepool v2.2.2 (see ANALYSIS.md). **The oracle is the authority**: when this
file and the oracle disagree, the oracle wins and this file gets corrected (via docs/NOTES.md; this file is protected).

## Identical

- A Die result is its full `items()` list: outcomes in icepool's sorted order, each with its exact integer quantity.
  Zero quantities that survive inside icepool survive in the port. icepool does **not** reduce fractions: a 2:2 coin
  is not a 1:1 coin.
- Errors: both sides raise = match. The type and message don't have to match.
- Floats are compared bit for bit (via Python `repr`).

## Numbers

- Quantities are unbounded integers. khepri's heavy case reaches 258-bit denominators: u64 and u128 are not enough.
  Use a big-integer type with a cheap small-value path.
- `Fraction` results (`probability`, `mean`, ...) are exact rationals.

## Outcomes follow Python's rules

- `bool` is an `int`: `True + True == 2`, `True * 2 == 2`, `-True == -1`. khepri relies on this:
  `(d20 > save) * woundsPerFailure`, `n @ (d20 > save)`.
- `True == 1` and `hash(True) == hash(1)`: `Die([True, 1])` merges them into **one** outcome (which one is kept is
  whatever Python's dict does: the first key inserted).
- Comparing incomparable types (an int with a str) raises, so building or sorting such a die raises.
- `tuple + tuple` concatenates. `Vector + Vector` is elementwise. `str + str` concatenates.
- `zero()` is `outcome * 0` (gives `''` for str, `()` for tuple).
- `/` on ints gives floats; `//` floors; `%` follows Python's sign rules; `**` with negative exponents gives floats.
- `Die.__eq__` returns a `DieWithTruth`: its truth value is structural equality, its die is outcome-wise `==`. The
  core has two separate operations; the Python facade restores the dual meaning.
- Sorting: outcomes must be totally ordered under Python's `<`. Mixed int/float/Fraction/bool sort together
  numerically.

## Algorithms

- Mixing dice (a map that returns Dice, `Die([die1, die2])`, Again, explode) uses `weighted_lcm`
  (`math.py`): denominators are scaled by gcd-minimal factors, not multiplied out. Copy it exactly, or the
  quantities differ even when the probabilities agree.
- `n @ die` scales each branch by `die.denominator() ** (max|n| - |n|)`.
- Multiset evaluation: backward DP preferred, forward as fallback; the order negotiation (`order.py`,
  `PoolSource.order_preference`) must be matched exactly. Note `order.pool_pop_order` is dead code: port
  `PoolSource`'s version.
- Use per-call caches in Rust rather than icepool's global ones, unless an observable result depends on caching.
