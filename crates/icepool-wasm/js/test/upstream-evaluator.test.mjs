// Translates upstream icepool tests/evaluator_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const { Pool, Order, UnsupportedOrder, multisetFunction } = ip;
const [d6, d8, d10] = [ip.d(6), ip.d(8), ip.d(10)];

class SumRerollIfAnyOnes extends ip.MultisetEvaluator {
  nextState(state, order, outcome, count) {
    if (outcome === 1 && count > 0) {
      return ip.Reroll;
    } else if (state === null) {
      return outcome * count;
    } else {
      return state + outcome * count;
    }
  }
}

test("test_reroll", () => {
  const result = new SumRerollIfAnyOnes().evaluate(d6.pool(5));
  const expected = ip.d(5).add(1).rmatmul(5);
  assert.ok(result.equals(expected));
});

test("test_sum_descending", () => {
  const result = d6.pool(3).forceOrder(Order.Descending).sum();
  const expected = d6.rmatmul(3);
  assert.ok(result.equals(expected));
});

test("test_sum_descending_limit_outcomes", () => {
  const result = new Pool([d8.neg(), d6.neg()]).forceOrder(Order.Descending).sum().neg();
  const expected = d6.add(d8);
  assert.ok(result.equals(expected));
});

test("test_sum_descending_keep_highest", () => {
  const result = d6.pool([0, 1, 1, 1]).forceOrder(Order.Descending).sum();
  const expected = d6.highest(4, 3);
  assert.ok(result.equals(expected));
});

test("test_zero_weight_outcomes", () => {
  const result = new ip.Die([0, 1, 2, 3, 4], [0, 1, 0, 1, 0]).highest(3, 2);
  assert.equal(result.size, 3);
});

test("test_standard_pool", () => {
  const result = ip.dPool([8, 8, 6, 6, 6]).sum();
  const expected = d6.rmatmul(3).add(d8.rmatmul(2));
  assert.ok(result.equals(expected));

  const resultDict = ip.dPool(new Map([[8, 2], [6, 3]])).sum();
  assert.ok(result.equals(resultDict));
});

test("test_standard_pool_zero_dice", () => {
  const result = ip.dPool([]).sum();
  const expected = new ip.Die([0]);
  assert.ok(result.equals(expected));
});

// Upstream's LargestStraightAndOutcomeEvaluatorDescending overrides an `order()` method that icepool no longer
// calls, so it evaluates exactly as LargestStraightAndOutcomeEvaluator: its results are translated as the same call.

test("test_runs", () => {
  const pool = ip.dPool([12, 10, 8]);
  const result = pool.largestStraightAndOutcome("high");

  function f(...outcomes) {
    outcomes = [...outcomes].sort((x, y) => x - y);
    const a = outcomes[1] === outcomes[0] + 1;
    const b = outcomes[2] === outcomes[1] + 1;
    if (a && b) return [3, outcomes[2]];
    else if (b) return [2, outcomes[2]];
    else if (a) return [2, outcomes[1]];
    else return [1, outcomes[2]];
  }

  const expected = ip.map(f, pool, { star: true });
  assert.ok(result.equals(expected));

  const resultDescending = pool.largestStraightAndOutcome("high");
  assert.ok(resultDescending.equals(expected));
});

test("test_runs_low", () => {
  const pool = ip.dPool([12, 10, 8]);
  const result = ip.dPool([12, 10, 8]).largestStraightAndOutcome("low");

  function f(...outcomes) {
    outcomes = [...outcomes].sort((x, y) => x - y);
    const a = outcomes[1] === outcomes[0] + 1;
    const b = outcomes[2] === outcomes[1] + 1;
    if (a && b) return [3, outcomes[0]];
    else if (a) return [2, outcomes[0]];
    else if (b) return [2, outcomes[1]];
    else return [1, outcomes[0]];
  }

  const expected = ip.map(f, pool, { star: true });
  assert.ok(result.equals(expected));

  const resultDescending = pool.largestStraightAndOutcome("low");
  assert.ok(resultDescending.equals(expected));
});

test("test_runs_skip", () => {
  const die = new ip.Die([0, 10]);
  const result = die.pool(10).largestStraightAndOutcome("high");
  assert.deepEqual(result.outcomes(), [[1, 0], [1, 10]]);
});

class SumFixedOrderAuto extends ip.MultisetEvaluator {
  nextState(state, order, outcome, count) {
    return (state ?? 0) + outcome * count;
  }
}

class SumFixedOrderAscending extends ip.MultisetEvaluator {
  nextState(state, order, outcome, count) {
    if (order !== Order.Ascending) throw new UnsupportedOrder();
    return (state ?? 0) + outcome * count;
  }
}

class SumFixedOrderDescending extends ip.MultisetEvaluator {
  nextState(state, order, outcome, count) {
    if (order !== Order.Descending) throw new UnsupportedOrder();
    return (state ?? 0) + outcome * count;
  }
}

const testPools = [
  ip.dPool([6, 6, 6]),
  ip.dPool([6, 6, 6, 6]).keep([0, 0, 0, 1]),
  ip.dPool([6, 6, 6, 6]).keep([0, 1, 1, 1]),
  ip.dPool([6, 6, 6, 6]).keep([-1, 0, 0, 1]),
  ip.dPool([12, 10, 8, 8, 6, 6, 6, 4]),
  new Pool([d6.neg(), d8.neg(), d10.neg()]),
  d6.rmatmul(3).pool(12).slice(-6, null),
];

const evalAscending = new SumFixedOrderAscending();
const evalDescending = new SumFixedOrderDescending();
const evalAuto = new SumFixedOrderAuto();

testPools.forEach((pool, i) => {
  test(`test_sum_order[pool${i}]`, () => {
    assert.ok(evalAscending.evaluate(pool).equals(evalDescending.evaluate(pool)));
    assert.ok(evalAscending.evaluate(pool).equals(evalAuto.evaluate(pool)));
  });
});

test("test_joint_evaluate", () => {
  const testEvaluator = multisetFunction((x) => [x.sum(), x.sum()]);

  const result = testEvaluator.call(d6.pool(3));
  const expected = d6.rmatmul(3).map((x) => [x, x]);
  assert.ok(result.equals(expected));
});

test("test_joint_evaluator_order_ascending", () => {
  const jointEvaluation = multisetFunction((x) => [evalAscending.evaluate(x), evalAuto.evaluate(x)]);

  const result = jointEvaluation.call(d6.pool(3));
  const expected = d6.rmatmul(3).map((x) => [x, x]);
  assert.ok(result.equals(expected));
});

test("test_joint_evaluator_order_descending", () => {
  const jointEvaluation = multisetFunction((x) => [evalDescending.evaluate(x), evalAuto.evaluate(x)]);

  const result = jointEvaluation.call(d6.pool(3));
  const expected = d6.rmatmul(3).map((x) => [x, x]);
  assert.ok(result.equals(expected));
});

test("test_enumerate_pool_vs_outer_product", () => {
  // ExpandEvaluator()(pool) is pool.expand().
  const result = d6.pool(3).expand();
  const expected = ip.vectorize(d6, d6, d6).map((x) => [...x].sort((a, b) => a - b));
  assert.ok(result.equals(expected));
});

testPools.forEach((pool, i) => {
  test(`test_expand_vs_sum[pool${i}]`, (t) => {
    if (pool.keepTuple().some((x) => x < 0)) {
      t.skip();
    } else {
      const result = pool.expand().map((x) => x.reduce((a, b) => a + b, 0));
      const expected = pool.sum();
      assert.ok(result.equals(expected));
    }
  });
});

test("test_contains_subset_vs_intersection_size", () => {
  const pool = d6.pool(10);
  const resultA = pool.issuperset([1, 2, 3, 3]);
  const resultB = pool.and([1, 2, 3, 3]).size().eq(4);
  assertDieEquals(resultA, resultB);
});

test("test_empty", () => {
  const result = d6.pool(1).and(d6.pool(1)).empty();
  assertDieEquals(result, d6.ne(d6));
  const resultAny = d6.pool(1).and(d6.pool(1)).any();
  assert.ok(pyEq(ip.ops.add(result.mean(), resultAny.mean()), 1));
});
