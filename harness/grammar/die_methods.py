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


def _cumulative_dice(g, n):
    """Dice representing the cumulative distribution at each point: mostly `d(k) >= j` / `coin`-like truth dice,
    sometimes a plain die (its outcomes != False count), an int or bool after the first (made into a die), or no
    die first (then the ints path)."""
    out = []
    for _ in range(n):
        r = g.rng.random()
        if r < 0.6:
            out.append({"op": "ge", "a": {"op": "d", "sides": g.rng.randint(1, 4)}, "b": g.rng.randint(0, 4)})
        elif r < 0.8:
            out.append({"op": "d", "sides": g.rng.randint(1, 3)})
        elif r < 0.9:
            out.append({"op": "die", "outcomes": [g.rng.choice([0, 1, {"bool": False}, {"bool": True}, {"str": "a"}])
                                                  for _ in range(g.rng.randint(1, 3))]})
        else:
            out.append(g.rng.choice([0, 1, 2, {"bool": True}]))
    return out


def _gen_from_cumulative(g, depth):
    n = g.rng.randint(0, 5)
    if g.rng.random() < 0.8:
        outcomes = sorted(g.rng.sample(range(-4, 13), n))
    else:
        # Unsorted, repeated (a dict keeps the last difference), typed, or a die (mixed in by Die()).
        outcomes = [g.outcome() if g.rng.random() < 0.7 else
                    (g.die(0) if g.rng.random() < 0.3 else _typed_outcome(g, "num")) for _ in range(n)]
    cumulative = _cumulative_dice(g, n) if g.rng.random() < 0.25 else _cumulative(g, n)
    return {"op": "from_cumulative", "outcomes": outcomes, "cumulative": cumulative,
            "reverse": {"bool": g.rng.random() < 0.4}}


@node("from_cumulative", "die", gen=_gen_from_cumulative, weight=0.5)
def from_cumulative(outcomes, cumulative, reverse):
    """icepool.from_cumulative(outcomes, cumulative, reverse=reverse) with int cumulative quantities or dice (the
    quantity of each harmonized die != False)."""
    return icepool.from_cumulative(outcomes, cumulative, reverse=reverse)


DIE_METHODS = ["getitem", "contains", "len", "iter", "keys", "values", "items", "keys_index", "values_index",
               "items_index", "keys_eq", "values_eq", "items_eq", "min_outcome", "max_outcome", "is_empty"]


def _view_arg(g, name, die):
    r = g.rng
    if name in ("getitem", "contains"):
        x = r.random()
        if x < 0.85:
            return _typed_outcome(g, "num") if r.random() < 0.3 else g.outcome()
        if x < 0.95:
            return r.choice([{"str": "a"}, {"tuple": [1, 2]}, None])
        return []  # unhashable
    if name.endswith("_index"):
        if r.random() < 0.7:
            return r.choice([0, 0, 1, -1, 2, -3, 7, {"bool": True}, {"str": "a"}, None])
        return [r.choice([None, -2, -1, 0, 1, 3]) for _ in range(2)] + [r.choice([None, 1, 2, -1, 0])]
    if name.endswith("_eq"):
        x = r.random()
        if x < 0.5:
            # The die's own view (or another die's) as a tuple.
            other = die if r.random() < 0.7 else g.die(0)
            return {"op": "die_method", "die": other, "name": {"str": name[:-3]}, "arg": None}
        if x < 0.8:
            return {"tuple": [g.outcome() for _ in range(r.randint(0, 3))]}
        return [1, 2] if r.random() < 0.5 else 3
    return None


def _gen_die_method(g, depth):
    from harness.grammar.pool import _die_without_nan
    name = g.rng.choice(DIE_METHODS)
    # No NaN outcomes: dict lookups and tuple == compare NaN by identity first.
    die = _die_without_nan(g, depth)
    return {"op": "die_method", "die": die, "name": {"str": name}, "arg": _view_arg(g, name, die)}


@node("die_method", "value", gen=_gen_die_method, weight=0.5)
def die_method(die, name, arg):
    """Die's mapping protocol and its Counts views (CountsKeysView / CountsValuesView / CountsItemsView, both views
    and sequences): die[arg] (KeyError if absent), arg in die, len(die), tuple(die), tuple(die.keys() / values() /
    items()), view[arg] (an int, or slice(*arg) for a list), view == arg, min_outcome(), max_outcome(),
    is_empty()."""
    match name:
        case "getitem":
            return die[arg]
        case "contains":
            return arg in die
        case "len":
            return len(die)
        case "iter":
            return tuple(die)
        case "keys" | "values" | "items":
            return tuple(getattr(die, name)())
        case "keys_index" | "values_index" | "items_index":
            return getattr(die, name[:-6])()[slice(*arg) if isinstance(arg, list) else arg]
        case "keys_eq" | "values_eq" | "items_eq":
            return getattr(die, name[:-3])() == arg
        case "min_outcome" | "max_outcome" | "is_empty":
            return getattr(die, name)()
    raise ValueError(name)
