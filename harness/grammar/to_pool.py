"""map_to_pool (map_tools/function.py) and the Die methods that make mixtures of pools with it (population/die.py):
map_to_pool, explode_to_pool, reroll_to_pool. Same rules as basic.py: the Python side is a direct call into icepool;
generators keep programs small.

A mixture's repr holds addresses, like any expression's, so these nodes are kind "mexpr" without a generator of
their own: `to_pool_evaluate` builds one (now and then under a keep-like operation) and evaluates it.
"""
import icepool

from harness.catalogue.functions import fn_node
from harness.core.grammar_api import node
from harness.grammar.basic import fresh_dz
from harness.grammar.pool import fresh_caches, straight_safe_method


@node("map_to_pool", "mexpr")
def map_to_pool(f, args, star):
    """icepool.map_to_pool(f, *args, star=star): f's result for each joint outcome of args (dice expand, anything
    else as-is, expressions too) is a pool (Pool(result) unless it is an expression; Reroll drops the outcome);
    the result is the mixture of those pools, weighted by the joint quantities."""
    fresh_dz()
    return icepool.map_to_pool(f, *args, star=star)


@node("die_map_to_pool", "mexpr")
def die_map_to_pool(die, f, args, star):
    """die.map_to_pool(f, *args, star=star): icepool.map_to_pool(f, die, *args); f=None uses the outcomes
    themselves as the pools' contents (the inverse of pool.expand())."""
    fresh_dz()
    return die.map_to_pool(f, *args, star=star)


@node("explode_to_pool", "mexpr")
def explode_to_pool(die, rolls, outcomes, star, depth):
    """die.explode_to_pool(rolls, outcomes, star=star, depth=depth): each explosion is a separate die of the pool
    (outcomes=None explodes the max outcome; no outcome selected or depth 0 is die.pool(rolls))."""
    return die.explode_to_pool(rolls, outcomes, star=star, depth=depth)


@node("reroll_to_pool", "mexpr")
def reroll_to_pool(die, rolls, outcomes, max_rerolls, star, depth, mode):
    """die.reroll_to_pool(rolls, outcomes, max_rerolls=..., star=..., depth=..., mode=...): at most max_rerolls
    rerolls shared across the pool, chosen by mode ('random', 'low', 'high', 'drop'); max_rerolls='inf' is
    die.reroll(outcomes, depth=depth).pool(rolls)."""
    return die.reroll_to_pool(rolls, outcomes, max_rerolls=max_rerolls, star=star, depth=depth, mode=mode)


# --- Generating --------------------------------------------------------------------------------------------------

_METHODS = ["sum", "sum", "size", "expand", "all_counts", "highest_outcome_and_count", "largest_count",
            "product_of_counts", "empty", "largest_straight"]


def _small_die(g):
    """A small die of small ints (a pool of these stays cheap), now and then an empty one or a non-int outcome."""
    r = g.rng
    k = r.random()
    if k < 0.45:
        return {"op": "d", "sides": r.randint(1, 4)}
    if k < 0.9:
        return {"op": "die", "outcomes": [r.randint(-2, 5) for _ in range(r.randint(1, 4))]}
    if k < 0.95:
        return {"op": "die", "outcomes": []}
    return {"op": "die", "outcomes": [r.randint(0, 3), {"float": "1.5"}, {"bool": True}][: r.randint(1, 3)]}


def _pair_die(g):
    """A die of pairs of small ints (for star=True and f=None: the outcomes are the pools' contents)."""
    r = g.rng
    if r.random() < 0.4:
        # The sorted tuples of a small pool, as in upstream's test of pool.expand().map_to_pool().
        return {"op": "mx_evaluate", "expr": {"op": "die_pool", "die": {"op": "d", "sides": r.randint(1, 3)},
                                              "rolls": r.randint(0, 2)}, "method": {"str": "expand"}}
    return {"op": "die", "outcomes": [{"tuple": [r.randint(0, 4) for _ in range(r.choice([2, 2, 1, 3]))]}
                                      for _ in range(r.randint(1, 3))]}


def _which(g, none_ok=True):
    """What to explode / reroll: None (the max / min outcome), a catalogue predicate or a collection."""
    r = g.rng.random()
    if none_ok and r < 0.35:
        return None
    if r < 0.6:
        return fn_node(g, pred=True)[0]
    if r < 0.65:
        return fn_node(g, name="mapping")[0]
    return [g.rng.randint(-2, 5) for _ in range(g.rng.randint(0, 3))]


def _star(g):
    return g.rng.choice([None] * 8 + [{"bool": True}, {"bool": False}])


def _gen_map_to_pool(g):
    r = g.rng
    k = r.random()
    if k < 0.25:
        return {"op": "die_map_to_pool", "die": _pair_die(g), "f": None, "args": [],
                "star": None if r.random() < 0.9 else _star(g)}
    name = r.choice(["n_of", "n_of", "pool_d", "pair_list", "reroll_or_pair", "pair_k", "tuple_d", "mapping"])
    f = fn_node(g, name=name)[0]
    if name == "pair_list":
        args = r.choice([[_pair_die(g)], [_small_die(g), _small_die(g)], [_small_die(g), r.randint(0, 3)]])
    else:
        args = [_small_die(g)] if r.random() < 0.9 else [_small_die(g), _small_die(g)]
    if r.random() < 0.5:
        return {"op": "die_map_to_pool", "die": args[0], "f": f, "args": args[1:], "star": _star(g)}
    if r.random() < 0.03:
        args = []
    return {"op": "map_to_pool", "f": f, "args": args, "star": _star(g)}


def _odd(g, x, strs=True):
    """x, or now and then a count of another type (bool, float, str), for which Python's arithmetic raises or not.
    No strs for `rolls`: `die.pool('x')` is a keep index, which the port does not model for strs."""
    if g.rng.random() < 0.95:
        return x
    return g.rng.choice([{"bool": True}, {"bool": False}, {"float": "2.0"}, {"float": "1.5"}, -2, 7]
                        + ([{"str": "inf"}, {"str": "x"}] if strs else []))


def _gen_explode_to_pool(g):
    r = g.rng
    return {"op": "explode_to_pool", "die": _small_die(g), "rolls": _odd(g, r.choice([0, 1, 1, 2, 2, 3, -1]), strs=False),
            "outcomes": _which(g), "star": _star(g), "depth": _odd(g, r.choice([0, 1, 1, 2, 2, 3, -1]))}


def _gen_reroll_to_pool(g):
    r = g.rng
    max_rerolls = r.choice([0, 1, 1, 2, 2, 3, {"str": "inf"}, -1 if r.random() < 0.2 else 1])
    depth = r.choice([1, 1, 1, 2, 3, {"str": "inf"}, 0 if r.random() < 0.3 else 1])
    mode = r.choice(["random", "random", "low", "high", "drop", "drop", "other" if r.random() < 0.1 else "low"])
    return {"op": "reroll_to_pool", "die": _small_die(g), "rolls": _odd(g, r.choice([0, 1, 1, 2, 2, 3, 4, -1]), strs=False),
            "outcomes": _which(g), "max_rerolls": _odd(g, max_rerolls), "star": _star(g),
            # (With max_rerolls 'inf' the depth goes to die.reroll, which the port only models for ints.)
            "depth": depth if max_rerolls == {"str": "inf"} else _odd(g, depth),
            "mode": {"str": mode}}


def _gen_wallenius_deal_expr(g):
    """Wallenius.deal is a map_to_pool over the sorted tuples of weights pulled (wallenius.py)."""
    from harness.grammar.function import _gen_wallenius
    p = _gen_wallenius(g, 0)
    del p["method"]
    return {**p, "op": "wallenius_deal_expr"}


def _gen_to_pool_evaluate(g, depth):
    from harness.grammar.multiset import _keep
    r = g.rng
    k = r.random()
    if k < 0.35:
        e = _gen_map_to_pool(g)
    elif k < 0.6:
        e = _gen_explode_to_pool(g)
    elif k < 0.85:
        e = _gen_reroll_to_pool(g)
    else:
        e = _gen_wallenius_deal_expr(g)
    if r.random() < 0.3:
        # Keep-like operations apply to each pool of a mixture of keep generators.
        e = _keep(g, e)
    return {"op": "to_pool_evaluate", "expr": e, "method": {"str": straight_safe_method(g, e, _METHODS)}}


@node("to_pool_evaluate", "die", gen=_gen_to_pool_evaluate, weight=1.0)
def to_pool_evaluate(expr, method):
    """getattr(expr, method)() for a mixture of pools (or what explode_to_pool / reroll_to_pool return when they
    shortcut to a single pool; or a Wallenius deal, also a map_to_pool)."""
    fresh_caches()
    return getattr(expr, method)()
