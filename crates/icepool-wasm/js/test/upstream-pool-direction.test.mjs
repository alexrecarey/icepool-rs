// Translates upstream icepool tests/pool_direction_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, raises } from "./upstream.mjs";

const { MultisetEvaluator, Order, pool } = ip;
const [d6, d8] = [ip.d(6), ip.d(8)];

test("test_force_order_conflict", () => {
  raises(() => d6.pool(1).forceOrder(Order.Ascending).forceOrder(Order.Descending).sum(), "ConflictingOrderError");
});

class LastOutcome extends MultisetEvaluator {
  nextState(state, order, outcome, count) {
    return outcome;
  }
}

test("test_order_ascending", () => {
  assert.equal(new LastOutcome().evaluate(d6.pool(1).forceOrder(Order.Ascending)).probability(6).toString(), "1");
});

test("test_order_descending", () => {
  assert.equal(new LastOutcome().evaluate(d6.pool(1).forceOrder(Order.Descending)).probability(1).toString(), "1");
});

// The auto order should maximize skips if there are no other considerations.
// Note that this is the *opposite* of the preferred pop order.
test("test_auto_order_uniform", () => {
  assert.equal(new LastOutcome().evaluate(d6.pool([0, 0, 1, 1])).probability(6).toString(), "1");
  assert.equal(new LastOutcome().evaluate(d6.pool([1, 1, 0, 0])).probability(1).toString(), "1");
});

// Above that, the auto order should favor the wide-to-narrow ordering.
test("test_auto_order_max_truncate_min", () => {
  assert.equal(new LastOutcome().evaluate(pool([d8, d6, d6, d6]).keep([1, 1, 1, 0])).probability(8).toString(), "1");
  assert.equal(new LastOutcome().evaluate(pool([d8, d6, d6, d6]).keep([0, 1, 1, 1])).probability(8).toString(), "1");
});
