"""Weightless generators (generator/weightless.py): `g.weightless()` of a multiset generator, in which each possible
multiset is weighted 1. Same rules as basic.py: the Python side is a direct call into icepool; generators keep
programs small.

A `WeightlessGenerator`'s repr holds an address, like any expression's, so `weightless` (kind "mexpr") has no
generator of its own: `weightless_evaluate` builds one (alone, or as a leaf of a multiset expression) and evaluates
it.
"""
import icepool

from harness.core.grammar_api import node
from harness.grammar.pool import fresh_caches, straight_safe_method


@node("weightless", "mexpr")
def weightless(generator):
    """generator.weightless(): a WeightlessGenerator (itself if it already is one). Popping an outcome yields the
    wrapped source's possibilities with weight 1; two with the same count raise UnsupportedOrder (keeps and mixed
    pools usually do), so the evaluation tries the other order, then raises ConflictingOrderError. Only multiset
    generators (pools, deals, compound keeps) have the method."""
    return generator.weightless()


# --- Generating --------------------------------------------------------------------------------------------------

_METHODS = ["sum", "size", "expand", "expand", "all_counts", "highest_outcome_and_count", "largest_count",
            "product_of_counts", "empty", "largest_straight"]


def _generator(g, depth):
    """A multiset generator: a pool or deal, mostly without keeps (weightless rarely survives one), now and then
    a compound keep (`+` of two generators)."""
    from harness.grammar.pool import gen_pool
    r = g.rng
    leaf = lambda: (_small(g) if r.random() < 0.5 else
                    gen_pool(g, depth, negative=r.random() < 0.2, deals=True, expressions=False))
    if r.random() < 0.15:
        return {"op": "pool_add", "a": leaf(), "b": leaf()}
    p = leaf()
    # Strip a pool's keep-like operations most of the time: they usually make the same count twice.
    while r.random() < 0.6 and isinstance(p, dict) and p.get("op") in (
            "pool_slice", "pool_keep", "pool_highest", "pool_lowest", "pool_middle", "pool_mul", "pool_neg"):
        p = p["pool"]
    if r.random() < 0.2:
        # A keep over a small generator: some survive (a single die kept highest), most raise UnsupportedOrder.
        from harness.grammar.pool import _keep_op
        p = _keep_op(g, p, final=r.random() < 0.3)
    return p


def _small(g):
    """A small pool or deal of small ints, so that most evaluations get to pop."""
    from harness.grammar.to_pool import _small_die
    r = g.rng
    if r.random() < 0.3:
        outcomes = [r.randint(0, 4) for _ in range(r.randint(0, 6))]
        return {"op": "deal", "deck": {"op": "deck", "outcomes": outcomes, "times": r.choice([1, 1, 2])},
                "hand_size": r.randint(0, 4)}
    if r.random() < 0.3:
        return {"op": "pool", "dice": [_small_die(g) for _ in range(r.randint(1, 3))], "times": 1}
    return {"op": "die_pool", "die": _small_die(g), "rolls": r.randint(0, 4)}


def _weightless_leaf(g, depth):
    """`weightless()` of a generator (now and then twice, or of an expression, which has no such method)."""
    r = g.rng
    k = r.random()
    if k < 0.04:
        from harness.grammar.multiset import gen_expression
        inner = gen_expression(g, depth, budget=1)
    else:
        inner = _generator(g, depth)
    w = {"op": "weightless", "generator": inner}
    if r.random() < 0.08:
        w = {"op": "weightless", "generator": w}
    return w


def _leaf(g, depth):
    """A leaf of an expression: mostly weightless, now and then an ordinary generator beside it."""
    from harness.grammar.multiset import _leaf as plain_leaf
    return _weightless_leaf(g, depth) if g.rng.random() < 0.7 else plain_leaf(g, depth)


def _gen_weightless_evaluate(g, depth):
    from harness.grammar.multiset import gen_expression
    if g.rng.random() < 0.5:
        expr = _weightless_leaf(g, depth)
    else:
        expr = gen_expression(g, depth, budget=g.rng.choice([1, 1, 2]), leaf=_leaf)
    return {"op": "weightless_evaluate", "expr": expr, "method": {"str": straight_safe_method(g, expr, _METHODS)}}


@node("weightless_evaluate", "die", gen=_gen_weightless_evaluate, weight=0.5)
def weightless_evaluate(expr, method):
    """getattr(expr, method)() over an expression with weightless generators in it (= mx_evaluate): each distinct
    multiset counts once."""
    fresh_caches()
    return getattr(expr, method)()
