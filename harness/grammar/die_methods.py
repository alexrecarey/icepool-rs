"""Die methods without a callable: zero, simplify, _pop_min / _pop_max, and icepool.from_cumulative.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.
"""
import icepool

from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome


@node("zero", "die", gen=lambda g, depth: {"op": "zero", "die": g.die(depth)}, weight=0.5)
def zero(die):
    """die.zero(): every outcome times 0, which must resolve to a single outcome (same denominator)."""
    return die.zero()


def _gen_simplify(g, depth):
    # Most generated dice are already simplified: scale one up first, sometimes with a zero quantity.
    die = g.die(depth)
    if g.rng.random() < 0.7:
        k = g.rng.choice([2, 3, 6])
        die = {"op": "die_times", "outcomes": [die, g.outcome()], "times": [k, g.rng.choice([0, k, 2 * k])]}
    return {"op": "simplify", "die": die}


@node("simplify", "die", gen=_gen_simplify, weight=0.5)
def simplify(die):
    """die.simplify(): all quantities divided by their greatest common denominator; zero quantities drop out."""
    return die.simplify()


def _gen_pop(g, depth):
    return {"op": "die_pop", "die": g.die(depth), "max": {"bool": g.rng.random() < 0.5}}


@node("die_pop", "value", gen=_gen_pop, weight=0.4)
def die_pop(die, max):
    """die._pop_max() if max else die._pop_min(): (the die without that outcome, zero quantities kept; the
    quantity of the outcome removed)."""
    return die._pop_max() if max else die._pop_min()


def _cumulative(g, n):
    """Mostly non-decreasing ints; sometimes a decrease (Die raises), a bool or a float."""
    total, out = 0, []
    for _ in range(n):
        total += g.rng.choice([0, 1, 1, 2, 3, 5])
        out.append(total)
    r = g.rng.random()
    if out and r < 0.1:
        out[g.rng.randrange(n)] -= g.rng.randint(1, 3)
    elif out and r < 0.15:
        out[g.rng.randrange(n)] = {"bool": g.rng.random() < 0.5}
    elif out and r < 0.2:
        i = g.rng.randrange(n)
        out[i] = {"float": repr(float(out[i]))}
    elif r < 0.23:
        out = out[:g.rng.randint(0, max(n - 1, 0))]
    return out


def _gen_from_cumulative(g, depth):
    n = g.rng.randint(0, 5)
    if g.rng.random() < 0.8:
        outcomes = sorted(g.rng.sample(range(-4, 13), n))
    else:
        # Unsorted, repeated (a dict keeps the last difference), typed, or a die (mixed in by Die()).
        outcomes = [g.outcome() if g.rng.random() < 0.7 else
                    (g.die(0) if g.rng.random() < 0.3 else _typed_outcome(g, "num")) for _ in range(n)]
    return {"op": "from_cumulative", "outcomes": outcomes, "cumulative": _cumulative(g, n),
            "reverse": {"bool": g.rng.random() < 0.4}}


@node("from_cumulative", "die", gen=_gen_from_cumulative, weight=0.5)
def from_cumulative(outcomes, cumulative, reverse):
    """icepool.from_cumulative(outcomes, cumulative, reverse=reverse) with int cumulative quantities."""
    return icepool.from_cumulative(outcomes, cumulative, reverse=reverse)
