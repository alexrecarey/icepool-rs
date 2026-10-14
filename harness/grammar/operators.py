"""Arithmetic and comparison operators on dice (Die.binary_operator / unary_operator), beyond basic.py's
add/sub/mul/neg/lt/ge. Python semantics apply outcome by outcome: bool is an int, int / int is a float, tuples and
strs concatenate and repeat, mixed types raise.

Same rules as basic.py: the Python side is a direct call into icepool (an operator on a Die); generators keep
programs small. The operand generators are shared with basic.py's binary nodes.
"""
import math

from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome


def scalar(g):
    """A non-die operand: mostly a small int, sometimes a bool, float, Fraction, str, tuple or None."""
    r = g.rng.random()
    if r < 0.5:
        return g.rng.randint(-3, 6)
    if r < 0.97:
        return _typed_outcome(g, g.rng.choice(["num", "num", "num", "str", "tuple"]))
    return None


def operand(g, depth):
    """The other operand of a binary operator: a die (any generator), an int-kind node, a typed scalar, or now and
    then a Vector or Symbols."""
    from harness.grammar.collection import collection_lit
    r = g.rng.random()
    if r < 0.2:
        return g.make("int", depth)
    if r < 0.33:
        return scalar(g)
    if r < 0.35:
        return collection_lit(g)
    return g.die(depth)


def binary(op):
    """A die and an operand, in either order (so `3 - die` and `die - 3` both occur)."""
    def gen(g, depth):
        a, b = g.die(depth), operand(g, depth)
        if g.rng.random() < 0.5:
            a, b = b, a
        return {"op": op, "a": a, "b": b}
    return gen


def _small(g):
    """A small exponent or shift count: a die or scalar whose magnitude keeps `**` and `<<` cheap."""
    r = g.rng
    k = r.random()
    if k < 0.3:
        return {"op": "d", "sides": r.randint(1, 4)}
    if k < 0.55:
        return {"op": "die", "outcomes": [r.randint(-2, 3) for _ in range(r.randint(1, 3))]}
    if k < 0.75:
        return r.randint(-2, 4)
    if k < 0.9:
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, "num") for _ in range(r.randint(1, 3))]}
    return _typed_outcome(g, "num")


def _small_right(op):
    """Left operand any die (or a scalar when the right one is a die), right operand small."""
    def gen(g, depth):
        b = _small(g)
        if isinstance(b, dict) and "op" in b and g.rng.random() < 0.4:
            a = scalar(g)
        else:
            a = g.die(min(depth, 1))
        return {"op": op, "a": a, "b": b}
    return gen


@node("truediv", "die", gen=binary("truediv"))
def truediv(a, b):
    """a / b (int / int is a float; division by zero raises)"""
    return a / b


@node("floordiv", "die", gen=binary("floordiv"))
def floordiv(a, b):
    """a // b"""
    return a // b


@node("mod", "die", gen=binary("mod"))
def mod(a, b):
    """a % b"""
    return a % b


@node("pow", "die", gen=_small_right("pow"))
def pow_(a, b):
    """a ** b (negative int exponents give floats, negative bases to fractional powers give complex: raises)"""
    return a ** b


@node("lshift", "die", gen=_small_right("lshift"), weight=0.5)
def lshift(a, b):
    """a << b"""
    return a << b


@node("rshift", "die", gen=_small_right("rshift"), weight=0.5)
def rshift(a, b):
    """a >> b"""
    return a >> b


@node("and", "die", gen=binary("and"), weight=0.5)
def and_(a, b):
    """a & b (bool & bool stays bool)"""
    return a & b


@node("or", "die", gen=binary("or"), weight=0.5)
def or_(a, b):
    """a | b"""
    return a | b


@node("xor", "die", gen=binary("xor"), weight=0.5)
def xor(a, b):
    """a ^ b"""
    return a ^ b


@node("le", "die", gen=binary("le"), weight=0.5)
def le(a, b):
    """a <= b, a Die of bools"""
    return a <= b


@node("gt", "die", gen=binary("gt"), weight=0.5)
def gt(a, b):
    """a > b, a Die of bools"""
    return a > b


def _unary(op):
    return lambda g, depth: {"op": op, "a": g.die(depth)}


@node("pos", "die", gen=_unary("pos"), weight=0.3)
def pos(a):
    """+a"""
    return +a


@node("invert", "die", gen=_unary("invert"), weight=0.3)
def invert(a):
    """~a"""
    return ~a


@node("abs", "die", gen=_unary("abs"), weight=0.5)
def abs_(a):
    """abs(a)"""
    return abs(a)


@node("round", "die", weight=0.5,
      gen=lambda g, depth: {"op": "round", "a": g.die(depth),
                            "ndigits": g.rng.choice([None, None, 0, 1, 2, -1, -2])})
def round_(a, ndigits):
    """round(a, ndigits): half to even; ndigits None gives ints"""
    return round(a, ndigits)


@node("trunc", "die", gen=_unary("trunc"), weight=0.3)
def trunc(a):
    """math.trunc(a)"""
    return math.trunc(a)


@node("floor", "die", gen=_unary("floor"), weight=0.3)
def floor(a):
    """math.floor(a)"""
    return math.floor(a)


@node("ceil", "die", gen=_unary("ceil"), weight=0.3)
def ceil(a):
    """math.ceil(a)"""
    return math.ceil(a)
