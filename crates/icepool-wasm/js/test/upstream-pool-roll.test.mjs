// Translates upstream icepool tests/pool_roll_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals } from "./upstream.mjs";

const { Pool, Die } = ip;

test("test_pool_roll_eq", () => {
  // Python's `==` / `!=` of pools: a DieWithTruth whose truth compares the pools as objects (their hash keys).
  assert.ok(ip.truth(new Pool([1, 2, 3, 4]).eq(new Pool([1, 2, 3, 4]))));
  assert.ok(ip.truth(new Pool([1, 2, 3, 4]).ne(new Pool([1, 1, 2, 2, 3, 3, 4, 4]))));
  assert.ok(ip.truth(new Pool([1, 2, 3, 4]).ne(ip.d(4).pool(1))));
});

test("test_pool_roll_sum", () => {
  const roll = new Pool([1, 2, 3, 4]);
  assertDieEquals(roll.sum(), new Die([10]));
});

test("test_pool_roll_sum_duplicates", () => {
  const roll = new Pool([1, 1, 1, 1, 2, 3, 4]);
  assertDieEquals(roll.sum(), new Die([13]));
});

test("test_pool_roll_dups", () => {
  const roll = new Pool([1, 2, 3], [3, 2, 1]);
  assertDieEquals(roll.sum(), new Die([10]));
});
