"""icepool's collections as outcomes: `Vector` (collection/vector.py, with `VectorWithTruthOnly`), `vectorize`
(expand.py) and `Symbols` (collection/symbols.py).

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.

Dice of vectors and of symbols (`die_vectors`, `die_symbols`, `vectorize`) are ordinary "die" nodes, so every
die node (arithmetic, comparisons, pools, statistics, ...) gets fed them. The value-level nodes work on the kind
"collection": a `Vector` or `Symbols` value, or whatever an operator on one returned (a bool, an int, a
`VectorWithTruthOnly`...). The wire format spells a vector by its elements and a `Symbols` by `str()`, so the
truth value of a vector and the key order and zero counts of a `Symbols` only show through `coll_truth`,
`symbols_method` (`keys`, `values`, `len`) and `repr`.
"""
import operator

import icepool

from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome

BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor",
          "lt", "le", "gt", "ge", "eq", "ne"]
UNARY = ["neg", "pos", "invert", "abs", "trunc", "floor", "ceil", "round"]
VECTOR_METHODS = ["append", "concatenate", "getitem", "slice", "len", "repr", "str", "iter"]
SYMBOLS_METHODS = ["issubset", "issuperset", "isdisjoint", "count_subset", "has_negative_counts", "size",
                   "multiply_counts", "divide_counts", "modulo_counts", "additive_union", "difference",
                   "intersection", "union", "symmetric_difference", "getitem", "getattr", "len", "keys", "values",
                   "repr", "str"]
ALPHABET = "abc"


def _element(g):
    """A vector element: mostly a small int, sometimes another type (which may not compare or add)."""
    if g.rng.random() < 0.8:
        return g.rng.randint(-3, 6)
    return _typed_outcome(g, g.rng.choice(["num", "num", "str", "tuple"]))


def vector_lit(g, n=None):
    n = g.rng.randint(0, 3) if n is None else n
    return {"vector": [_element(g) for _ in range(n)]}


def _symbols_str(g):
    r = g.rng
    if r.random() < 0.03:
        # Illegal symbols or two dashes: ValueError.
        return r.choice(["a-b-c", "a_b", "a b", "a\tb"])
    s = "".join(r.choice(ALPHABET) for _ in range(r.randint(0, 3)))
    if r.random() < 0.3:
        s += r.choice(["-", " - ", "- ", " -"]) + "".join(r.choice(ALPHABET) for _ in range(r.randint(0, 2)))
    return s


def symbols_lit(g):
    return {"symbols": _symbols_str(g)}


def collection_lit(g):
    return vector_lit(g) if g.rng.random() < 0.5 else symbols_lit(g)


def _gen_die_vectors(g, depth):
    r = g.rng
    n = r.randint(1, 3)
    # Mostly one length (different lengths still sort: lexicographically, through VectorWithTruthOnly).
    return {"op": "die_vectors",
            "outcomes": [vector_lit(g, n if r.random() < 0.9 else None) for _ in range(r.randint(1, 4))]}


@node("die_vectors", "die", gen=_gen_die_vectors, leaf=True, weight=0.25)
def die_vectors(outcomes):
    """icepool.Die(list of Vectors): vectors sort as their tuples; mixed element types may raise."""
    return icepool.Die(outcomes)


@node("die_symbols", "die", leaf=True, weight=0.2,
      gen=lambda g, depth: {"op": "die_symbols", "outcomes": [symbols_lit(g) for _ in range(g.rng.randint(1, 4))]})
def die_symbols(outcomes):
    """icepool.Die(list of Symbols): Symbols sort by the count of each character in alphabetical order."""
    return icepool.Die(outcomes)


def _gen_vectorize(g, depth):
    r = g.rng
    args = [g.die(min(depth, 1)) if r.random() < 0.6 else g.outcome() for _ in range(r.randint(1, 3))]
    if not any(isinstance(a, dict) for a in args):
        args[0] = g.die(min(depth, 1))
    # (Reroll among the arguments returns Reroll itself, which other "die" nodes do not take: fixtures only.)
    return {"op": "vectorize", "args": args}


@node("vectorize", "die", gen=_gen_vectorize, weight=0.3)
def vectorize(args):
    """icepool.vectorize(*args): the Cartesian product of dice as a die of Vectors (Reroll among them: Reroll)."""
    return icepool.vectorize(*args)


def _gen_tupleize(g, depth):
    return dict(_gen_vectorize(g, depth), op="tupleize")


@node("tupleize", "die", gen=_gen_tupleize, weight=0.3)
def tupleize(args):
    """icepool.tupleize(*args): the Cartesian product of dice as a die of tuples (Reroll among them: Reroll; no
    die: a tuple)."""
    return icepool.tupleize(*args)


@node("vector", "collection", leaf=True, weight=1.0,
      gen=lambda g, depth: {"op": "vector", "items": vector_lit(g)["vector"]})
def vector(items):
    """icepool.Vector(items)"""
    return icepool.Vector(items)


def _gen_symbols(g, depth):
    r = g.rng.random()
    if r < 0.8:
        return {"op": "symbols", "x": {"str": _symbols_str(g)}}
    if r < 0.9:
        # An iterable of characters (sometimes not single characters, or not strings).
        return {"op": "symbols", "x": {"tuple": [g.rng.choice([{"str": "a"}, {"str": "b"}, {"str": "ab"}, 1])
                                                 for _ in range(g.rng.randint(0, 3))]}}
    return {"op": "symbols", "x": g.make("collection", depth)}


@node("symbols", "collection", gen=_gen_symbols, leaf=True, weight=1.0)
def symbols(x):
    """icepool.Symbols(x): from a string (negatives after a dash), an iterable of characters, or a mapping."""
    return icepool.Symbols(x)


def _operand(g, depth):
    """The other operand: a collection, a scalar, or a string of symbols."""
    r = g.rng.random()
    if r < 0.5:
        return g.make("collection", depth)
    if r < 0.65:
        return {"str": _symbols_str(g)}
    if r < 0.75:
        return collection_lit(g)
    if r < 0.85:
        return g.rng.randint(-2, 3)
    return _typed_outcome(g, "any")


def _gen_binary(g, depth):
    name = g.rng.choice(BINARY)
    a, b = g.make("collection", depth), _operand(g, depth)
    if name in ("pow", "lshift"):
        # A small right operand keeps the numbers small.
        b = g.rng.choice([-1, 0, 1, 2, 3, vector_lit(g)])
    elif g.rng.random() < 0.3:
        a, b = b, a
    return {"op": "coll_binary", "a": a, "b": b, "name": {"str": name}}


@node("coll_binary", "collection", gen=_gen_binary, weight=1.5)
def coll_binary(a, b, name):
    """operator.<name>(a, b) on values: Vector elementwise (comparisons with a truth value), Symbols as a multiset."""
    return getattr(operator, name + "_" if name in ("and", "or") else name)(a, b)


def _gen_unary(g, depth):
    name = g.rng.choice(UNARY)
    out = {"op": "coll_unary", "a": g.make("collection", depth), "name": {"str": name}, "ndigits": None}
    if name == "round":
        out["ndigits"] = g.rng.choice([None, 0, 1, -1])
    return out


@node("coll_unary", "collection", gen=_gen_unary, weight=0.6)
def coll_unary(a, name, ndigits):
    """-a, +a, ~a, abs(a), math.trunc/floor/ceil(a), round(a, ndigits) on a value."""
    import math
    if name == "round":
        return round(a, ndigits)
    f = {"neg": operator.neg, "pos": operator.pos, "invert": operator.invert, "abs": abs, "trunc": math.trunc,
         "floor": math.floor, "ceil": math.ceil}[name]
    return f(a)


@node("coll_truth", "value", weight=0.6, gen=lambda g, depth: {"op": "coll_truth", "x": g.make("collection", depth)})
def coll_truth(x):
    """bool(x): a Vector has one only if it came from a comparison; a Symbols is truthy if it has any key."""
    return bool(x)


def _gen_vector_method(g, depth):
    r = g.rng
    name = r.choice(VECTOR_METHODS)
    v = g.make("collection", depth) if r.random() < 0.3 else {"op": "vector", "items": vector_lit(g)["vector"]}
    if name in ("repr", "str") and r.random() < 0.3:
        # A VectorWithTruthOnly: vectors of different lengths compared.
        a, b = ({"op": "vector", "items": vector_lit(g, n)["vector"]} for n in r.sample([0, 1, 2, 3], 2))
        v = {"op": "coll_binary", "a": a, "b": b, "name": {"str": r.choice(["lt", "le", "gt", "ge"])}}
    arg = None
    if name == "append":
        arg = _element(g)
    elif name == "concatenate":
        arg = r.choice([vector_lit(g), {"tuple": vector_lit(g)["vector"]}, {"str": "ab"}, symbols_lit(g), 3])
    elif name == "getitem":
        arg = r.choice([0, 0, 1, -1, 2, -3, 5, {"bool": True}, {"str": "a"}, None])
    elif name == "slice":
        arg = [r.choice([None, -2, -1, 0, 1, 2, 4]) for _ in range(2)] + [r.choice([None, 1, 2, -1, 0])]
    return {"op": "vector_method", "v": v, "name": {"str": name}, "arg": arg}


@node("vector_method", "collection", gen=_gen_vector_method, weight=0.8)
def vector_method(v, name, arg):
    """A Vector's methods and protocols: append, concatenate, v[i], v[a:b:c], len, repr, str, tuple(v)."""
    if name == "append":
        return v.append(arg)
    if name == "concatenate":
        return v.concatenate(arg)
    if name == "getitem":
        return v[arg]
    if name == "slice":
        return v[slice(*arg)]
    if name == "len":
        return len(v)
    if name == "repr":
        return repr(v)
    if name == "str":
        return str(v)
    if name == "iter":
        return tuple(v)
    raise ValueError(name)


def _gen_symbols_method(g, depth):
    r = g.rng
    name = r.choice(SYMBOLS_METHODS)
    s = g.make("collection", depth) if r.random() < 0.3 else {"op": "symbols", "x": {"str": _symbols_str(g)}}
    arg = None
    if name in ("issubset", "issuperset", "isdisjoint", "symmetric_difference"):
        arg = _operand(g, depth)
    elif name == "count_subset":
        arg = [_operand(g, depth), r.choice([None, None, 0, 7])]
    elif name in ("multiply_counts", "divide_counts", "modulo_counts"):
        arg = r.choice([2, 3, -2, 0, 1, {"bool": True}, {"float": "2.0"}, {"str": "a"}])
    elif name in ("additive_union", "difference", "intersection", "union"):
        arg = [_operand(g, depth) for _ in range(r.randint(0, 2))]
    elif name in ("getitem", "getattr"):
        arg = {"str": r.choice(["a", "b", "c", "ab", "ba", "cab", "", "aa", "d"])}
        if name == "getitem" and r.random() < 0.1:
            arg = r.choice([1, None])
    return {"op": "symbols_method", "s": s, "name": {"str": name}, "arg": arg}


@node("symbols_method", "collection", gen=_gen_symbols_method, weight=0.8)
def symbols_method(s, name, arg):
    """A Symbols' methods and protocols. `keys`/`values` show the dict's order and zero counts; `getattr` is
    `s.<arg>` (forwarded to `s[arg]`)."""
    if name in ("additive_union", "difference", "intersection", "union"):
        return getattr(s, name)(*arg)
    if name == "count_subset":
        return s.count_subset(arg[0], empty_divisor=arg[1])
    if name in ("has_negative_counts", "size"):
        return getattr(s, name)()
    if name == "getitem":
        return s[arg]
    if name == "getattr":
        return getattr(s, arg)
    if name == "len":
        return len(s)
    if name == "keys":
        return tuple(s)
    if name == "values":
        return tuple(s.values())
    if name == "repr":
        return repr(s)
    if name == "str":
        return str(s)
    return getattr(s, name)(arg)
