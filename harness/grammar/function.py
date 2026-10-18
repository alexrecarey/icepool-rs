"""function.py's free functions (z and the dN / zN attributes, coin, stochastic_round, one_hot, pointwise_max/min,
min/max_outcome, consecutive, sorted_union, harmonize_denominators, from_rv), map_tools/function.py reduce /
accumulate, and Wallenius.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.
"""
from fractions import Fraction

import icepool

from harness.catalogue.functions import fn_node
from harness.catalogue.rvs import RVS
from harness.core.grammar_api import node
from harness.grammar.basic import fresh_dz
from harness.grammar.construction import _typed_outcome


def _sides(g):
    r = g.rng
    x = r.random()
    if x < 0.85:
        return r.randint(1, 8)
    if x < 0.92:
        return r.choice([0, -1])
    return {"bool": True} if x < 0.95 else _typed_outcome(g, "num")


@node("z", "die", gen=lambda g, depth: {"op": "z", "sides": _sides(g)}, leaf=True, weight=0.3)
def z(sides):
    """icepool.z(sides): a fair die numbered 0..sides-1 (TypeError for a non-int, ValueError below 1)."""
    fresh_dz()
    return icepool.z(sides)


def _gen_dz_attr(g, depth):
    r = g.rng
    if r.random() < 0.8:
        name = r.choice("dz") + str(r.randint(-1, 12))
    else:
        # What int() accepts or refuses: signs, underscores, spaces; and the empty name (IndexError).
        name = r.choice("dzx") + "".join(r.choice("0123456789_+- ") for _ in range(r.randint(0, 3)))
        try:
            if abs(int(name[1:])) > 99:
                # Three digits: a die of up to 999 outcomes, which pooled lands in the band where Python's
                # recursion limit is not knowable from the program (dev/NOTES.md, "Python's recursion limit").
                name = name[:-1]
        except ValueError:
            pass
        if r.random() < 0.05:
            name = ""
    if name in vars(icepool.function) or name in vars(icepool):
        # A name icepool defines (d, z): not __getattr__.
        name += "1"
    return {"op": "dz_attr", "name": {"str": name}}


@node("dz_attr", "die", gen=_gen_dz_attr, leaf=True, weight=0.3)
def dz_attr(name):
    """getattr(icepool, name) for names icepool does not define: function.py's module __getattr__ makes d<int> /
    z<int> dice; anything else (incl. d0: d's ValueError is swallowed) is AttributeError."""
    fresh_dz()
    return getattr(icepool, name)


def _probability(g):
    r = g.rng
    x = r.random()
    if x < 0.4:
        return r.randint(-1, 7)
    if x < 0.6:
        return {"float": repr(r.choice([0.0, 0.25, 0.5, 1 / 3, 0.1, 0.9, 1.0, 1.5, -0.5, 2 / 7]))}
    if x < 0.8:
        f = Fraction(r.randint(-2, 9), r.randint(1, 7))
        return {"frac": [f.numerator, f.denominator]}
    if x < 0.9:
        return {"bool": r.random() < 0.5}
    return {"float": r.choice(["nan", "inf"])} if r.random() < 0.3 else _typed_outcome(g, "num")


def _max_denominator(g):
    r = g.rng
    return None if r.random() < 0.5 else r.choice([1, 2, 3, 5, 10, 100, 0, -1])


def _gen_coin(g, depth):
    r = g.rng
    d = 1 if r.random() < 0.5 else (r.randint(0, 8) if r.random() < 0.9 else _probability(g))
    return {"op": "coin", "n": _probability(g), "d": d, "max_denominator": _max_denominator(g)}


@node("coin", "die", gen=_gen_coin, leaf=True, weight=0.5)
def coin(n, d, max_denominator):
    """icepool.coin(n, d, max_denominator=max_denominator): True with probability n / d (a non-int n is a
    probability, made a Fraction and limit_denominator'ed; then d must be 1)."""
    return icepool.coin(n, d, max_denominator=max_denominator)


def _gen_stochastic_round(g, depth):
    r = g.rng
    x = _probability(g) if r.random() < 0.3 else _typed_outcome(g, "num")
    return {"op": "stochastic_round", "x": x, "max_denominator": _max_denominator(g)}


@node("stochastic_round", "die", gen=_gen_stochastic_round, leaf=True, weight=0.4)
def stochastic_round(x, max_denominator):
    """icepool.stochastic_round(x, max_denominator=max_denominator): floor(x) + coin(x - floor(x))."""
    return icepool.stochastic_round(x, max_denominator=max_denominator)


def _gen_die_stochastic_round(g, depth):
    r = g.rng
    if r.random() < 0.8:
        # Fractional outcomes (no NaN: a die cannot sort it), now and then an int, a bool or a str.
        outcomes = [_probability(g) for _ in range(r.randint(1, 4))]
        outcomes = [o for o in outcomes if o not in ({"float": "nan"},)] or [{"frac": [1, 2]}]
        die = {"op": "die_typed", "outcomes": outcomes}
    else:
        die = g.make("die", depth)
    return {"op": "die_stochastic_round", "die": die, "max_denominator": _max_denominator(g)}


@node("die_stochastic_round", "die", gen=_gen_die_stochastic_round, weight=0.3)
def die_stochastic_round(die, max_denominator):
    """die.stochastic_round(max_denominator=max_denominator): die.map(lambda x: icepool.stochastic_round(x,
    max_denominator=max_denominator)), each outcome rounded up with probability x - floor(x)."""
    return die.stochastic_round(max_denominator=max_denominator)



@node("one_hot", "die", gen=lambda g, depth: {"op": "one_hot", "sides": _sides(g)}, leaf=True, weight=0.3)
def one_hot(sides):
    """icepool.one_hot(sides): a die of Vectors with one True (range(sides): a non-positive int is empty)."""
    return icepool.one_hot(sides)


def _dice(g, depth, n=None):
    """A few dice, now and then a plain outcome (implicitly a die)."""
    r = g.rng
    n = r.randint(0, 3) if n is None else n
    return [g.die(min(depth, 1)) if r.random() < 0.85 else g.outcome() for _ in range(n)]


def _gen_pointwise(g, depth):
    r = g.rng
    spread = r.random() < 0.6
    args = _dice(g, depth, r.randint(1 if spread else 0, 3))
    if not spread and r.random() < 0.1:
        # A single die: iterated, so its outcomes are the dice.
        args = g.die(min(depth, 1))
    return {"op": "pointwise", "args": args, "max": {"bool": r.random() < 0.5}, "spread": {"bool": spread}}


@node("pointwise", "die", gen=_gen_pointwise, weight=0.6)
def pointwise(args, max, spread):
    """(icepool.pointwise_max if max else icepool.pointwise_min)(*args if spread else args): the highest chance
    of rolling >= (max) or <= (min) each outcome among the dice, after harmonize_denominators. One argument is
    the iterable of dice (a die: its outcomes)."""
    f = icepool.pointwise_max if max else icepool.pointwise_min
    return f(*args) if spread else f(args)


def _gen_extreme(g, depth):
    r = g.rng
    spread = r.random() < 0.6
    args = [g.die(min(depth, 1)) if r.random() < 0.6 else
            (g.outcome() if r.random() < 0.8 else _typed_outcome(g, "num")) for _ in range(r.randint(0, 3))]
    if r.random() < 0.05:
        args.append(_typed_outcome(g, "str"))
    if not spread and r.random() < 0.15:
        args = g.die(min(depth, 1))
    return {"op": "extreme_outcome", "args": args, "max": {"bool": r.random() < 0.5}, "spread": {"bool": spread}}


@node("extreme_outcome", "value", gen=_gen_extreme, weight=0.5)
def extreme_outcome(args, max, spread):
    """(icepool.max_outcome if max else icepool.min_outcome)(*args if spread else args): Python's min / max over
    the outcomes of the populations and the single outcomes (one argument: iterated instead)."""
    f = icepool.max_outcome if max else icepool.min_outcome
    return f(*args) if spread else f(args)


def _int_set(g, depth):
    r = g.rng
    x = r.random()
    if x < 0.15:
        return g.die(min(depth, 1))
    if x < 0.2:
        return {"tuple": [_typed_outcome(g, "num") for _ in range(r.randint(0, 2))]}
    return [r.randint(-5, 12) for _ in range(r.randint(0, 3))]


@node("consecutive", "value", weight=0.4,
      gen=lambda g, depth: {"op": "consecutive", "args": [_int_set(g, depth) for _ in range(g.rng.randint(0, 3))]})
def consecutive(args):
    """icepool.consecutive(*args): range(min, max + 1) over every element of every argument, as a tuple (() if
    there is none; non-ints raise in range())."""
    return icepool.consecutive(*args)


def _outcome_set(g, depth):
    r = g.rng
    x = r.random()
    if x < 0.3:
        return g.die(min(depth, 1))
    kind = r.choice(["num", "num", "str", "tuple"]) if r.random() < 0.9 else "mixed"
    if r.random() < 0.5:
        return [g.outcome() for _ in range(r.randint(0, 4))]
    return [_typed_outcome(g, kind) for _ in range(r.randint(0, 3))]


@node("sorted_union", "value", weight=0.4,
      gen=lambda g, depth: {"op": "sorted_union", "args": [_outcome_set(g, depth)
                                                           for _ in range(g.rng.randint(0, 3))]})
def sorted_union(args):
    """icepool.sorted_union(*args): the union of the arguments as sets, sorted, as a tuple."""
    return icepool.sorted_union(*args)


def _gen_harmonize(g, depth):
    r = g.rng
    dice = _dice(g, depth)
    weights = None
    if r.random() < 0.5:
        n = len(dice) if r.random() < 0.85 else r.randint(0, 4)
        weights = [r.choice([0, 1, 1, 2, 3, 5]) if r.random() < 0.9 else
                   r.choice([-1, {"float": "1.5"}, {"bool": True}, {"frac": [1, 2]}]) for _ in range(n)]
    return {"op": "harmonize_denominators", "dice": dice, "weights": weights}


@node("harmonize_denominators", "value", gen=_gen_harmonize, weight=0.5)
def harmonize_denominators(dice, weights):
    """icepool.harmonize_denominators(dice) (weights None) or (dice, weights): a tuple of the dice with
    quantities scaled so the denominators are proportional to the weights (zip stops at the shorter)."""
    if weights is None:
        return icepool.harmonize_denominators(dice)
    return icepool.harmonize_denominators(dice, weights)


def _two_arg_fn(g):
    if g.rng.random() < 0.9:
        return fn_node(g, name=g.rng.choice(["add2", "max2", "max2", "break_eq", "count_diff"]))[0]
    return fn_node(g)[0]


def _gen_reduce(op):
    def gen(g, depth):
        r = g.rng
        dice = _dice(g, depth, r.randint(0, 3))
        initial = None if r.random() < 0.6 else (g.die(min(depth, 1)) if r.random() < 0.7 else g.outcome())
        return {"op": op, "f": _two_arg_fn(g), "dice": dice, "initial": initial}
    return gen


@node("reduce", "die", gen=_gen_reduce("reduce"), weight=0.5)
def reduce(f, dice, initial):
    """icepool.reduce(f, dice, initial=initial): map(f, result, die) for each die in turn (StopIteration with
    neither dice nor initial)."""
    return icepool.reduce(f, dice, initial=initial)


@node("accumulate", "value", gen=_gen_reduce("accumulate"), weight=0.4)
def accumulate(f, dice, initial):
    """tuple(icepool.accumulate(f, dice, initial=initial)): every intermediate result of reduce."""
    return tuple(icepool.accumulate(f, dice, initial=initial))


def _gen_from_rv(g, depth):
    r = g.rng
    rv = r.choice(sorted(RVS))
    lo = r.randint(-3, 4)
    hi = lo + r.choice([1, 2, 3, 5, 8]) if r.random() < 0.95 else lo
    n = r.randint(0, 6)
    if rv == "discrete_uniform" or r.random() < 0.5:
        outcomes = sorted(r.sample(range(lo - 2, lo + 10), n))
    else:
        outcomes = sorted({r.randint(4 * lo - 8, 4 * hi + 8) / 4 for _ in range(n)})
        outcomes = [{"float": repr(x)} for x in outcomes]
    denominator = r.choice([1, 6, 10, 100, 1000, 7, 2 ** 70]) if r.random() < 0.95 else r.choice([0, -5])
    return {"op": "from_rv", "rv": {"str": rv}, "params": [lo, hi], "outcomes": outcomes,
            "denominator": denominator}


@node("from_rv", "die", gen=_gen_from_rv, weight=0.4)
def from_rv(rv, params, outcomes, denominator):
    """icepool.from_rv(RVS[rv](*params), outcomes, denominator): harness/catalogue/rvs.py random variables; the
    CDF at the outcomes (continuous: at the midpoints), times denominator, rounded, then from_cumulative."""
    return icepool.from_rv(RVS[rv](*params), outcomes, denominator)


_METHODS = ["expand", "sum", "size", "largest_count", "all_counts", "highest_outcome_and_count"]


def _gen_wallenius(g, depth):
    r = g.rng
    mapping = r.random() < 0.5
    n = r.randint(1, 4)
    outcomes = r.sample(range(1, 9), n) if r.random() < 0.9 else [r.choice("abc") for _ in range(n)]
    data = []
    for o in outcomes:
        o = o if isinstance(o, int) else {"str": o}
        w = r.choice([1, 1, 2, 3, 5]) if r.random() < 0.95 else r.choice([0, -1])
        if mapping and r.random() < 0.4:
            data.append([o, {"tuple": [w, r.choice([0, 1, 2, 3])]}])
        else:
            data.append([o, w])
    hand_size = r.randint(0, 4) if r.random() < 0.95 else -1
    return {"op": "wallenius_deal", "data": data, "mapping": {"bool": mapping}, "hand_size": hand_size,
            "method": {"str": r.choice(_METHODS)}}


@node("wallenius_deal", "die", gen=_gen_wallenius, weight=0.4)
def wallenius_deal(data, mapping, hand_size, method):
    """getattr(icepool.Wallenius(dict(data) if mapping else data).deal(hand_size), method)(): Wallenius'
    noncentral hypergeometric distribution (the whole weight of a card goes away when it is pulled); data are
    (outcome, weight) pairs, or a mapping to weights or (weight, quantity)."""
    return getattr(icepool.Wallenius(dict(data) if mapping else data).deal(hand_size), method)()


@node("wallenius_deal_expr", "mexpr")
def wallenius_deal_expr(data, mapping, hand_size):
    """icepool.Wallenius(dict(data) if mapping else data).deal(hand_size): the hand as a multiset expression (a
    mixture of deals, or Pool([]) for a hand of 0). Kind "mexpr" without a generator of its own, as the
    to_pool.py nodes: `to_pool_evaluate` builds one and evaluates it."""
    return icepool.Wallenius(dict(data) if mapping else data).deal(hand_size)
