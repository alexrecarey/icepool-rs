// Translates upstream tests/function_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { d, minOutcome, maxOutcome, pointwiseMax, pointwiseMin } = ip;
const [d6, d8, d20] = [d(6), d(8), d(20)];
const threeD6 = ip.Die.sum(3, d6);

/** Python's `max(a, b)` of two Fractions. */
const maxFraction = (a, b) => (ip.ops.ge(a, b) ? a : b);

test("test_min_outcome_single_arg", () => {
  assert.equal(minOutcome([d6, d8]), 1);
});

test("test_min_outcome_multiple_arg", () => {
  assert.equal(minOutcome(d6, d8), 1);
});

test("test_min_outcome_bare_outcome", () => {
  assert.equal(minOutcome(d6, d8, 0, 2), 0);
});

test("test_max_outcome_single_arg", () => {
  assert.equal(maxOutcome([d6, d8]), 8);
});

test("test_max_outcome_multiple_arg", () => {
  assert.equal(maxOutcome(d6, d8), 8);
});

test("test_max_outcome_bare_outcome", () => {
  assert.equal(maxOutcome(d6, d8, 10, 2), 10);
});

/** `result.probability(comparison, outcome) == max(...)` for every outcome of a d20. */
function checkPointwise(result, comparison) {
  for (let outcome = 1; outcome <= 20; outcome++) {
    const expected = maxFraction(threeD6.probability(comparison, outcome), d20.probability(comparison, outcome));
    assert.equal(result.probability(comparison, outcome).toString(), expected.toString(), `outcome ${outcome}`);
  }
}

test("test_pointwise_max", () => {
  checkPointwise(pointwiseMax(threeD6, d20), ">=");
});

test("test_pointwise_max_single_argument", () => {
  checkPointwise(pointwiseMax([threeD6, d20]), ">=");
});

test("test_pointwise_min", () => {
  checkPointwise(pointwiseMin(threeD6, d20), "<=");
});

test("test_pointwise_min_single_argument", () => {
  checkPointwise(pointwiseMin([threeD6, d20]), "<=");
});
