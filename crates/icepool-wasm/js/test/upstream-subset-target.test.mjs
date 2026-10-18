// Translates upstream icepool tests/subset_target_test.py.
import { test } from "node:test";
import { ip, assertDieEquals } from "./upstream.mjs";

const { Pool, Die, Order, MultisetExpression, multisetFunction, pool, zPool } = ip;
const d = ip.d;

/** Python's `xs * n` for a list. */
const repeat = (xs, n) => Array(n).fill(xs).flat();

test("test_contains_subset", () => {
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).issuperset([1, 2, 4]), new Die([true]));
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).issuperset([1, 2, 2, 4]), new Die([false]));
});

test("test_intersection_size", () => {
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).and([1, 2, 4]).size(), new Die([3]));
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).and([1, 2, 2, 4]).size(), new Die([3]));
});

test("test_keep_outcomes_size", () => {
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).keepOutcomes([1, 2]).size(), new Die([3]));
});

test("test_drop_outcomes_size", () => {
  assertDieEquals(new Pool([1, 1, 2, 4, 4]).dropOutcomes([1, 2]).size(), new Die([2]));
});

test("test_largest_count", () => {
  const result = new Pool([1, 1, 2, 4, 4]).largestCount().simplify();
  const expected = new Die([2]);
  assertDieEquals(result, expected);
});

const orders = [["Order.Ascending", Order.Ascending], ["Order.Descending", Order.Descending]];
const range = (n) => [...Array(n).keys()];

for (let wild = 0; wild < 8; wild++) {
  for (const [orderName, order] of orders) {
    test(`test_largest_count_wild[${wild}-${orderName}]`, { todo: "API: largestCount() takes no `wild` option" }, () => {
      const pool = zPool([6, 6, 8, 8]);
      const result = pool.forceOrder(order).largestCount({ wild: [wild] });

      const expectedMap = (rolls) => {
        const c = new Map();
        for (const x of rolls) c.set(x, (c.get(x) ?? 0) + 1);
        const wilds = c.get(wild) ?? 0;
        c.set(wild, 0);
        const n = Math.max(...c.values());
        return n + wilds;
      };

      const expected = ip.map(expectedMap, pool);
      assertDieEquals(result, expected);
    });
  }
}

for (let wild = 0; wild < 8; wild++) {
  for (const [orderName, order] of orders) {
    test(`test_largest_count_wild_low[${wild}-${orderName}]`, { todo: "API: largestCount() takes no `wildLow` option" }, () => {
      const pool = zPool([6, 6, 8, 8]);
      const result = pool.forceOrder(order).largestCount({ wildLow: [wild] });

      const expectedMap = (rolls) => {
        const counts = Array(8).fill(0);
        for (const x of rolls) counts[x] += 1;
        const wilds = counts[wild];
        if (wild === 7) return Math.max(...counts);
        const maxLow = Math.max(...counts.slice(0, wild + 1));
        const maxHigh = Math.max(...counts.slice(wild + 1)) + wilds;
        return Math.max(maxLow, maxHigh);
      };

      const expected = ip.map(expectedMap, pool);
      assertDieEquals(result, expected);
    });
  }
}

for (let wild = 0; wild < 8; wild++) {
  for (const [orderName, order] of orders) {
    test(`test_largest_count_wild_high[${wild}-${orderName}]`, { todo: "API: largestCount() takes no `wildHigh` option" }, () => {
      const pool = zPool([6, 6, 8, 8]);
      const result = pool.forceOrder(order).largestCount({ wildHigh: [wild] });

      const expectedMap = (rolls) => {
        const counts = Array(9).fill(0);
        for (const x of rolls) counts[x] += 1;
        const wilds = counts[wild];
        if (wild === 0) return Math.max(...counts);
        const maxLow = Math.max(...counts.slice(0, wild)) + wilds;
        const maxHigh = Math.max(...counts.slice(wild));
        return Math.max(maxLow, maxHigh);
      };

      const expected = ip.map(expectedMap, pool);
      assertDieEquals(result, expected);
    });
  }
}

test("test_largest_matching_set_and_outcome", () => {
  const pool = new Pool([1, 1, 2, 4, 4]);
  const result = pool.largestCountAndOutcome().simplify();
  const expected = new Die([[2, 4]]);
  assertDieEquals(result, expected);
});

test("test_all_matching_sets", () => {
  const pool = new Pool([1, 1, 2, 4, 4]);
  const result = pool.allCounts().simplify();
  const expected = new Die([[2, 2, 1]]);
  assertDieEquals(result, expected);
});

test("test_largest_straight_full", () => {
  const result = new Pool([1, 2, 3]).largestStraight().simplify();
  const expected = new Die([3]);
  assertDieEquals(result, expected);
});

test("test_largest_straight_gap", () => {
  const result = new Pool([1, 2, 5, 10, 11]).largestStraight().simplify();
  const expected = new Die([2]);
  assertDieEquals(result, expected);
});

test("test_largest_straight_include_outcome", () => {
  const result = new Pool([1, 2, 3]).largestStraightAndOutcome().simplify();
  const expected = new Die([[3, 3]]);
  assertDieEquals(result, expected);
});

test("test_largest_straight_include_outcome_tiebreaker", () => {
  const result = new Pool([1, 2, 5, 10, 11]).largestStraightAndOutcome().simplify();
  const expected = new Die([[2, 11]]);
  assertDieEquals(result, expected);
});

test("test_all_straights", () => {
  const result = new Pool([1, 1, 2, 3, 3, 5]).allStraights().simplify();
  const expected = new Die([[3, 1, 1, 1]]);
  assertDieEquals(result, expected);
});

test("test_all_straights_reduce_counts", { todo: "API: no allStraightsReduceCounts()" }, () => {
  // cribbage double-double run and an extra
  const result = new Pool([1, 1, 2, 3, 3, 5]).allStraightsReduceCounts().simplify();
  const expected = new Die([[[3, 4], [1, 1]]]);
  assertDieEquals(result, expected);
});

// Python calls `MultisetExpression.argsort` on the class, a sequence as `self`: the method of its pool here.
const toExpr = (x) => (x instanceof MultisetExpression ? x : pool(x));

test("test_argsort_drop", () => {
  const result = toExpr([10, 9, 5]).argsort([9, 9], { tie: "drop" });
  const expected = new Die([[0, 1, 0]]);
  assertDieEquals(result, expected);
});

test("test_argsort_drop_nco", () => {
  const action = d(6).pool(3);
  const danger = d(6).pool(2);
  const result = toExpr(action).argsort(danger, { limit: 2, limitPad: 1, tie: "drop" });

  const twoSideDifference = multisetFunction((a, b) => [a.sub(b).expand(Order.Descending), b.sub(a).expand(Order.Descending)]);

  const computeExpected = (a, b) => {
    if (a[0] < b[0]) {
      return [1, a[0] < b[1]];
    } else {
      return [0, a[1] < b[0]];
    }
  };

  const expected = twoSideDifference.call(action.add([-1, -1]), danger.add([0, 0])).map(computeExpected);
  assertDieEquals(result, expected);
});

test("test_argsort_drop_3_args", () => {
  const result = toExpr([10, 9, 5]).argsort([9, 9, 9, 9], [9, 9], { tie: "drop" });
  const expected = new Die([[0, 1, 1, 0]]);
  assertDieEquals(result, expected);
});

test("test_argsort_drop_3_args_limit", () => {
  const result = toExpr([10, 9, 5]).argsort([9, 9, 9, 9], [9, 9], { tie: "drop", limit: 2 });
  const expected = new Die([[0, 1]]);
  assertDieEquals(result, expected);
});

test("test_argsort_left_3_args", () => {
  const result = toExpr([10, 9, 5]).argsort([9, 9, 9, 9], [9, 9], { tie: "left" });
  const expected = new Die([[0, 0, 1, 1, 1, 1, 2, 2, 0]]);
  assertDieEquals(result, expected);
});

test("test_argsort_right_3_args", () => {
  const result = toExpr([10, 9, 5]).argsort([9, 9, 9, 9], [9, 9], { tie: "right" });
  const expected = new Die([[0, 2, 2, 1, 1, 1, 1, 0, 0]]);
  assertDieEquals(result, expected);
});

test("test_argsort_grouped", () => {
  const result = toExpr([10, 9, 5]).argsortGrouped([9, 9]);
  const expected = new Die([[[0], [0, 1, 1], [0]]]);
  assertDieEquals(result, expected);
});

test("test_count_subsets", () => {
  // Python's `pool // [1, 2, 3]` is `countSubset`, and `pool >= [...]` is `issuperset`.
  let result = d(6).pool(6).countSubset([1, 2, 3]).gt(0);
  let expected = d(6).pool(6).issuperset([1, 2, 3]);
  assertDieEquals(result, expected);

  result = d(6).pool(6).countSubset([1, 2, 3]).gt(1);
  expected = d(6).pool(6).issuperset(repeat([1, 2, 3], 2));
  assertDieEquals(result, expected);
});
