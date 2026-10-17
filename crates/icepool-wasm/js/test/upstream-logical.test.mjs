// Translates upstream tests/logical_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, pyEq } from "./upstream.mjs";

test("test_invert", () => {
  assert.ok(ip.d(6).invert().mean() < 0);
});

test("test_and", () => {
  assert.ok(pyEq(ip.coin(1, 4).and(ip.coin(1, 4)).mean(), new ip.Fraction(1, 16)));
});

test("test_or", () => {
  assert.ok(pyEq(ip.coin(1, 4).or(ip.coin(1, 4)).mean(), new ip.Fraction(7, 16)));
});

test("test_xor", () => {
  console.log(ip.coin(1, 4).xor(ip.coin(1, 4)).toString());
  console.log(String(ip.coin(1, 4).xor(ip.coin(1, 4)).probability(1)));
  assert.ok(pyEq(ip.coin(1, 4).xor(ip.coin(1, 4)).mean(), new ip.Fraction(6, 16)));
});

test("test_ifelse", () => {
  const result = ip.coin(1, 2).ifElse(ip.d(8), ip.d(6)).simplify();
  const expected = new ip.Die([1, 2, 3, 4, 5, 6, 7, 8], [14, 14, 14, 14, 14, 14, 6, 6]).simplify();
  assert.ok(result.equals(expected));
});
