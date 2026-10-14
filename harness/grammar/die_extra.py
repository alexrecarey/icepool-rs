"""The rest of die.py / population/base.py that S2 found without a node: zero_outcome, standardized_moment, sign,
cmp, is_in, count, if_else and repr.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.
"""
import icepool

from harness.core.grammar_api import node
from harness.grammar.again import _again_end, _again_expr
from harness.grammar.collection import _gen_die_vectors
from harness.grammar.construction import _typed_outcome
from harness.grammar.statistics import _stat_die


def _some_die(g, depth):
    """`_stat_die` (typed outcomes too), and sometimes the result of `==` (a DieWithTruth) or a die of vectors."""
    r = g.rng.random()
    if r < 0.1:
        return {"op": "eq", "a": g.die(0), "b": g.die_or_int(0)}
    if r < 0.15:
        return _gen_die_vectors(g, depth)
    return _stat_die(g, depth)


@node("zero_outcome", "value", weight=0.3, gen=lambda g, depth: {"op": "zero_outcome", "die": _some_die(g, depth)})
def zero_outcome(die):
    """die.zero_outcome(): die.zero().outcomes()[0] (zero raises unless the zeros merge into one outcome)."""
    return die.zero_outcome()


def _moment_k(g):
    # No fractional k: a negative deviation ** 0.5 is a complex number, and the port has no complex arithmetic
    # yet (task V1). A float k with an integral value keeps everything real.
    return g.rng.choice([0, 1, 2, 3, 4, 5, -1, {"float": "2.0"}, {"float": "3.0"}])


@node("standardized_moment", "value", weight=0.4,
      gen=lambda g, depth: {"op": "standardized_moment", "die": _stat_die(g, depth), "k": _moment_k(g)})
def standardized_moment(die, k):
    """die.standardized_moment(k): sum(p * (outcome - mean)**k) / sd**k, in floats."""
    return die.standardized_moment(k)


@node("sign", "die", weight=0.3, gen=lambda g, depth: {"op": "sign", "die": _some_die(g, depth)})
def sign(die):
    """die.sign(): 1 if an outcome is > its own `x * 0`, -1 if <, else 0 (nan and -0.0 give 0)."""
    return die.sign()


def _other(g, depth):
    r = g.rng.random()
    if r < 0.6:
        return g.die(depth)
    if r < 0.85:
        return g.outcome()
    return _typed_outcome(g, "any")


@node("die_cmp", "die", weight=0.3,
      gen=lambda g, depth: {"op": "die_cmp", "die": _some_die(g, depth), "other": _other(g, depth)})
def die_cmp(die, other):
    """die.cmp(other): -1, 0, 1 weighted by the True quantities of die < other, die == other, die > other."""
    return die.cmp(other)


def _container(g):
    """What `x in outcomes` searches: a list or tuple of outcomes, or sometimes a str, a die, or an int (which
    raises TypeError once there is an outcome to test)."""
    r = g.rng.random()
    items = [g.outcome() if g.rng.random() < 0.85 else _typed_outcome(g, "num") for _ in range(g.rng.randint(0, 4))]
    if r < 0.5:
        return items
    if r < 0.8:
        return {"tuple": items}
    if r < 0.88:
        return {"str": g.rng.choice(["", "a", "ab", "Ab"])}
    if r < 0.95:
        return g.die(0)
    return g.outcome()


@node("is_in", "die", weight=0.3,
      gen=lambda g, depth: {"op": "is_in", "die": _some_die(g, depth), "outcomes": _container(g)})
def is_in(die, outcomes):
    """die.is_in(outcomes): a die of `x in outcomes`."""
    return die.is_in(outcomes)


@node("die_count", "die", weight=0.3,
      gen=lambda g, depth: {"op": "die_count", "die": _some_die(g, depth),
                            "rolls": g.rng.choice([0, 1, 2, 3, -2]) if g.rng.random() < 0.9 else
                            {"op": "d", "sides": g.rng.randint(1, 3)},
                            "outcomes": _container(g)})
def die_count(die, rolls, outcomes):
    """die.count(rolls, outcomes): rolls @ die.is_in(outcomes)."""
    return die.count(rolls, outcomes)


def _branch(g, depth):
    r = g.rng.random()
    if r < 0.4:
        return g.outcome()
    if r < 0.6:
        return g.die(0)
    if r < 0.8:
        return _again_expr(g, 1, True)
    if r < 0.9:
        return {"op": "Reroll"}
    return _typed_outcome(g, "num")


def _gen_if_else(g, depth):
    count, depth_arg = None, None
    r = g.rng.random()
    if r < 0.2:
        count = g.rng.randint(0, 3)
    elif r < 0.5:
        depth_arg = g.rng.choice([0, 1, 2])
    elif r < 0.53:
        count, depth_arg = 1, 1  # both: ValueError when there is an Again
    return {"op": "if_else", "die": _some_die(g, depth), "if_true": _branch(g, depth), "if_false": _branch(g, depth),
            "count": count, "depth": depth_arg, "end": _again_end(g)}


@node("if_else", "die", weight=0.4, gen=_gen_if_else)
def if_else(die, if_true, if_false, count, depth, end):
    """die.if_else(if_true, if_false, again_count=count, again_depth=depth, again_end=end): truthy outcomes become
    if_true, falsy ones if_false; the again arguments go to the final Die."""
    return die.if_else(if_true, if_false, again_count=count, again_depth=depth, again_end=end)


@node("die_repr", "value", weight=0.3, gen=lambda g, depth: {"op": "die_repr", "die": _some_die(g, depth)})
def die_repr(die):
    """repr(die): `Die({outcome!r: quantity, ...})`, or `DieWithTruth(...)` for the result of ==."""
    return repr(die)
