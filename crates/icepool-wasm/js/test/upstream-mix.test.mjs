// Translates upstream tests/mix_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, approx, pyEq } from "./upstream.mjs";

test("test_mix_d6_faces", () => {
  const result = new ip.Die([1, 2, 3, 4, 5, 6]);
  const expected = ip.d(6);
  assert.ok(result.equals(expected));
});

test("test_mix_identical", () => {
  approx(new ip.Die([ip.d(6), ip.d(6), ip.d(6), ip.d(6)]).probabilities(), ip.d(6).probabilities());
});

test("test_mix_mixed", () => {
  const die = new ip.Die([ip.d(4), ip.d(6)]);
  const actual = die.probabilities();
  const expected = new ip.Die([1, 2, 3, 4, 5, 6], [5, 5, 5, 5, 2, 2]).probabilities();
  // Python list ==: same length and element-wise Python numeric equality.
  assert.equal(actual.length, expected.length);
  actual.forEach((p, i) => assert.ok(pyEq(p, expected[i]), `${p} != ${expected[i]}`));
});

test("test_mix_reroll", () => {
  const result = new ip.Die([1, 2, 3, 4, ip.Reroll, ip.Reroll]);
  const expected = ip.d(4);
  assert.ok(result.equals(expected));
});

test("test_if_else", () => {
  assert.ok(ip.d(6).ge(4).ifElse(ip.d(6), ip.d(6).add(6)).equals(ip.d(12), { simplify: true }));
  assert.ok(ip.d(6).sub(3).ifElse(true, false).equals(ip.d(6).ne(6), { simplify: true }));
});
