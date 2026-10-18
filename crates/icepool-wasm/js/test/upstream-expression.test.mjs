// Translates upstream icepool tests/expression_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const d6 = ip.d(6);

/** Python's `xs * n` for a list. */
const repeat = (xs, n) => Array(n).fill(xs).flat();

test("test_keep_highest_expression", () => {
  const result = d6.pool(4).and(repeat([1, 2, 3, 4, 5, 6], 4)).highest(3).sum();

  const dropLowest = (a, b, c, e) => a + b + c + e - Math.min(a, b, c, e);

  const expected = ip.map(dropLowest, d6, d6, d6, d6);

  assertDieEquals(result, expected);
});

test("test_additive_union", () => {
  assert.ok(Number(d6.pool(4).additiveUnion(d6.pool(2).keep([-1, -1])).sum().lt(0).mean()) > 0);
});

test("test_difference_keep_negative", () => {
  assert.ok(Number(d6.pool(4).difference(d6.pool(2), { keepNegativeCounts: true }).sum().lt(0).mean()) > 0);
});

test("test_intersection", () => {
  assert.ok(Number(d6.pool(4).intersection(d6.pool(2).keep([-1, -1])).sum().lt(0).mean()) > 0);
});

test("test_union", () => {
  assert.ok(Number(d6.pool(4).keep([-1, -1, -1, -1]).union(d6.pool(2).keep([-1, -1])).sum().lt(0).mean()) > 0);
});

test("test_pos", () => {
  assert.ok(pyEq(d6.pool(2).keep([-1, -1]).pos().xor(d6.pool(2).keep([-1, -1]).pos()).sum().probability(0), 1));
});
