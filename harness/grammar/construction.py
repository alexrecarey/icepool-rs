"""Die construction beyond plain ints: Python's outcome types, and `Die(outcomes, times)` mixing dice.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.
"""
from fractions import Fraction

import icepool

from harness.core.grammar_api import node


def _typed_outcome(g, kind):
    r = g.rng
    if kind == "num":
        k = r.choice(["int", "int", "bool", "float", "frac"])
        if k == "int":
            return g.outcome()
        if k == "bool":
            return {"bool": r.random() < 0.5}
        if k == "float":
            # Halves and quarters collide with ints and Fractions; the specials exercise ordering and repr.
            x = r.choice([r.randint(-8, 24) / 4, r.choice([float("inf"), float("-inf"), 1e16, 1e-5, 0.1, -0.0])])
            return {"float": repr(x)}
        f = Fraction(r.randint(-8, 12), r.randint(1, 4))
        return {"frac": [f.numerator, f.denominator]}
    if kind == "str":
        return {"str": "".join(r.choice("aAb") for _ in range(r.randint(0, 2)))}
    if kind == "tuple":
        return {"tuple": [_typed_outcome(g, r.choice(["num", "num", "str"]) if r.random() < 0.2 else "num")
                          for _ in range(r.randint(0, 2))]}
    return _typed_outcome(g, r.choice(["num", "str", "tuple"]))


def _gen_die_typed(g, depth):
    # Mostly one kind per die (so it usually sorts); sometimes a free mix, which usually raises as in Python.
    kind = g.rng.choice(["num", "num", "num", "str", "tuple", "mixed"])
    return {"op": "die_typed", "outcomes": [_typed_outcome(g, kind) for _ in range(g.rng.randint(1, 5))]}


@node("die_typed", "die", gen=_gen_die_typed, leaf=True, weight=0.5)
def die_typed(outcomes):
    """icepool.Die(list) over Python's outcome types: bool merges with 1, numbers compare exactly, str/tuple."""
    return icepool.Die(outcomes)


def _gen_die_times(g, depth):
    r = g.rng
    n = r.randint(1, 4)
    outcomes = [g.die(min(depth, 1)) if r.random() < 0.5 else g.outcome() for _ in range(n)]
    if r.random() < 0.3:
        times = r.choice([0, 1, 2, 3, -1])
    else:
        # Occasionally the wrong length, which raises.
        m = n if r.random() < 0.9 else r.randint(0, 5)
        times = [r.choice([0, 1, 1, 2, 3, 5, -1 if r.random() < 0.1 else 4]) for _ in range(m)]
    return {"op": "die_times", "outcomes": outcomes, "times": times}


@node("die_times", "die", gen=_gen_die_times, weight=0.7)
def die_times(outcomes, times):
    """icepool.Die(outcomes, times): dice among the outcomes are mixed in with weighted_lcm scaling."""
    return icepool.Die(outcomes, times)
