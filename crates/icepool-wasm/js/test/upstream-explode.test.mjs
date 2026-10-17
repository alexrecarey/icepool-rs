// Translates upstream tests/explode_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals } from "./upstream.mjs";

function bfExplodeBasic(die, depth) {
  function f(...outcomes) {
    let result = 0;
    for (const outcome of outcomes) {
      result += outcome;
      if (outcome < die.maxOutcome()) break;
    }
    return result;
  }
  return ip.map(f, ...Array(depth + 1).fill(die), { star: false });
}

for (let depth = 0; depth < 6; depth++) {
  test(`test_explode_d6[${depth}]`, () => {
    const result = ip.d(6).explode(null, { depth });
    const resultInt = ip.d(6).explode([6], { depth });
    const expected = bfExplodeBasic(ip.d(6), depth);
    assertDieEquals(result, expected);
    assertDieEquals(resultInt, expected);
  });
}

for (let depth = 0; depth < 6; depth++) {
  test(`test_explode_multiple_weight[${depth}]`, () => {
    const result = ip.d(6).explode([5, 6], { depth });
    assert.equal(result.denominator(), 6n ** BigInt(depth + 1));
  });
}
