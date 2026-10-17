"""Exact comparison of two results. PROTECTED.

"Identical" means: both raised, or both returned and the encoded results are equal as JSON values. Everything is
compared exactly: outcome order, outcome types, integer quantities (zero quantities included), float bits (via
repr). There is no tolerance, except the opt-in ICEPOOL_FLOAT_ULPS for WASM runs. Values are compared as canonical JSON text, so JSON `true` never equals `1`.
"""
import json
import math
import os
import struct

# Floats may differ by up to this many ULPs (units in the last place). 0 = bit-exact, the default everywhere.
# The gate sets it only for runs through WASM: there the port uses Rust's own float math (sin, pow, log...), which
# can round the last bit differently from the platform libm Python uses (Alex's call, 2026-10-17). Native runs, the
# Python facade and cargo test stay bit-exact.
FLOAT_ULPS = int(os.environ.get("ICEPOOL_FLOAT_ULPS", "0"))


def _ulps(a, b):
    def key(f):
        i = struct.unpack("<q", struct.pack("<d", f))[0]
        return i if i >= 0 else -(i & 0x7FFFFFFFFFFFFFFF)
    return abs(key(a) - key(b))


def _float_close(a, b):
    try:
        fa, fb = float(a), float(b)
    except (TypeError, ValueError):
        return False
    if math.isnan(fa) or math.isnan(fb):
        return math.isnan(fa) and math.isnan(fb)
    if math.isinf(fa) or math.isinf(fb):
        return fa == fb
    return _ulps(fa, fb) <= FLOAT_ULPS


def _eq(a, b):
    """Structural equality; {"float": ...} leaves within FLOAT_ULPS of each other count as equal."""
    if isinstance(a, dict) and isinstance(b, dict):
        if set(a) == {"float"} and set(b) == {"float"}:
            return a["float"] == b["float"] or _float_close(a["float"], b["float"])
        return a.keys() == b.keys() and all(_eq(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(_eq(x, y) for x, y in zip(a, b))
    return canon(a) == canon(b)


def match(a, b):
    return canon(a) == canon(b) if FLOAT_ULPS == 0 else _eq(a, b)


def canon(x):
    return json.dumps(x, sort_keys=True, separators=(",", ":"))


def same(expected, actual):
    """-> (True, None) or (False, short reason)."""
    if not isinstance(actual, dict) or "ok" not in actual:
        return False, f"malformed result: {str(actual)[:200]}"
    # An op the port hasn't implemented is never a match, even against an oracle error.
    if not actual["ok"] and str(actual.get("error", "")).startswith("unimplemented"):
        return False, f"port: {actual['error']}"
    if not expected["ok"] or not actual["ok"]:
        if expected["ok"] == actual["ok"]:
            return True, None
        if not expected["ok"]:
            return False, f"oracle raised {expected.get('error')}, port returned a result"
        return False, f"port raised {actual.get('error')!r}, oracle returned a result"
    if "die" in expected:
        if "die" not in actual:
            return False, "oracle returned a Die, port returned a value"
        e, a = expected["die"], actual["die"]
        if len(e) != len(a):
            return False, f"{len(e)} outcomes expected, port gave {len(a)}"
        for i, (eo, ao) in enumerate(zip(e, a)):
            if not match(eo, ao):
                return False, f"item {i}: expected {eo}, port gave {ao}"
        return True, None
    if "value" not in actual:
        return False, "oracle returned a value, port returned a Die"
    if not match(expected["value"], actual["value"]):
        return False, f"expected {str(expected['value'])[:200]}, port gave {str(actual['value'])[:200]}"
    return True, None
