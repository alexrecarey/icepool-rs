"""Again (depth mode) and explode. Same rules as basic.py: the Python side is a direct call into icepool; generators
keep programs small.

An `Again` expression is built with the ordinary operator nodes (add, matmul, neg, ...) applied to the `Again`
node. A bare expression is never a program result (its repr holds an address), so it is only generated inside the
outcomes of `again`.
"""
import icepool

from harness.core.grammar_api import node
from harness.grammar.maps import _star, _which


@node("Again", "again_expr")
def again_const():
    """icepool.Again"""
    return icepool.Again


def _small(g):
    return g.rng.randint(-2, 4)


def _again_expr(g, depth, counts):
    """`Again`, or an operator applied to an expression and a small int or die. `counts` allows `Again @ die`."""
    if depth <= 0 or g.rng.random() < 0.3:
        return {"op": "Again"}
    inner = _again_expr(g, depth - 1, counts)
    r = g.rng.random()
    if r < 0.4:
        op = g.rng.choice(["add", "add", "sub", "mul", "floordiv", "lt", "ge"])
        # Multiply only by small ints: products grow at every level, and a die of big counts on the left of `@`
        # makes Python's `_sum_all` recurse past its limit.
        other = _small(g) if op in ("mul", "floordiv") or g.rng.random() < 0.7 else g.die(0)
        a, b = (inner, other) if g.rng.random() < 0.6 else (other, inner)
        return {"op": op, "a": a, "b": b}
    if r < 0.55:
        return {"op": g.rng.choice(["neg", "abs"]), "a": inner}
    if r < 0.75:
        # `k @ Again`: roll again k times and add those up.
        return {"op": "matmul", "n": g.rng.randint(0, 2), "die": inner}
    if r < 0.85 and counts:
        # `Again @ die`: roll again, then roll die that many times. Only on a bare Again, and with a small die.
        return {"op": "matmul", "n": {"op": "Again"}, "die": {"op": "d", "sides": g.rng.randint(1, 3)}}
    return {"op": "add", "a": inner, "b": _again_expr(g, depth - 1, counts)}


def _again_end(g):
    r = g.rng.random()
    if r < 0.5:
        return None
    if r < 0.7:
        return g.outcome()
    if r < 0.85:
        return {"op": "Reroll"}
    return g.die(0)


def _gen_again(g, depth):
    n = g.rng.randint(1, 4)
    depth_arg = g.rng.choice([None, None, 0, 1, 2, 3, -1 if g.rng.random() < 0.2 else 1])
    # `Again @ die` rolls the die as many times as the level below rolled: keep that to one level.
    counts = depth_arg in (None, 0, 1, -1)
    outcomes = []
    for _ in range(n):
        r = g.rng.random()
        if r < 0.4:
            outcomes.append(_again_expr(g, 2, counts))
        elif r < 0.55:
            outcomes.append(g.die(0))
        elif r < 0.6:
            outcomes.append({"op": "Reroll"})
        else:
            outcomes.append(g.outcome())
    return {"op": "again", "outcomes": outcomes, "depth": depth_arg, "end": _again_end(g)}


@node("again", "die", gen=_gen_again, weight=0.7)
def again(outcomes, depth, end):
    """icepool.Die(outcomes, again_depth=depth, again_end=end): outcomes may be Again expressions, rolled again up
    to depth times (None: the default, 1); past that, Again is end (None: a zero; Reroll: those outcomes drop)."""
    return icepool.Die(outcomes, again_depth=depth, again_end=end)


def _gen_explode(g, depth):
    return {"op": "explode", "die": g.die(depth), "which": _which(g, depth, none_ok=True), "star": _star(g),
            "depth": g.rng.choice([0, 1, 1, 2, 3, 4, -1 if g.rng.random() < 0.2 else 2]),
            "end": _again_end(g)}


@node("explode", "die", gen=_gen_explode)
def explode(die, which, star, depth, end):
    """die.explode(which, star=star, depth=depth, end=end): the selected outcomes (None: the max) roll again and add,
    at most depth more times; past that the next roll counts as end (None: a zero; Reroll: reroll it)."""
    return die.explode(which, star=star, depth=depth, end=end)
