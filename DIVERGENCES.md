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
