"""Exact comparison of two results. PROTECTED.

"Identical" means: both raised, or both returned and the encoded results are equal as JSON values. Everything is
compared exactly: outcome order, outcome types, integer quantities (zero quantities included), float bits (via
repr). There is no tolerance anywhere. Values are compared as canonical JSON text, so JSON `true` never equals `1`.
"""
import json


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
            if canon(eo) != canon(ao):
                return False, f"item {i}: expected {eo}, port gave {ao}"
        return True, None
    if "value" not in actual:
        return False, "oracle returned a value, port returned a Die"
    if canon(expected["value"]) != canon(actual["value"]):
        return False, f"expected {str(expected['value'])[:200]}, port gave {str(actual['value'])[:200]}"
    return True, None
