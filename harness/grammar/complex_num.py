"""Python's complex numbers as outcomes and values (task V1). They arise from `negative ** fractional` (and then
from arithmetic on the result, e.g. `die.standardized_moment(0.5)`); CPython 3.13's own arithmetic decides every
bit: `_Py_c_quot`, `c_powi` / `_Py_c_pow`, `abs` (hypot), and `repr`. A die with a complex outcome raises when it
sorts, so these nodes work on plain values.

The Python side is `complex(re, im)` and `operator.<name>` on values, as collection.py's coll_binary/coll_unary are.
"""
import math
import operator

from harness.core.grammar_api import node
from harness.grammar.collection import BINARY, UNARY
from harness.grammar.construction import _typed_outcome

# Real and imaginary parts: small ints and halves (exact products, so most results are exact), signed zeros,
# infinities and NaN (the C-level endcases), and magnitudes where products and hypot overflow or underflow.
_PARTS = [0.0, -0.0, 1.0, -1.0, 2.0, -3.0, 0.5, -2.5, 0.1, 1e16, 1e-5, 1e200, -1e300, 1e-200, 5e-324, 1.5e308,
          math.inf, -math.inf, math.nan]


def _part(g):
    r = g.rng
    k = r.random()
    if k < 0.45:
        return r.randint(-6, 6) if r.random() < 0.3 else {"float": repr(r.randint(-12, 12) / 2)}
    if k < 0.7:
        # Arbitrary doubles: products, quotients and powers round, and must round as CPython's do.
        return {"float": repr(r.uniform(-10, 10) * 10.0 ** r.randint(-3, 3))}
    return {"float": repr(r.choice(_PARTS))}


def complex_lit(g):
    return {"op": "complex", "re": _part(g), "im": _part(g)}


@node("complex", "complex", leaf=True, weight=0.3, gen=lambda g, depth: complex_lit(g))
def complex_(re, im):
    """complex(re, im) from two real numbers (an int too large for a float raises OverflowError)."""
    return complex(re, im)


def _exponent(g):
    """`**`'s right operand: small integral exponents take c_powi (|n| <= 100), others the polar form."""
    r = g.rng
    return r.choice([0, 1, 2, 3, -1, -2, 5, 100, -100, 101, {"float": "0.5"}, {"float": "-1.5"}, {"float": "2.0"},
                     {"frac": [1, 3]}, {"frac": [4, 1]}, {"bool": True}, {"float": "1e+300"}, complex_lit(g)])


def _operand(g, depth):
    """A complex (literal, or computed when depth allows), or a real number, or now and then a str or tuple."""
    r = g.rng.random()
    if r < 0.45:
        return complex_lit(g)
    if r < 0.6 and depth > 0:
        return g.make("complex", depth)
    if r < 0.9:
        return _typed_outcome(g, "num")
    return _typed_outcome(g, g.rng.choice(["str", "tuple"]))


def _gen_binary(g, depth):
    # Mostly the operators complex has; the rest (and comparisons) raise TypeError.
    name = g.rng.choice(BINARY + ["add", "sub", "mul", "truediv", "pow"] * 4)
    a, b = complex_lit(g) if g.rng.random() < 0.7 else g.make("complex", depth), _operand(g, depth)
    if name == "pow" and g.rng.random() < 0.7:
        b = _exponent(g)
    if g.rng.random() < 0.4:
        a, b = b, a
    return {"op": "cx_binary", "a": a, "b": b, "name": {"str": name}}


@node("cx_binary", "complex", gen=_gen_binary, weight=1.0)
def cx_binary(a, b, name):
    """operator.<name>(a, b) with a complex operand: + - * / ** in complex arithmetic (a real operand becomes
    complex(float(x))), == and != by value, everything else TypeError."""
    return getattr(operator, name + "_" if name in ("and", "or") else name)(a, b)


def _gen_unary(g, depth):
    name = g.rng.choice(UNARY + ["neg", "abs", "abs"])
    a = complex_lit(g) if g.rng.random() < 0.7 else g.make("complex", depth)
    return {"op": "cx_unary", "a": a, "name": {"str": name},
            "ndigits": g.rng.choice([None, 1]) if name == "round" else None}


@node("cx_unary", "complex", gen=_gen_unary, weight=0.5)
def cx_unary(a, name, ndigits):
    """-a, +a, abs(a) (hypot; OverflowError when a finite number's magnitude overflows); ~a, math.trunc/floor/ceil
    and round raise TypeError."""
    if name == "round":
        return round(a, ndigits)
    f = {"neg": operator.neg, "pos": operator.pos, "invert": operator.invert, "abs": abs, "trunc": math.trunc,
         "floor": math.floor, "ceil": math.ceil}[name]
    return f(a)
