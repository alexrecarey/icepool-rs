// Helpers for the upstream-*.test.mjs files: upstream icepool's Python tests ($ICEPOOL_ORACLE/tests/*_test.py)
// translated to the JS API, one JS test per Python test function, with the same name and the same assertions.

import assert from "node:assert/strict";
import * as ip from "../dist/node.js";

export { ip };

/** `pytest.approx(expected)` for a number, or element by element for an array (relative 1e-6, absolute 1e-12, as
 * pytest's defaults). Fractions and Floats compare as their numbers. */
export function approx(actual, expected, message) {
  if (Array.isArray(expected)) {
    assert.ok(Array.isArray(actual) || actual?.[Symbol.iterator], message ?? `expected an array, got ${actual}`);
    const a = Array.from(actual);
    assert.equal(a.length, expected.length, message);
    a.forEach((x, i) => approx(x, expected[i], message));
    return;
  }
  const [x, e] = [Number(actual), Number(expected)];
  const tolerance = Math.max(1e-6 * Math.abs(e), 1e-12);
  assert.ok(Math.abs(x - e) <= tolerance, message ?? `${x} is not approximately ${e}`);
}

/** `with pytest.raises(PythonType): f()`: `f` throws an `IcepoolError` whose `pythonType` is one of the given names
 * (Python's exception hierarchy flattened: pass every name Python's `except` would catch). */
export function raises(f, ...pythonTypes) {
  assert.throws(f, (e) => {
    assert.ok(e instanceof ip.IcepoolError, `expected an IcepoolError, got ${e?.stack ?? e}`);
    assert.ok(pythonTypes.includes(e.pythonType), `expected ${pythonTypes.join(" or ")}, got ${e.pythonType}: ${e.message}`);
    return true;
  });
}

/** `die.equals(expected)` with the two dice's reprs in the failure message. */
export function assertDieEquals(actual, expected, options = {}) {
  assert.ok(actual.equals(expected, options), `${actual.repr()} does not equal ${expected.repr()}`);
}

/** Python's `x == y` between two plain values (numbers of any type compare by value, as in Python). */
export function pyEq(x, y) {
  const result = ip.ops.eq(x, y);
  return typeof result === "boolean" ? result : ip.truth(result);
}
