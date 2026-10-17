// Translates upstream tests/comparator_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const d4 = ip.d(4);
const d6 = ip.d(6);

test("test_lt", () => {
  const result = d6.lt(d6);
  const expected = ip.coin(15, 36);
  assertDieEquals(result, expected);
});

test("test_lt_fixed", () => {
  assert.ok(pyEq(d6.lt(1).mean(), 0));
  assert.ok(pyEq(d6.lt(7).mean(), 1));
});

test("test_lt_len", () => {
  assert.equal(d6.lt(1).size, 1);
  assert.equal(d6.lt(7).size, 1);
});

test("test_gt", () => {
  const result = d6.gt(d6);
  const expected = ip.coin(15, 36);
  assertDieEquals(result, expected);
});

test("test_gt_fixed", () => {
  assert.ok(pyEq(d6.gt(0).mean(), 1));
  assert.ok(pyEq(d6.gt(6).mean(), 0));
});

test("test_gt_len", () => {
  assert.equal(d6.gt(0).size, 1);
  assert.equal(d6.gt(6).size, 1);
});

test("test_leq", () => {
  const result = d6.le(d6);
  const expected = ip.coin(21, 36);
  assertDieEquals(result, expected);
});

test("test_leq_fixed", () => {
  assert.ok(pyEq(d6.le(0).mean(), 0));
  assert.ok(pyEq(d6.le(6).mean(), 1));
});

test("test_leq_len", () => {
  assert.equal(d6.le(0).size, 1);
  assert.equal(d6.le(6).size, 1);
});

test("test_geq", () => {
  const result = d6.ge(d6);
  const expected = ip.coin(21, 36);
  assertDieEquals(result, expected);
});

test("test_geq_fixed", () => {
  assert.ok(pyEq(d6.ge(1).mean(), 1));
  assert.ok(pyEq(d6.ge(7).mean(), 0));
});

test("test_geq_len", () => {
  assert.equal(d6.ge(1).size, 1);
  assert.equal(d6.ge(7).size, 1);
});

test("test_eq", () => {
  const result = d6.eq(d6);
  const expected = ip.coin(6, 36);
  assertDieEquals(result, expected);
});

test("test_eq_fixed", () => {
  assertDieEquals(d6.eq(1), ip.coin(1, 6));
  assertDieEquals(d6.eq(6), ip.coin(1, 6));
});

test("test_ne", () => {
  const result = d6.ne(d6);
  const expected = ip.coin(30, 36);
  assertDieEquals(result, expected);
});

test("test_ne_fixed", () => {
  assertDieEquals(d6.ne(1), ip.coin(5, 6));
  assertDieEquals(d6.ne(6), ip.coin(5, 6));
});

test("test_sign", () => {
  const result = d6.sub(3).sign();
  const expected = new ip.Die(new Map([[-1, 2n], [0, 1n], [1, 3n]]));
  assertDieEquals(result, expected);
});

test("test_cmp", () => {
  const result = d6.cmp(d6.sub(1));
  const expected = new ip.Die(new Map([[-1, 10n], [0, 5n], [1, 21n]]));
  assertDieEquals(result, expected);
});

test("test_cmp_len", () => {
  assert.equal(d6.cmp(0).size, 1);
  assert.equal(d6.cmp(7).size, 1);
  assert.equal(new ip.Die([1]).cmp(1).size, 1);
  assert.equal(new ip.Die(new Map([[-1, 0n], [0, 0n], [1, 0n]])).cmp(0).size, 0);
});

test("test_quantity_le", () => {
  assert.equal(d6.quantity("<=", 3), 3n);
});

test("test_quantity_lt", () => {
  assert.equal(d6.quantity("<", 3), 2n);
});

test("test_quantity_le_min", () => {
  assert.equal(d6.quantity("<=", 1), 1n);
});

test("test_quantity_lt_min", () => {
  assert.equal(d6.quantity("<", 1), 0n);
});

test("test_quantity_ge", () => {
  assert.equal(d6.quantity(">=", 3), 4n);
});

test("test_quantity_gt", () => {
  assert.equal(d6.quantity(">", 3), 3n);
});

test("test_quantity_ge_max", () => {
  assert.equal(d6.quantity(">=", 6), 1n);
});

test("test_quantity_gt_max", () => {
  assert.equal(d6.quantity(">", 6), 0n);
});

const dieSpaced = new ip.Die([-3, -2, -1, 0, 1, 2, 3], [1, 0, 0, 1, 0, 0, 1]);

test("test_quantity_le_zero_weight", () => {
  assert.equal(dieSpaced.quantity("<=", -1), 1n);
  assert.equal(dieSpaced.quantity("<=", 0), 2n);
  assert.equal(dieSpaced.quantity("<=", 1), 2n);
});

test("test_quantity_lt_zero_weight", () => {
  assert.equal(dieSpaced.quantity("<", -1), 1n);
  assert.equal(dieSpaced.quantity("<", 0), 1n);
  assert.equal(dieSpaced.quantity("<", 1), 2n);
});

test("test_quantity_ge_zero_weight", () => {
  assert.equal(dieSpaced.quantity(">=", -1), 2n);
  assert.equal(dieSpaced.quantity(">=", 0), 2n);
  assert.equal(dieSpaced.quantity(">=", 1), 1n);
});

test("test_quantity_gt_zero_weight", () => {
  assert.equal(dieSpaced.quantity(">", -1), 2n);
  assert.equal(dieSpaced.quantity(">", 0), 1n);
  assert.equal(dieSpaced.quantity(">", 1), 1n);
});

test("test_nearest_le", () => {
  assert.equal(d6.nearest("<=", 0), null);
  assert.equal(d6.nearest("<=", 1), 1);
  assert.equal(d6.nearest("<=", 6), 6);
  assert.equal(d6.nearest("<=", 7), 6);
});

test("test_nearest_le_gap", () => {
  const die = new ip.Die([-3, 0, 3]);
  assert.equal(die.nearest("<=", -4), null);
  assert.equal(die.nearest("<=", -3), -3);
  assert.equal(die.nearest("<=", -2), -3);
  assert.equal(die.nearest("<=", -1), -3);
  assert.equal(die.nearest("<=", 0), 0);
  assert.equal(die.nearest("<=", 1), 0);
  assert.equal(die.nearest("<=", 2), 0);
  assert.equal(die.nearest("<=", 3), 3);
  assert.equal(die.nearest("<=", 4), 3);
});

test("test_nearest_lt", () => {
  assert.equal(d6.nearest("<", 0), null);
  assert.equal(d6.nearest("<", 1), null);
  assert.equal(d6.nearest("<", 6), 5);
  assert.equal(d6.nearest("<", 7), 6);
});

test("test_nearest_lt_gap", () => {
  const die = new ip.Die([-3, 0, 3]);
  assert.equal(die.nearest("<", -4), null);
  assert.equal(die.nearest("<", -3), null);
  assert.equal(die.nearest("<", -2), -3);
  assert.equal(die.nearest("<", -1), -3);
  assert.equal(die.nearest("<", 0), -3);
  assert.equal(die.nearest("<", 1), 0);
  assert.equal(die.nearest("<", 2), 0);
  assert.equal(die.nearest("<", 3), 0);
  assert.equal(die.nearest("<", 4), 3);
});

test("test_nearest_ge", () => {
  assert.equal(d6.nearest(">=", 0), 1);
  assert.equal(d6.nearest(">=", 1), 1);
  assert.equal(d6.nearest(">=", 6), 6);
  assert.equal(d6.nearest(">=", 7), null);
});

test("test_nearest_ge_gap", () => {
  const die = new ip.Die([-3, 0, 3]);
  assert.equal(die.nearest(">=", -4), -3);
  assert.equal(die.nearest(">=", -3), -3);
  assert.equal(die.nearest(">=", -2), 0);
  assert.equal(die.nearest(">=", -1), 0);
  assert.equal(die.nearest(">=", 0), 0);
  assert.equal(die.nearest(">=", 1), 3);
  assert.equal(die.nearest(">=", 2), 3);
  assert.equal(die.nearest(">=", 3), 3);
  assert.equal(die.nearest(">=", 4), null);
});

test("test_nearest_gt", () => {
  assert.equal(d6.nearest(">", 0), 1);
  assert.equal(d6.nearest(">", 1), 2);
  assert.equal(d6.nearest(">", 6), null);
  assert.equal(d6.nearest(">", 7), null);
});

test("test_nearest_gt_gap", () => {
  const die = new ip.Die([-3, 0, 3]);
  assert.equal(die.nearest(">", -4), -3);
  assert.equal(die.nearest(">", -3), 0);
  assert.equal(die.nearest(">", -2), 0);
  assert.equal(die.nearest(">", -1), 0);
  assert.equal(die.nearest(">", 0), 3);
  assert.equal(die.nearest(">", 1), 3);
  assert.equal(die.nearest(">", 2), 3);
  assert.equal(die.nearest(">", 3), null);
  assert.equal(die.nearest(">", 4), null);
});

test("test_highest", () => {
  const result = ip.highest(d4.add(1), d6);
  const expected = new ip.Die(new Map([[2, 2n], [3, 4n], [4, 6n], [5, 8n], [6, 4n]]));
  assertDieEquals(result, expected);
});

test("test_lowest", () => {
  const result = ip.lowest(d4.add(1), d6);
  const expected = new ip.Die(new Map([[1, 4n], [2, 8n], [3, 6n], [4, 4n], [5, 2n]]));
  assertDieEquals(result, expected);
});

// Python compares the float lists with ==, so 25.0 == 25: compare as numbers.
const numbers = (xs) => Array.from(xs, Number);

test("test_probabilities", () => {
  assert.deepEqual(numbers(d4.probabilities()), [0.25, 0.25, 0.25, 0.25]);
  assert.deepEqual(numbers(d4.probabilities("<=")), [0.25, 0.5, 0.75, 1.0]);
  assert.deepEqual(numbers(d4.probabilities("<")), [0.0, 0.25, 0.5, 0.75]);
  assert.deepEqual(numbers(d4.probabilities(">=")), [1.0, 0.75, 0.5, 0.25]);
  assert.deepEqual(numbers(d4.probabilities(">")), [0.75, 0.5, 0.25, 0.0]);

  assert.deepEqual(numbers(d4.probabilities(null, { percent: true })), [25, 25, 25, 25]);
  assert.deepEqual(numbers(d4.probabilities("<=", { percent: true })), [25, 50, 75, 100]);
  assert.deepEqual(numbers(d4.probabilities("<", { percent: true })), [0, 25, 50, 75]);
  assert.deepEqual(numbers(d4.probabilities(">=", { percent: true })), [100, 75, 50, 25]);
  assert.deepEqual(numbers(d4.probabilities(">", { percent: true })), [75, 50, 25, 0]);
});

// Upstream skips it: 'target is no longer supported'.
test.skip("test_probabilities_with_target", () => {});
