// Translates upstream tests/reroll_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals } from "./upstream.mjs";

test("test_reroll_default", () => {
  const result = ip.d(6).reroll(null, { depth: Infinity });
  const expected = ip.d(5).add(1);
  assertDieEquals(result, expected);
});

test("test_reroll_1", () => {
  const result = ip.d(6).reroll([1], { depth: Infinity });
  const expected = ip.d(5).add(1);
  assertDieEquals(result, expected);
});

test("test_reroll_2", () => {
  const result = ip.d(6).reroll([1, 2], { depth: Infinity });
  const expected = ip.d(4).add(2);
  assertDieEquals(result, expected);
});

test("test_reroll_limit", () => {
  const result = ip.d(4).reroll([1], { depth: 1 });
  const expected = new ip.Die([1, 2, 3, 4], [1, 5, 5, 5]);
  assertDieEquals(result, expected);
});

test("test_reroll_until_limit", () => {
  const result = ip.d(4).filter([2, 3, 4], { depth: 1 });
  const expected = new ip.Die([1, 2, 3, 4], [1, 5, 5, 5]);
  assertDieEquals(result, expected);
});

test("test_reroll_func", () => {
  const result = ip.d(4).reroll((x) => x === 1, { depth: 1 });
  const expected = new ip.Die([1, 2, 3, 4], [1, 5, 5, 5]);
  assertDieEquals(result, expected);
});

test("test_reroll_until_func", () => {
  const result = ip.d(4).filter((x) => x !== 1, { depth: 1 });
  const expected = new ip.Die([1, 2, 3, 4], [1, 5, 5, 5]);
  assertDieEquals(result, expected);
});

test("test_reroll_depth_0", () => {
  assert.ok(ip.d(4).reroll(null, { depth: 0 }).eq(ip.d(4)).truth());
});

test("test_reroll_depth_3", () => {
  const result = ip.d(4).reroll(null, { depth: 3 });
  const expected = ip.d(4).reroll(null, { depth: 1 }).reroll(null, { depth: 1 });
  assertDieEquals(result, expected);
});

test("test_reroll_depth_3_mixed", () => {
  const die = ip.Die.sum(2, ip.d(6));
  const result = die.reroll([2, 3], { depth: 3 });
  const expected = die.reroll([2, 3], { depth: 1 }).reroll([2, 3], { depth: 1 });
  assertDieEquals(result, expected);
});

test("test_infinite_reroll", () => {
  assert.ok(ip.d(4).reroll([1, 2, 3, 4], { depth: Infinity }).isEmpty());
});

test("test_reroll_multidim", () => {
  const result = new ip.Die([[1, 0], [0, 1]]).reroll((x) => x[0] === 0, { depth: Infinity });
  const expected = new ip.Die([[1, 0]]);
  assertDieEquals(result, expected);
});

test("test_reroll_until_multidim", () => {
  const result = new ip.Die([[1, 0], [0, 1]]).filter((x) => x[0] === 0, { depth: Infinity });
  const expected = new ip.Die([[0, 1]]);
  assertDieEquals(result, expected);
});
