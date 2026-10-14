"""The wire format shared by the Python oracle and the Rust port. PROTECTED: the loop may not edit harness/core/.

Outcomes (and literal values inside programs) are typed JSON, so that Python's type distinctions survive the trip:

    int       -> JSON integer (any size; Python's json and Rust's serde_json arbitrary_precision keep it exact)
    bool      -> {"bool": true}          (bool is NOT folded into int: Die([True]) and Die([1]) differ in type)
    str       -> {"str": "abc"}
    tuple     -> {"tuple": [...]}        (also namedtuple: its fields, as a plain tuple)
    float     -> {"float": "<repr>"}     (repr round-trips exactly; inf / -inf / nan as Python spells them)
    Fraction  -> {"frac": [num, den]}    (normalised, den > 0)
    None      -> null
    Vector    -> {"vector": [...]}
    Symbols   -> {"symbols": "<str(symbols)>"}
    Die       -> {"die": [[outcome, quantity], ...]}   (when a Die appears *inside* a value, e.g. a tuple of dice)
    anything else -> {"repr": "<type name>: <repr>"}  (unsupported: the port should never need to produce it)

Results of running a program:

    {"ok": true, "die": [[outcome, quantity], ...]}   a Die: every (outcome, quantity) in icepool's order,
                                                        zero quantities included
    {"ok": true, "value": <typed value>}              anything else (a number, a Fraction, a tuple of dice, ...)
    {"ok": false, "error": "<ExceptionType>"}         raised. Two errors always compare equal: messages and
                                                        exception types are informational only.
"""
from fractions import Fraction
import math

import icepool


def enc(x):
    if x is None:
        return None
    if isinstance(x, bool):
        return {"bool": x}
    if isinstance(x, int):
        return x
    if isinstance(x, str):
        return {"str": x}
    if isinstance(x, float):
        return {"float": repr(x)}
    if isinstance(x, Fraction):
        return {"frac": [x.numerator, x.denominator]}
    if isinstance(x, icepool.Vector):
        return {"vector": [enc(v) for v in x]}
    if isinstance(x, icepool.Symbols):
        return {"symbols": str(x)}
    if isinstance(x, icepool.Die):
        return {"die": [[enc(o), q] for o, q in x.items()]}
    if isinstance(x, tuple):
        return {"tuple": [enc(v) for v in x]}
    return {"repr": f"{type(x).__name__}: {x!r}"}


def dec(j):
    """Typed JSON -> Python value (used for literals inside programs)."""
    if j is None or isinstance(j, int) and not isinstance(j, bool):
        return j
    if isinstance(j, bool):
        raise ValueError('bare JSON booleans are not allowed; write {"bool": true}')
    if isinstance(j, float):
        raise ValueError('bare JSON floats are not allowed; write {"float": "1.5"}')
    if isinstance(j, str):
        raise ValueError('bare JSON strings are not allowed; write {"str": "x"}')
    if isinstance(j, list):
        raise ValueError('bare JSON lists are not allowed as values; write {"tuple": [...]}')
    if isinstance(j, dict) and len(j) == 1:
        (k, v), = j.items()
        if k == "bool":
            return bool(v)
        if k == "str":
            return str(v)
        if k == "float":
            return float(v)
        if k == "frac":
            return Fraction(v[0], v[1])
        if k == "tuple":
            return tuple(dec(e) for e in v)
        if k == "vector":
            return icepool.Vector(dec(e) for e in v)
        if k == "symbols":
            return icepool.Symbols(v)
    raise ValueError(f"not a typed value: {j!r}")


def result_of(value):
    """Encode a program's return value."""
    if isinstance(value, icepool.Die):
        return {"ok": True, "die": [[enc(o), q] for o, q in value.items()]}
    return {"ok": True, "value": enc(value)}


def error_of(exc):
    return {"ok": False, "error": type(exc).__name__}


def is_nan(j):
    return isinstance(j, dict) and j.get("float") == "nan"
