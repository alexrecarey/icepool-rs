// Translates upstream tests/vector_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, raises } from "./upstream.mjs";

const { d, vectorize, Die, Vector, Deck } = ip;
const [d4, d6, d8] = [d(4), d(6), d(8)];

/** Python's `a == b` between dice (or Vectors), as a boolean. */
const eq = (a, b) => ip.truth(a.eq(b));
/** Python's `sum(x)` of a Vector or tuple outcome. */
const sum = (x) => Array.from(x).reduce((a, b) => a + b, 0);

test("test_cartesian_product", () => {
  const result = vectorize(d6, d6);
  assert.equal(Number(result.covariance(0, 1)), 0);
  assert.ok(result.map(sum).equals(Die.sum(2, d6)));
});

test("test_cartesian_product_cast", () => {
  const result = vectorize(d6, 2);
  assert.equal(Number(result.covariance(0, 1)), 0);
  assert.ok(result.map(sum).equals(d6.add(2)));
});

test("test_vector_add", () => {
  const result = vectorize(d8, 1).add(vectorize(0, d6));
  assert.ok(result.equals(vectorize(d8, d6.add(1))));
});

test("test_vector_matmul", () => {
  const result = ip.ops.matmul(2, vectorize(d6, d8));
  assert.ok(result.equals(vectorize(Die.sum(2, d6), Die.sum(2, d8))));
});

test("test_vector_compare_eq_to_scalar", () => {
  const result = vectorize(true, false, true).eq(false);
  assert.ok(eq(result, vectorize(false, true, false)));
  assert.equal(ip.truth(result), false);
});

test("test_vector_compare_ne_to_scalar", () => {
  const result = vectorize(true, false, true).ne(true);
  assert.ok(eq(result, vectorize(false, true, false)));
  assert.equal(ip.truth(result), true);
});

test("test_nested_unary_elementwise", () => {
  const result = new Die([vectorize(vectorize(vectorize(1)))]).neg();
  assert.ok(result.marginal(0).marginal(0).marginal(0).equals(new Die([-1])));
});

test("test_nested_binary_elementwise", () => {
  let result = new Die([vectorize(vectorize(vectorize(1)))]);
  result = result.add(result);
  assert.ok(result.marginal(0).marginal(0).marginal(0).equals(new Die([2])));
});

test("test_binary_op_mismatch_outcome_len", () => {
  raises(() => vectorize(d6, d8).add(vectorize(1, 2, 3)), "IndexError");
});

test("test_map_star", () => {
  const result = vectorize(d6, d6).map((a, b) => a + b);
  assert.ok(result.equals(Die.sum(2, d6)));
});

test("test_reroll_star", () => {
  let result = vectorize(d6, d6);
  result = result.reroll((a, b) => a === 6 && b === 6, { depth: Infinity });
  result = result.map((a, b) => a + b);
  assert.ok(result.equals(Die.sum(2, d6).reroll([12], { depth: Infinity })));
});

test("test_filter_star", () => {
  let result = vectorize(d6, d6);
  result = result.filter((a, b) => a === 6 && b === 6, { depth: Infinity });
  result = result.map((a, b) => a + b);
  assert.ok(result.equals(Die.sum(2, d6).filter([12], { depth: Infinity })));
});

test("test_explode_star", () => {
  const base = vectorize(d6, d6);
  const result = base.explode((a, b) => a === 6 && b === 6);
  assert.ok(result.equals(base.explode()));
});

test("test_unpack_marginals", () => {
  const [a, b] = vectorize(d6, d6).marginals();
  assert.ok(eq(a, b));
  assert.ok(eq(a.simplify(), d6));
});

test("test_one_hot", () => {
  class OneHotEvaluator extends ip.MultisetEvaluator {
    nextState(state, order, outcome, count) {
      return [...(state ?? []), count];
    }
    finalOutcome(finalState) {
      return new Vector(finalState);
    }
    extraOutcomes() {
      return [1, 2, 3, 4, 5, 6];
    }
  }
  const result = ip.ops.matmul(3, ip.oneHot(6));
  const expected = new OneHotEvaluator().evaluate(d6.pool(3));
  assert.ok(eq(result, expected));
});

test("test_vector_scalar_mult", () => {
  assert.ok(eq(vectorize(d6, d8).mul(2), vectorize(d6.mul(2), d8.mul(2))));
});

test("test_pool_vector_sum", () => {
  assert.ok(eq(vectorize(d6, d6).pool(2).sum(), vectorize(Die.sum(2, d6), Die.sum(2, d6))));
});

test("test_vector_comparison", () => {
  const result = vectorize(d6, d6).gt(vectorize(0, 0));
  assert.ok(eq(result, new Die([vectorize(true, true)], 36)));
});

test("test_vector_append", () => {
  assert.ok(eq(new Vector([1, 2]).append(3), new Vector([1, 2, 3])));
});

test("test_vector_concatenate", () => {
  assert.ok(eq(new Vector([1, 2]).concatenate([0, 1]), new Vector([1, 2, 0, 1])));
});

test("test_to_one_hot", () => {
  const expected = new Die([new Vector([1, 0, 0, 0]), new Vector([0, 1, 0, 0]), new Vector([0, 0, 1, 0]), new Vector([0, 0, 0, 1])]);
  assert.ok(eq(d4.toOneHot(), expected));
});

// A namedtuple outcome (`marginals.y`): JS has no counterpart.
test.skip("test_named_tuple", () => {});

test("test_auto_tupleize", () => {
  assert.ok(eq(new Die([[d6, d6, 6]]).map(sum), Die.sum(2, d6).add(6)));
});

test("test_die_sequence", () => {
  assert.ok(eq(d6.sequence(3), ip.map((a, b, c) => [a, b, c], d6, d6, d6)));
});

test("test_deck_sequence", () => {
  const result = new Deck([0, 1, 2, 3, 4]).sequence(3);
  assert.equal(result.size, 5 * 4 * 3);
  assert.equal(result.map((a, b, c) => a !== b && a !== c && b !== c).probability(true).toString(), "1");
});

test("test_deck_sequence_with_dups", () => {
  const result = new Deck([0, 0, 1, 1]).sequence(2);
  const expected = new Die(new Map([[[0, 0], 2], [[0, 1], 4], [[1, 0], 4], [[1, 1], 2]]));
  assert.ok(eq(result, expected));
});

test("test_unequal_length_comparison", () => {
  const result = new Vector([0]).lt(new Vector([0, 1]));
  assert.ok(ip.truth(result));
  raises(() => result.length, "ValueError");
});
