// Translates upstream icepool tests/generator_operator_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const { Pool, Deck, Die, Order, MultisetExpression, multisetFunction } = ip;
const d6 = ip.d(6);

test("test_difference", () => {
  const result = d6.pool(3).sub(d6.pool(1)).size();
  const expected = ip.highest(d6.ne(6).rmatmul(3), 2);
  assertDieEquals(result.simplify(), expected.simplify());
});

test("test_difference_keep_negative_counts", () => {
  const result = d6.pool(3).difference(d6.pool(1), { keepNegativeCounts: true }).size();
  assert.ok(pyEq(result.probability(2), 1));
});

test("test_intersection", () => {
  // Python's `pool & {6: 5}`: a dict converts implicitly to `Pool({6: 5})`.
  const result = d6.pool(10).and(new Pool(new Map([[6, 5]]))).size();
  const expected = ip.lowest(d6.eq(6).rmatmul(10), 5);
  assertDieEquals(result, expected);
});

test("test_mul", () => {
  const result = d6.pool(3).mul(2).sum();
  const expected = d6.rmatmul(3).mul(2);
  assertDieEquals(result, expected);
});

test("test_multiple_union", () => {
  const result = d6.pool(1).union([6], [7]).sum();
  const expected = d6.add(12);
  assertDieEquals(result, expected);
});

test("test_multiple_additive_union", () => {
  // The difference keeps the left side from being merged into a single pool.
  const result = d6.pool(1).sub([7]).additiveUnion([1], [2]).sum();
  const expected = d6.add(3);
  assertDieEquals(result, expected);
});

test("test_multiple_intersection", () => {
  const result = d6.pool(1).intersection([d6], [d6]).sum();
  const expected = d6.rmatmul(2).eq(12).mul(d6);
  assertDieEquals(result, expected);
});

const opsAndExpected = [
  ["add", (a, b) => a.add(b), [1, 1, 2, 2, 2, 3, 4]],
  ["sub", (a, b) => a.sub(b), [2, 3]],
  ["and_", (a, b) => a.and(b), [1, 2]],
  ["or_", (a, b) => a.or(b), [1, 2, 2, 3, 4]],
  ["xor", (a, b) => a.xor(b), [2, 3, 4]],
];

opsAndExpected.forEach(([name, op, expected], i) => {
  test(`test_operator_examples[${name}-expected${i}]`, () => {
    const result = op(new Pool([1, 2, 2, 3]), new Pool([1, 2, 4])).expand(Order.Ascending);
    assertDieEquals(result, new Die([expected]));
  });
});

opsAndExpected.forEach(([name, op, expected], i) => {
  test(`test_operator_examples_deal[${name}-expected${i}]`, () => {
    const result = op(new Deck([1, 2, 2, 3]).deal(4), new Deck([1, 2, 4]).deal(3)).expand(Order.Ascending);
    assertDieEquals(result, new Die([expected]));
  });
});

test("test_example_multiply_counts", () => {
  const result = new Pool([1, 2, 2, 3]).mul(2).expand(Order.Ascending);
  const expected = new Die([[1, 1, 2, 2, 2, 2, 3, 3]]);
  assertDieEquals(result, expected);
});

test("test_example_divide_counts", () => {
  const result = new Pool([1, 2, 2, 3]).floordiv(2).expand(Order.Ascending);
  const expected = new Die([[2]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_le", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts("<=", 2).expand(Order.Ascending);
  const expected = new Die([[1, 2, 2]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_lt", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts("<", 2).expand(Order.Ascending);
  const expected = new Die([[1]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_ge", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts(">=", 2).expand(Order.Ascending);
  const expected = new Die([[2, 2, 3, 3, 3]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_gt", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts(">", 2).expand(Order.Ascending);
  const expected = new Die([[3, 3, 3]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_eq", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts("==", 2).expand(Order.Ascending);
  const expected = new Die([[2, 2]]);
  assertDieEquals(result, expected);
});

test("test_example_keep_counts_ne", () => {
  const result = new Pool([1, 2, 2, 3, 3, 3]).keepCounts("!=", 2).expand(Order.Ascending);
  const expected = new Die([[1, 3, 3, 3]]);
  assertDieEquals(result, expected);
});

// Python's `pool.map_counts(function=...)` is the static `MultisetExpression.mapCounts(pool, { function })` in JS.
test("test_example_keep_counts_ge_using_map", () => {
  const result = MultisetExpression.mapCounts(new Pool([1, 2, 2, 3]), { function: (o, c) => (c >= 2 ? c : 0) }).expand(Order.Ascending);
  const expected = new Die([[2, 2]]);
  assertDieEquals(result, expected);
});

test("test_example_unique", () => {
  const result = new Pool([1, 2, 2, 3]).unique().expand(Order.Ascending);
  const expected = new Die([[1, 2, 3]]);
  assertDieEquals(result, expected);
});

test("test_example_unique_using_map", () => {
  const result = MultisetExpression.mapCounts(new Pool([1, 2, 2, 3]), { function: (_, c) => Math.min(c, 1) }).expand(Order.Ascending);
  const expected = new Die([[1, 2, 3]]);
  assertDieEquals(result, expected);
});

test("test_example_intersect_using_map", () => {
  const intersect = (_, a, b) => Math.min(a, b);

  const result = Pool.mapCounts(d6.pool(3), d6.pool(3), { function: intersect }).size();
  const expected = d6.pool(3).and(d6.pool(3)).size();
  assertDieEquals(result, expected);
});

test("test_mod", () => {
  const expected = multisetFunction((x) => x.sub(x.floordiv(2).mul(2)).expand());

  const pool = d6.pool(4);
  assertDieEquals(pool.mod(2).expand(), expected.call(pool));
});
