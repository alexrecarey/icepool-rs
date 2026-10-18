"""Population statistics and probabilities (population/base.py): quantities / probabilities with comparators,
nearest, quantiles, the scalar and joint statistics, marginals, group_by, and quantity management.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small. Results that
icepool returns as views or dicts are made tuples here so they encode (`quantities('==')` is a values view,
`group_by` a dict).
"""
import icepool

from harness.catalogue.functions import fn_node
from harness.core.grammar_api import node
from harness.grammar import peek
from harness.grammar.construction import _typed_outcome
from harness.grammar.pool import _die_without_nan, _has_nan

COMPARISONS = ["==", "!=", "<=", "<", ">=", ">"]

# Methods taking no arguments, as getattr names (the program layer knows exactly these).
STATS = ["mean", "variance", "standard_deviation", "sd", "skewness", "excess_kurtosis", "mode", "modal_quantity",
         "median", "median_low", "median_high", "min_outcome", "max_outcome", "denominator", "is_empty"]
QUANTILES = ["quantile", "quantile_low", "quantile_high"]
JOINT = ["covariance", "correlation"]
QUANTITY_OPS = ["multiply_quantities", "divide_quantities", "modulo_quantities"]
DISTANCES = ["kolmogorov_smirnov", "cramer_von_mises"]


def _comparison(g, none_ok=True):
    r = g.rng.random()
    if none_ok and r < 0.1:
        return None
    if r < 0.15:
        return {"str": g.rng.choice(["=", "<>", "=<"])}  # invalid: ValueError
    return {"str": g.rng.choice(COMPARISONS)}


def _query(g):
    """An outcome to compare against: mostly a small int, sometimes another type (which may not compare)."""
    return g.outcome() if g.rng.random() < 0.85 else _typed_outcome(g, "any")


def _stat_die(g, depth):
    """Mostly plain dice; sometimes typed ones (floats, Fractions, strs, tuples) or a mixture with uneven weights."""
    r = g.rng.random()
    if r < 0.7:
        return g.die(depth)
    if r < 0.85:
        kind = g.rng.choice(["num", "num", "num", "str", "tuple"])
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, kind) for _ in range(g.rng.randint(1, 5))]}
    return {"op": "die_times", "outcomes": [{"op": "weights", "items": [[g.outcome(), g.rng.choice([0, 1, 2])]
                                                                         for _ in range(g.rng.randint(1, 3))]},
                                            g.outcome()],
            "times": [1, g.rng.choice([0, 1])]}


def _stat_die_without_nan(g, depth):
    """`_stat_die` without NaN outcomes, for kolmogorov_smirnov / cramer_von_mises: `sorted_union` sorts a set,
    and where NaNs land in it depends on their hashes (object identity). Asks the oracle, like pool.py."""
    for _ in range(10):
        p = _stat_die(g, depth)
        try:
            die = peek(p)
            if isinstance(die, icepool.Die) and any(_has_nan(o) for o in die.outcomes()):
                continue
        except Exception:  # noqa: BLE001 - a die that raises is a fine program too
            pass
        return p
    return {"op": "d", "sides": g.rng.randint(1, 6)}


def _tuple_die(g, depth):
    """A die of tuples (dice inside a tuple are tupleized), sometimes of strs or plain ints.

    The dice inside have no NaN outcomes: tuples holding the *same* NaN object compare equal in Python (identity),
    so their marginals merge where the port's never do (dev/NOTES.md, Gotchas)."""
    r = g.rng.random()
    if r < 0.75:
        items = [_die_without_nan(g, min(depth, 1)) if g.rng.random() < 0.7 else _query(g)
                 for _ in range(g.rng.randint(1, 3))]
        return {"op": "die", "outcomes": [{"op": "tuple", "items": items}]}
    if r < 0.85:
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, "tuple") for _ in range(g.rng.randint(1, 4))]}
    if r < 0.92:
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, "str") for _ in range(g.rng.randint(1, 4))]}
    return _stat_die(g, depth)


def _index(g):
    r = g.rng.random()
    if r < 0.85:
        return g.rng.choice([0, 0, 0, 1, 1, -1, -1, 2, -2, 3])
    return g.rng.choice([{"bool": True}, {"str": "x"}, None, {"float": "0.0"}])


def _slice_bound(g):
    return None if g.rng.random() < 0.4 else g.rng.choice([-4, -2, -1, 0, 1, 2, 3, {"bool": True}])


def _percent(g):
    return {"bool": g.rng.random() < 0.3}


@node("quantities", "value", weight=0.5,
      gen=lambda g, depth: {"op": "quantities", "die": _stat_die(g, depth), "comparison": _comparison(g)})
def quantities(die, comparison):
    """tuple(die.quantities(comparison)): None is '=='; '<=' is the CDF ('>=' of an empty die is (0,))."""
    return tuple(die.quantities(comparison))


@node("probabilities", "value", weight=0.5,
      gen=lambda g, depth: {"op": "probabilities", "die": _stat_die(g, depth), "comparison": _comparison(g),
                            "percent": _percent(g)})
def probabilities(die, comparison, percent):
    """die.probabilities(comparison, percent=percent): Fractions, or floats 100.0 * x."""
    return die.probabilities(comparison, percent=percent)


def _query_args(g):
    r = g.rng.random()
    if r < 0.2:
        return [_query(g)]
    if r < 0.25:
        return [_comparison(g, none_ok=False)]  # a lone comparison is an outcome to look up
    return [_comparison(g), _query(g)]


@node("quantity", "value", weight=0.5,
      gen=lambda g, depth: {"op": "quantity", "die": _stat_die(g, depth), "args": _query_args(g)})
def quantity(die, args):
    """die.quantity(*args): (outcome) or (comparison, outcome); an outcome of None means the one-argument form."""
    return die.quantity(*args)


@node("probability", "value", weight=0.5,
      gen=lambda g, depth: {"op": "probability", "die": _stat_die(g, depth), "args": _query_args(g),
                            "percent": _percent(g)})
def probability(die, args, percent):
    """die.probability(*args, percent=percent)"""
    return die.probability(*args, percent=percent)


@node("nearest", "value", weight=0.4,
      gen=lambda g, depth: {"op": "nearest", "die": _stat_die(g, depth),
                            "comparison": {"str": g.rng.choice(["<=", "<", ">=", ">", ">", "<", "=="])},
                            "outcome": _query(g)})
def nearest(die, comparison, outcome):
    """die.nearest(comparison, outcome): the nearest outcome fitting it (the argument itself if it is in the die
    and the comparison allows equality), or None."""
    return die.nearest(comparison, outcome)


@node("die_stat", "value", weight=1.0,
      gen=lambda g, depth: {"op": "die_stat", "die": _stat_die(g, depth), "stat": {"str": g.rng.choice(STATS)}})
def die_stat(die, stat):
    """getattr(die, stat)() for stat in mean, variance, standard_deviation, sd, skewness, excess_kurtosis, mode,
    modal_quantity, median(_low/_high), min_outcome, max_outcome, denominator, is_empty."""
    return getattr(die, stat)()


def _base(g):
    r = g.rng.random()
    if r < 0.4:
        return []
    if r < 0.7:
        return [g.rng.choice([2, 3, 10, {"float": "2.0"}, {"float": "10.0"}, {"float": "0.5"}])]
    return [g.rng.choice([1, 0, -2, {"bool": True}, {"frac": [3, 2]}, {"float": "inf"}, {"str": "e"},
                          2 ** 1100, {"float": "1e-300"}])]


@node("entropy", "value", weight=0.5,
      gen=lambda g, depth: {"op": "entropy", "die": _stat_die(g, depth), "base": _base(g)})
def entropy(die, base):
    """die.entropy(*base): -sum(p * math.log(p, base)) in bits by default (base 2.0)."""
    return die.entropy(*base)


def _quantile_args(g):
    r = g.rng.random()
    if r < 0.2:
        return [g.rng.randint(0, 100)]
    n, d = g.rng.randint(-1, 7), g.rng.choice([1, 2, 3, 4, 6, 100, 0 if g.rng.random() < 0.2 else 5])
    if g.rng.random() < 0.1:
        n = g.rng.choice([{"float": "0.5"}, {"frac": [1, 3]}, {"bool": True}])
    return [n, d]


@node("die_quantile", "value", weight=0.5,
      gen=lambda g, depth: {"op": "die_quantile", "die": _stat_die(g, depth),
                            "method": {"str": g.rng.choice(QUANTILES)}, "args": _quantile_args(g)})
def die_quantile(die, method, args):
    """getattr(die, method)(*args) for quantile / quantile_low / quantile_high: (n) or (n, d), d defaulting to
    100."""
    return getattr(die, method)(*args)


@node("cdf_distance", "value", weight=0.4,
      gen=lambda g, depth: {"op": "cdf_distance", "a": _stat_die_without_nan(g, depth),
                            "b": _stat_die_without_nan(g, depth),
                            "stat": {"str": g.rng.choice(DISTANCES)}})
def cdf_distance(a, b, stat):
    """getattr(a, stat)(b) for kolmogorov_smirnov (max |CDF difference|) and cramer_von_mises (sum of squares)."""
    return getattr(a, stat)(b)


@node("marginal", "die", weight=0.5,
      gen=lambda g, depth: {"op": "marginal", "die": _tuple_die(g, depth), "index": _index(g)})
def marginal(die, index):
    """die.marginals[index]: the [] operator applied to each outcome."""
    return die.marginals[index]


@node("marginal_slice", "die", weight=0.3,
      gen=lambda g, depth: {"op": "marginal_slice", "die": _tuple_die(g, depth), "start": _slice_bound(g),
                            "stop": _slice_bound(g), "step": g.rng.choice([None, None, 1, 2, -1, -2, 0])})
def marginal_slice(die, start, stop, step):
    """die.marginals[start:stop:step]"""
    return die.marginals[start:stop:step]


@node("marginals", "value", weight=0.3,
      gen=lambda g, depth: {"op": "marginals", "die": _tuple_die(g, depth)})
def marginals(die):
    """tuple(die.marginals): one marginal die per position, up to the shortest outcome's len()."""
    return tuple(die.marginals)


def _any_unary(g):
    """A one-argument catalogue callable (a predicate or a unary map), or a mapping."""
    if g.rng.random() < 0.15:
        return fn_node(g, name="mapping")[0]
    return fn_node(g, pred=True)[0]


def _gen_group_by(g, depth):
    if g.rng.random() < 0.25:
        # A callable of several parameters over a die of tuples: star inferred (or given), each tuple unpacked.
        name = g.rng.choice(["swap", "add2", "max2", "builtin_max", "first"])
        return {"op": "group_by", "die": _tuple_die(g, depth), "key": fn_node(g, name=name)[0],
                "star": g.rng.choice([None] * 4 + [True, False])}
    return {"op": "group_by", "die": _stat_die(g, depth), "key": _any_unary(g),
            "star": g.rng.choice([None] * 8 + [True, False])}


@node("group_by", "value", weight=0.4, gen=_gen_group_by)
def group_by(die, key, star):
    """tuple(die.group_by(key, star=star).items()): (key, die) per distinct key in first-seen order; a mapping
    leaves outcomes it lacks as their own key."""
    return tuple(die.group_by(key, star=star).items())


@node("group_by_index", "value", weight=0.3,
      gen=lambda g, depth: {"op": "group_by_index", "die": _tuple_die(g, depth), "index": _index(g)})
def group_by_index(die, index):
    """tuple(die.group_by[index].items()): grouped by outcome[index]."""
    return tuple(die.group_by[index].items())


def _attr_die(g, depth):
    """A die of numbers (ints, floats, Fractions, bools; their numeric attributes differ), sometimes strs, tuples
    or Symbols (whose `__getattr__` is `s[name]`)."""
    r = g.rng.random()
    if r < 0.4:
        return g.die(depth)
    if r < 0.8:
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, "num") for _ in range(g.rng.randint(1, 4))]}
    if r < 0.9:
        return _tuple_die(g, depth)
    return {"op": "die", "outcomes": [{"symbols": "".join(g.rng.choice("aimr") for _ in range(g.rng.randint(0, 3)))}
                                      for _ in range(g.rng.randint(1, 3))]}


def _attr_name(g):
    """The numeric tower's data attributes; rarely a private name (AttributeError before any outcome is looked
    at), an empty one (IndexError) or a non-str (TypeError)."""
    r = g.rng.random()
    if r < 0.85:
        return {"str": g.rng.choice(["real", "imag", "numerator", "denominator"])}
    if r < 0.95:
        return {"str": g.rng.choice(["_x", "__x__", "_real"])}
    return g.rng.choice([{"str": ""}, 1, None])


@node("marginal_attr", "die", weight=0.3,
      gen=lambda g, depth: {"op": "marginal_attr", "die": _attr_die(g, depth), "name": _attr_name(g)})
def marginal_attr(die, name):
    """getattr(die.marginals, name): getattr(outcome, name) applied to each outcome (a name starting with '_'
    raises AttributeError)."""
    return getattr(die.marginals, name)


@node("group_by_attr", "value", weight=0.3,
      gen=lambda g, depth: {"op": "group_by_attr", "die": _attr_die(g, depth), "name": _attr_name(g)})
def group_by_attr(die, name):
    """tuple(getattr(die.group_by, name).items()): grouped by getattr(outcome, name) (a name starting with '_'
    raises AttributeError)."""
    return tuple(getattr(die.group_by, name).items())


def _ij(g):
    return g.rng.choice([0, 0, 1, 1, -1, 2])


@node("joint_stat", "value", weight=0.4,
      gen=lambda g, depth: {"op": "joint_stat", "die": _tuple_die(g, depth), "stat": {"str": g.rng.choice(JOINT)},
                            "i": _ij(g), "j": _ij(g)})
def joint_stat(die, stat, i, j):
    """getattr(die, stat)(i, j) for covariance and correlation of tuple outcomes."""
    return getattr(die, stat)(i, j)


def _quantity_arg(g):
    r = g.rng.random()
    if r < 0.8:
        return g.rng.choice([0, 1, 1, 2, 3, 4, 6])
    return g.rng.choice([-1, {"bool": True}, {"float": "2.0"}, {"float": "0.0"}, {"frac": [1, 2]}, {"str": "x"}])


@node("quantities_op", "die", weight=0.4,
      gen=lambda g, depth: {"op": "quantities_op", "die": _stat_die(g, depth),
                            "method": {"str": g.rng.choice(QUANTITY_OPS)}, "x": _quantity_arg(g)})
def quantities_op(die, method, x):
    """getattr(die, method)(x) for multiply_quantities (scale 1 returns die), divide_quantities (rounding down,
    zeros dropped; divisor 0 returns die) and modulo_quantities."""
    return getattr(die, method)(x)


def _target_denominator(g, die, op):
    """Mostly the die's own denominator combined with a small int (so the target is often reachable), else a
    constant."""
    if g.rng.random() < 0.6:
        own = {"op": "die_stat", "die": die, "stat": {"str": "denominator"}}
        k = g.rng.choice([0, 1, 2, 3, -1]) if op == "add" else g.rng.choice([1, 2, 3, 0, -1])
        return {"op": op, "a": own, "b": k}
    return g.rng.choice([0, 1, 2, 4, 6, 8, 12, 36, -6]) if g.rng.random() < 0.9 else _quantity_arg(g)


def _gen_pad(g, depth):
    die = _stat_die(g, depth)
    return {"op": "pad_to_denominator", "die": die, "denominator": _target_denominator(g, die, "add"),
            "outcome": _query(g)}


def _gen_multiply_to(g, depth):
    die = _stat_die(g, depth)
    return {"op": "multiply_to_denominator", "die": die, "denominator": _target_denominator(g, die, "mul")}


@node("pad_to_denominator", "die", weight=0.3, gen=_gen_pad)
def pad_to_denominator(die, denominator, outcome):
    """die.pad_to_denominator(denominator, outcome): the outcome's quantity adjusted to reach the denominator."""
    return die.pad_to_denominator(denominator, outcome=outcome)


@node("multiply_to_denominator", "die", weight=0.3, gen=_gen_multiply_to)
def multiply_to_denominator(die, denominator):
    """die.multiply_to_denominator(denominator): ValueError unless the denominator divides it."""
    return die.multiply_to_denominator(denominator)


@node("quantity_where", "value", weight=0.3,
      gen=lambda g, depth: {"op": "quantity_where", "die": _stat_die(g, depth), "which": fn_node(g, pred=True)[0],
                            "star": g.rng.choice([None] * 8 + [True, False])})
def quantity_where(die, which, star):
    """die.quantity_where(which, star=star)"""
    return die.quantity_where(which, star=star)


@node("probability_where", "value", weight=0.3,
      gen=lambda g, depth: {"op": "probability_where", "die": _stat_die(g, depth),
                            "which": fn_node(g, pred=True)[0], "star": g.rng.choice([None] * 8 + [True, False]),
                            "percent": _percent(g)})
def probability_where(die, which, star, percent):
    """die.probability_where(which, star=star, percent=percent)"""
    return die.probability_where(which, star=star, percent=percent)
