"""map, mixtures and Reroll; reroll / filter / split / clip / truncate.

Callables come from harness/catalogue/functions.py through the `fn` node. Same rules as basic.py: the Python side
is a direct call into icepool; generators keep programs small.
"""
import icepool

from harness.catalogue.functions import FUNCTIONS, fn_node
from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome


@node("fn", "fn", lazy=("name",))
def fn(name, params):
    """FUNCTIONS[name](*params): a named callable (or mapping) from harness/catalogue/functions.py."""
    return FUNCTIONS[name](*params)


@node("Reroll", "reroll")
def reroll_const():
    """icepool.Reroll"""
    return icepool.Reroll


@node("tuple", "tuple")
def tuple_(items):
    """tuple(items): lets a program build tuples holding dice or Reroll."""
    return tuple(items)


def _star(g):
    return g.rng.choice([None] * 8 + [True, False])


def _gen_map(g, depth):
    f, e = fn_node(g)
    return {"op": "map", "f": f, "args": e.args(g, depth), "star": _star(g)}


@node("map", "die", gen=_gen_map, weight=1.5)
def map_(f, args, star):
    """icepool.map(f, *args, star=star): dice in args expand into their joint outcomes; results may be dice
    (mixed with weighted_lcm), Reroll, or tuples holding dice."""
    return icepool.map(f, *args, star=star)


def _kwargs(g):
    """Keyword arguments for map_kw: usually `k` (an int, a die passed as-is, or another scalar)."""
    from harness.grammar.operators import scalar
    r = g.rng.random()
    if r < 0.1:
        return []
    if r < 0.15:
        return [[{"str": "j"}, 1]]
    v = g.rng.random()
    k = g.rng.randint(-2, 6) if v < 0.5 else g.die(0) if v < 0.8 else scalar(g)
    return [[{"str": "k"}, k]]


def _gen_map_kw(g, depth):
    if g.rng.random() < 0.75:
        f, e = fn_node(g, name="add_kw")
    else:
        f, e = fn_node(g, kw=True)
    args = e.args(g, depth) if g.rng.random() < 0.9 else []
    return {"op": "map_kw", "f": f, "args": args, "star": _star(g), "kwargs": _kwargs(g)}


@node("map_kw", "die", gen=_gen_map_kw, weight=0.5)
def map_kw(f, args, star, kwargs):
    """icepool.map(f, *args, star=star, **dict(kwargs)): kwargs (a list of [name, value]) go to f as-is, dice
    included; with no args, the result is Die([f(**kwargs)])."""
    return icepool.map(f, *args, star=star, **{k: v for k, v in kwargs})


def _gen_map_function(g, depth):
    p = _gen_map_kw(g, depth) if g.rng.random() < 0.4 else dict(_gen_map(g, depth), kwargs=[])
    return dict(p, op="map_function", decorator=g.rng.random() < 0.5)


@node("map_function", "die", gen=_gen_map_function, weight=0.4)
def map_function(f, args, star, kwargs, decorator):
    """icepool.map_function(star=star, **dict(kwargs))(f)(*args) if decorator else
    icepool.map_function(f, star=star, **dict(kwargs))(*args): given f directly, star is dropped (only kwargs are
    bound)."""
    kw = {k: v for k, v in kwargs}
    if decorator:
        return icepool.map_function(star=star, **kw)(f)(*args)
    return icepool.map_function(f, star=star, **kw)(*args)


def _which(g, depth, none_ok):
    """What to select: a catalogue predicate, a collection of outcomes, or (reroll only) None."""
    r = g.rng.random()
    if none_ok and r < 0.15:
        return None
    if r < 0.55:
        return fn_node(g, pred=True)[0]
    if r < 0.6:
        return fn_node(g, name="mapping")[0]
    if r < 0.65:
        return {"tuple": [g.outcome() for _ in range(g.rng.randint(0, 3))]}
    return [g.outcome() if g.rng.random() < 0.9 else _typed_outcome(g, "num")
            for _ in range(g.rng.randint(0, 4))]


def _depth(g):
    return g.rng.choice([0, 1, 1, 2, 3, {"str": "inf"}, {"str": "inf"}, -1 if g.rng.random() < 0.3 else 1])


def _gen_reroll(op):
    def gen(g, depth):
        return {"op": op, "die": g.die(depth), "which": _which(g, depth, none_ok=(op == "reroll")),
                "star": _star(g), "depth": _depth(g)}
    return gen


@node("reroll", "die", gen=_gen_reroll("reroll"))
def reroll(die, which, star, depth):
    """die.reroll(which, star=star, depth=depth): which=None rerolls the min outcome; depth 'inf' rerolls forever."""
    return die.reroll(which, star=star, depth=depth)


@node("filter", "die", gen=_gen_reroll("filter"))
def filter_(die, which, star, depth):
    """die.filter(which, star=star, depth=depth): rerolls until one of the selected outcomes."""
    return die.filter(which, star=star, depth=depth)


@node("split", "dice_pair", weight=0.5,
      gen=lambda g, depth: {"op": "split", "die": g.die(depth), "which": _which(g, depth, none_ok=False),
                            "star": _star(g)})
def split(die, which, star):
    """die.split(which, star=star): (selected outcomes, the rest), a tuple of two dice."""
    return die.split(which, star=star)


def _bound(g):
    r = g.rng.random()
    if r < 0.3:
        return None
    if r < 0.9:
        return g.outcome()
    return _typed_outcome(g, "num")


def _gen_bounds(op):
    def gen(g, depth):
        return {"op": op, "die": g.die(depth), "min": _bound(g), "max": _bound(g)}
    return gen


@node("clip", "die", gen=_gen_bounds("clip"), weight=0.7)
def clip(die, min, max):
    """die.clip(min, max): outcomes beyond a bound become that bound."""
    return die.clip(min, max)


@node("truncate", "die", gen=_gen_bounds("truncate"), weight=0.7)
def truncate(die, min, max):
    """die.truncate(min, max): outcomes beyond a bound are dropped (rerolled)."""
    return die.truncate(min, max)


def _mixture_outcome(g, depth):
    r = g.rng.random()
    if r < 0.35:
        return g.die(min(depth, 1))
    if r < 0.45:
        return {"op": "Reroll"}
    if r < 0.6:
        # A tuple holding dice: tupleized into a die of tuples (or, holding Reroll, raises).
        items = [g.die(0) if g.rng.random() < 0.6 else g.outcome() for _ in range(g.rng.randint(1, 3))]
        if g.rng.random() < 0.1:
            items.append({"op": "Reroll"})
        return {"op": "tuple", "items": items}
    return g.outcome()


def _gen_mixture(g, depth):
    n = g.rng.randint(1, 4)
    outcomes = [_mixture_outcome(g, depth) for _ in range(n)]
    times = 1 if g.rng.random() < 0.3 else [g.rng.choice([0, 1, 1, 2, 3, 5]) for _ in range(n)]
    return {"op": "mixture", "outcomes": outcomes, "times": times}


@node("mixture", "die", gen=_gen_mixture, weight=0.7)
def mixture(outcomes, times):
    """icepool.Die(outcomes, times) where outcomes mix dice, Reroll, and tuples holding dice."""
    return icepool.Die(outcomes, times)
