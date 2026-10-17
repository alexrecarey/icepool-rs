// Translates upstream icepool tests/keep_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, raises, assertDieEquals } from "./upstream.mjs";

const { Pool, Die, Deck, multisetFunction, Order } = ip;
const d6 = ip.d(6);

/** Python's `xs[start:stop]` (negative and out-of-range bounds as Python treats them). */
function pySlice(xs, start, stop = xs.length) {
  const clamp = (i) => Math.min(Math.max(i < 0 ? i + xs.length : i, 0), xs.length);
  return xs.slice(clamp(start), clamp(stop));
}
const sorted = (xs) => [...xs].sort((a, b) => a - b);
const sum = (xs) => xs.reduce((a, b) => a + b, 0);

function bfKeepHighest(die, numDice, keep, drop = 0) {
  if (keep === 0) return new Die([0]);
  const f = (...outcomes) => sum(pySlice(sorted(outcomes), -(keep + drop), outcomes.length - drop));
  return ip.map(f, ...Array(numDice).fill(die));
}

function bfKeepLowest(die, numDice, keep, drop = 0) {
  if (keep === 0) return new Die([0]);
  const f = (...outcomes) => sum(pySlice(sorted(outcomes), drop, keep + drop));
  return ip.map(f, ...Array(numDice).fill(die));
}

function bfDiffHighestLowest(die, numDice) {
  const f = (...outcomes) => Math.max(...outcomes) - Math.min(...outcomes);
  return ip.map(f, ...Array(numDice).fill(die));
}

for (let keep = 1; keep < 6; keep++) {
  test(`test_keep_highest[${keep}]`, () => {
    const die = ip.d(12);
    const result = die.highest(4, keep);
    const expected = bfKeepHighest(die, 4, keep);
    assertDieEquals(result, expected);
  });
}

for (let keep = 1; keep < 6; keep++) {
  test(`test_keep_highest_drop_highest[${keep}]`, () => {
    const die = ip.d(12);
    const result = die.highest(4, keep, 1);
    const expected = bfKeepHighest(die, 4, keep, 1);
    assertDieEquals(result, expected);
  });
}

test("test_keep_highest_open_bound", () => {
  const die = ip.d(12);
  const result = die.highest(4, null, 1);
  const expected = bfKeepHighest(die, 4, 3, 1);
  assertDieEquals(result, expected);
});

for (let keep = 1; keep < 6; keep++) {
  test(`test_keep_lowest[${keep}]`, () => {
    const die = ip.d(12);
    const result = die.lowest(4, keep);
    const expected = bfKeepLowest(die, 4, keep);
    assertDieEquals(result, expected);
  });
}

for (let keep = 1; keep < 6; keep++) {
  test(`test_keep_lowest_drop_highest[${keep}]`, () => {
    const die = ip.d(12);
    const result = die.lowest(4, keep, 1);
    const expected = bfKeepLowest(die, 4, keep, 1);
    assertDieEquals(result, expected);
  });
}

test("test_keep_lowest_open_bound", () => {
  const die = ip.d(12);
  const result = die.lowest(4, null, 1);
  const expected = bfKeepLowest(die, 4, 3, 1);
  assertDieEquals(result, expected);
});

test("test_pool_select", () => {
  const pool = new Pool(Array(5).fill(d6));
  assertDieEquals(pool.at(-2), pool.slice(-2, -1).sum());
  assert.deepEqual(pool.slice(-2, null).keepTuple(), [0, 0, 0, 1, 1]);
});

test("test_sum_from_pool", () => {
  const pool = new Pool(Array(5).fill(d6));
  assert.ok(pool.sum().equals(d6.rmatmul(5)));
});

test("test_pool_select_multi", () => {
  const pool = d6.pool(5);
  const result = pool.keep([0, 0, 2, 0, 0]).sum();
  const expected = d6.highest(5, 1, 2).rmul(2);
  assertDieEquals(result, expected);
});

test("test_pool_select_negative", () => {
  const pool = d6.pool(5);
  const result = pool.keep([0, 0, -2, 0, 0]).sum();
  const expected = d6.highest(5, 1, 2).rmul(-2);
  assertDieEquals(result, expected);
});

test("test_pool_select_mixed_sign", () => {
  const pool = d6.pool(2);
  const result = pool.keep([-1, 1]).sum();
  const expected = d6.sub(d6).abs();
  assertDieEquals(result, expected);
});

test("test_pool_select_mixed_sign_split", () => {
  const pool = d6.pool(4);
  const result = pool.keep([-1, 0, 0, 1]).sum();
  const expected = bfDiffHighestLowest(d6, 4);
  assertDieEquals(result, expected);
});

test("test_highest", () => {
  const result = ip.highest(d6, d6);
  const expected = d6.highest(2, 1);
  assertDieEquals(result, expected);
});

test("test_lowest", () => {
  const result = ip.lowest(d6, d6);
  const expected = d6.lowest(2, 1);
  assertDieEquals(result, expected);
});

test("test_double_index", () => {
  const result = d6.pool(3).slice(null, 2).at(-1);
  const expected = d6.pool(3).at(1);
  assertDieEquals(result, expected);
});

test("test_expression_keep", () => {
  const result = d6.pool(3).or(d6.pool(3)).at(0);
  const expected = d6.pool(6).at(0);
  assertDieEquals(result, expected);
});

test("test_bad_pool_keep_int", () => {
  raises(() => d6.pool(3).at(10), "IndexError");
});

test("test_bad_expression_keep_int", () => {
  raises(() => d6.pool(3).and(d6.pool(3)).at(0), "IndexError");
});

test("test_middle_odd", () => {
  const result = new Pool([0, 10, 20]).middle(1).sum();
  assertDieEquals(result, new Die([10]));
});

test("test_middle_even", () => {
  const result = new Pool([0, 10, 20, 30]).middle(2).sum();
  assertDieEquals(result, new Die([30]));
});

test("test_middle_odd_index_even_pool_error", () => {
  raises(() => new Pool([0, 10, 20, 30]).middle(3).sum(), "IndexError");
});

test("test_middle_odd_index_even_pool_low", () => {
  const result = new Pool([0, 10, 20, 30]).middle(3, { tie: "low" }).sum();
  assertDieEquals(result, new Die([30]));
});

test("test_middle_odd_index_even_pool_high", () => {
  const result = new Pool([0, 10, 20, 30]).middle(3, { tie: "high" }).sum();
  assertDieEquals(result, new Die([60]));
});

test("test_middle_even_index_odd_pool_error", () => {
  raises(() => new Pool([0, 10, 20, 30, 40]).middle(2).sum(), "IndexError");
});

test("test_middle_even_index_odd_pool_low", () => {
  const result = new Pool([0, 10, 20, 30, 40]).middle(2, { tie: "low" }).sum();
  assertDieEquals(result, new Die([30]));
});

test("test_middle_even_index_odd_pool_high", () => {
  const result = new Pool([0, 10, 20, 30, 40]).middle(2, { tie: "high" }).sum();
  assertDieEquals(result, new Die([50]));
});

const deal5 = () => new Deck([0, 10, 20, 30, 40]).deal(5);

test("test_keep_expression_near_nonnegative", () => {
  assertDieEquals(deal5().slice(null, 2).sum(), new Die([10]));
});

test("test_keep_expression_far_nonnegative", () => {
  assertDieEquals(deal5().slice(2, null).sum(), new Die([90]));
});

test("test_keep_expression_near_negative", () => {
  assertDieEquals(deal5().slice(-2, null).sum(), new Die([70]));
});

test("test_keep_expression_far_negative", () => {
  assertDieEquals(deal5().slice(null, -2).sum(), new Die([30]));
});

test("test_keep_expression_both_nonnegative", () => {
  assertDieEquals(deal5().slice(1, 4).sum(), new Die([60]));
});

test("test_keep_expression_both_negative", () => {
  assertDieEquals(deal5().slice(-4, -1).sum(), new Die([60]));
});

test("test_keep_expression_positive_negative", () => {
  assertDieEquals(deal5().slice(1, -1).sum(), new Die([60]));
});

test("test_keep_expression_negative_positive", () => {
  assertDieEquals(deal5().slice(-4, 4).sum(), new Die([60]));
});

test("test_keep_die_method", () => {
  assertDieEquals(d6.keep([0, 1, 0]), d6.pool(3).keep(1));
  assertDieEquals(d6.keep(3, 1), d6.pool(3).keep(1));
  assertDieEquals(d6.keep(5, [1, ip.Ellipsis]), d6.lowest(5, 1));
});

test("test_keep_drop_negative_counts", () => {
  assertDieEquals(
    d6.pool(3).sub(d6.pool(3)).highest(1).sum(),
    d6.pool(3).sub(d6.pool(3)).pos().highest(1).sum(),
  );
});

test("test_keep_negative_fallback", () => {
  assertDieEquals(
    d6.pool(2).keep([-1, 1]).highest(1).sum(),
    new Die(new Map([[0, 6], [2, 2], [3, 4], [4, 6], [5, 8], [6, 10]])),
  );
});

test("test_keep_known_size_ascending", () => {
  const f = multisetFunction((x) => x.forceOrder(Order.Ascending).at(-1));
  assertDieEquals(f.call(d6.pool(3)), d6.highest(3));
});

test("test_keep_known_size_descending", () => {
  const f = multisetFunction((x) => x.forceOrder(Order.Descending).at(0));
  assertDieEquals(f.call(d6.pool(3)), d6.lowest(3));
});
