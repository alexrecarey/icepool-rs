"""Formatting, sampling and equality: population/format.py (`format(die, spec)`, `str(die)`,
`format_probability_inverse`, `common_outcome_length`), `Population.sample` (seeded), and `Die.__eq__` /
`__ne__` (`DieWithTruth`), `bool()` of those, `Die.equals` and `hash_key` comparisons.

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.

`eq` / `ne` return the kind "die_truth": a `DieWithTruth` behaves as a die everywhere, but its type name shows in
`format` and it has a truth value, so it only goes where those are what is tested (`format`, `truth`, `equals`,
`hash_key_compare`), never into the "die" nodes.
"""
import random

import icepool

from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome
from harness.grammar.pool import _die_without_nan

OUTPUTS = ["md", "md", "bbcode", "csv", "html"]
TOKENS = ["o", "*o", "q==", "q<=", "q>=", "p==", "p<=", "p>=", "%==", "%<=", "%>=", "i==", "i<=", "i>="]
# Strs that exercise CSV quoting, HTML escaping, the numeric-alignment regex and repr inside tuples.
SPECIAL_STRS = ["", "a", "a,b", 'say "hi"', "it's", "'\"", "1", "-2.5", "3%", "12\n", "x\ny", "<b>&", " ", "0.",
                "-", "\\", "\t", "\r", "1.5%\n"]


def _spec(g):
    """A format spec: mostly an output format and columns (sometimes `|`-separated); sometimes empty, invalid, or
    with too many colons."""
    r = g.rng
    x = r.random()
    if x < 0.12:
        return {"str": ""}
    if x < 0.2:
        return {"str": r.choice(OUTPUTS + ["txt"])}
    tokens = [r.choice(TOKENS) for _ in range(r.randint(0, 4))]
    if r.random() < 0.08:
        tokens.insert(r.randint(0, len(tokens)), r.choice(["x", "q<", "%", "*", "oo", "q!="]))
    sep = "|" if r.random() < 0.3 else ""
    spec = r.choice(OUTPUTS + (["txt"] if r.random() < 0.05 else [])) + ":" + sep.join(tokens)
    if r.random() < 0.04:
        spec += ":o"
    return {"str": spec}


def _special_die(g):
    r = g.rng
    kind = r.choice(["str", "str", "tuple", "tuple_str", "num"])
    if kind == "str":
        outcomes = [{"str": r.choice(SPECIAL_STRS)} for _ in range(r.randint(1, 4))]
    elif kind == "tuple_str":
        n = r.randint(0, 2)
        outcomes = [{"tuple": [{"str": r.choice(SPECIAL_STRS)} for _ in range(n)]} for _ in range(r.randint(1, 3))]
    else:
        outcomes = [_typed_outcome(g, kind) for _ in range(r.randint(1, 4))]
    return {"op": "die_typed", "outcomes": outcomes}


def _format_die(g, depth):
    """Mostly ordinary dice; also typed dice (strs that need quoting or escaping, tuples), an empty die, huge or
    tiny quantities (the default columns drop quantities from 10**30; tiny probabilities print as 1.00e+09), and
    `==` results (a `DieWithTruth`)."""
    r = g.rng
    x = r.random()
    if x < 0.45:
        return g.die(depth)
    if x < 0.7:
        return _special_die(g)
    if x < 0.75:
        return {"op": "die", "outcomes": []}
    if x < 0.85:
        return {"op": "weights", "items": [[g.outcome(), r.choice([1, 2, 10**9, 10**29, 10**30, 3 * 10**40])]
                                           for _ in range(r.randint(1, 3))]}
    return _gen_eq(g, depth, r.choice(["eq", "ne"]), die_first=True)


@node("format", "value", weight=0.6,
      gen=lambda g, depth: {"op": "format", "die": _format_die(g, depth),
                            "spec": None if g.rng.random() < 0.15 else _spec(g)})
def format_(die, spec):
    """format(die, spec): a Markdown/BBCode/CSV/HTML table; a None spec is str(die)."""
    if spec is None:
        return str(die)
    return format(die, spec)


def _probability(g):
    r = g.rng
    x = r.random()
    if x < 0.4:
        return {"frac": [r.randint(0, 3), r.choice([1, 2, 3, 6, 7, 20, 100, 10**4, 10**8, 3 * 10**9])]}
    if x < 0.55:
        return {"frac": [1, r.choice([10**30, 10**400])]}  # float(p) is tiny or 0.0 (then 1.0 / p raises)
    if x < 0.75:
        return {"float": repr(r.choice([0.5, 0.05, 1.0, 1e-7, 1e-300, 5e-324, 0.0, -0.1, 1.5, 0.3, 1 / 3]))}
    if x < 0.9:
        return r.choice([0, 1, 2, -1, {"bool": True}, {"bool": False}])
    return _typed_outcome(g, "any")


def _int_start(g):
    r = g.rng
    if r.random() < 0.5:
        return []
    return [r.choice([20, 20, 1, 2, 10, 100, 7, 10**20, 0, -5, {"bool": True}, {"float": "0.5"}, {"float": "0.01"},
                      {"float": "1000.0000000000001"}, {"frac": [5, 2]}, {"float": "inf"}, {"str": "20"}])]


@node("format_probability_inverse", "value", weight=0.4,
      gen=lambda g, depth: {"op": "format_probability_inverse", "probability": _probability(g),
                            "args": _int_start(g)})
def format_probability_inverse(probability, args):
    """icepool.format_probability_inverse(probability, *args): "1 in N" ('n/a' outside (0, 1])."""
    return icepool.format_probability_inverse(probability, *args)


@node("common_outcome_length", "value", weight=0.3,
      gen=lambda g, depth: {"op": "common_outcome_length",
                            "die": _special_die(g) if g.rng.random() < 0.7 else g.die(depth)})
def common_outcome_length(die):
    """die.common_outcome_length(): the common len() of all outcomes, or None."""
    return die.common_outcome_length()


def _seed(g):
    r = g.rng
    return r.choice([r.randint(-1000, 1000), r.randint(0, 2**70), -(2**40), {"bool": True}, 0])


@node("sample", "value", weight=0.4,
      gen=lambda g, depth: {"op": "sample", "die": g.die(depth) if g.rng.random() < 0.8 else _format_die(g, depth),
                            "seed": _seed(g), "n": g.rng.randint(0, 4)})
def sample(die, seed, n):
    """random.seed(seed); tuple(die.sample() for _ in range(n)): Python's Mersenne Twister, randrange(denominator)."""
    random.seed(seed)
    return tuple(die.sample() for _ in range(n))


def _operand(g, depth):
    """A die (without NaN, whose identity matters to tuple equality) or sometimes a plain value."""
    r = g.rng.random()
    if r < 0.75:
        return _die_without_nan(g, depth)
    if r < 0.85:
        return g.outcome()
    return _typed_outcome(g, "any")


def _gen_eq(g, depth, op, die_first=False):
    """`a == b`; with `die_first`, `a` is a die (so the result is a `DieWithTruth`, not a bool)."""
    a, b = _die_without_nan(g, depth) if die_first else _operand(g, depth), _operand(g, depth)
    if g.rng.random() < 0.2:
        b = a  # the same program twice: equal dice (truth value True for ==)
    return {"op": op, "a": a, "b": b}


@node("eq", "die_truth", gen=lambda g, depth: _gen_eq(g, depth, "eq"))
def eq(a, b):
    """a == b: with a die on either side a DieWithTruth (data: outcome-wise ==; truth: equals()), else a bool."""
    return a == b


@node("ne", "die_truth", gen=lambda g, depth: _gen_eq(g, depth, "ne"))
def ne(a, b):
    """a != b: with a die on either side a DieWithTruth (data: outcome-wise !=; truth: not equals()), else a bool."""
    return a != b


@node("truth", "value", weight=0.6,
      gen=lambda g, depth: {"op": "truth", "x": g.make("die_truth", depth) if g.rng.random() < 0.9
                            else g.die(depth)})
def truth(x):
    """bool(x): the truth value of a DieWithTruth (a plain die raises TypeError)."""
    return bool(x)


def _die_or_truth(g, depth):
    return g.make("die_truth", depth) if g.rng.random() < 0.15 else _die_without_nan(g, depth)


def _equal_ish(g, depth):
    """Two dice, often the same program, or the same outcomes with scaled quantities (equal after simplify)."""
    a = _die_or_truth(g, depth)
    x = g.rng.random()
    if x < 0.3:
        b = a
    elif x < 0.5:
        b = {"op": "mixture", "outcomes": [a], "times": [g.rng.choice([2, 3])]}
    elif x < 0.6:
        b = g.outcome()
    else:
        b = _die_or_truth(g, depth)
    return a, b


def _gen_equals(g, depth):
    a, b = _equal_ish(g, depth)
    return {"op": "equals", "a": a, "b": b, "simplify": {"bool": g.rng.random() < 0.5}}


@node("equals", "value", weight=0.5, gen=_gen_equals)
def equals(a, b, simplify):
    """a.equals(b, simplify=simplify): same outcomes and quantities (False if b is not a die)."""
    return a.equals(b, simplify=simplify)


def _gen_equals_self(g, depth):
    r = g.rng.random()
    kind = ("die" if r < 0.3 else "die_truth" if r < 0.4 else "deck" if r < 0.5 else "pool" if r < 0.65 else
            "mexpr" if r < 0.8 else "deal" if r < 0.85 else "multi_deal" if r < 0.9 else None)
    if kind is None:
        x = g.make("again_expr", depth) if g.rng.random() < 0.5 else g.outcome()
    else:
        x = g.make(kind, depth)
    return {"op": "equals_self", "x": x}


@node("equals_self", "value", weight=0.3, gen=_gen_equals_self)
def equals_self(x):
    """x.equals(x) on the same object: True for a die (Die.equals) and for a deck, a multiset expression or
    generator, or an Again expression (MaybeHashKeyed.equals), both of which check `self is other` first; anything
    else has no equals (AttributeError)."""
    return x.equals(x)


def _gen_hash_key_compare(g, depth):
    a, b = _equal_ish(g, depth)
    if not isinstance(b, dict):
        b = {"op": "d", "sides": g.rng.randint(1, 4)}
    return {"op": "hash_key_compare", "a": a, "b": b, "cmp": {"str": g.rng.choice(["<", "<=", ">", ">=", "==",
                                                                                   "!="])}}


@node("hash_key_compare", "value", weight=0.4, gen=_gen_hash_key_compare)
def hash_key_compare(a, b, cmp):
    """a.hash_key <cmp> b.hash_key: (Die, tuple(items)) compared as tuples (outcomes first, then quantities)."""
    match cmp:
        case "<":
            return a.hash_key < b.hash_key
        case "<=":
            return a.hash_key <= b.hash_key
        case ">":
            return a.hash_key > b.hash_key
        case ">=":
            return a.hash_key >= b.hash_key
        case "==":
            return a.hash_key == b.hash_key
        case "!=":
            return a.hash_key != b.hash_key

