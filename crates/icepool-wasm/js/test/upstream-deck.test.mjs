// Translates upstream icepool tests/deck_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const { MultisetEvaluator, Deck } = ip;
const range = (n) => Array.from({ length: n }, (_, i) => i);

/** Python's `deck == other`: a Deck compares as the Mapping of its outcomes to quantities. */
const assertDeckEquals = (actual, expected) => assert.deepEqual(actual.items(), expected.items());

class TrivialEvaluator extends MultisetEvaluator {
  nextState(state, order, outcome, ...counts) {
    return 0;
  }
}

// A MultiDeal's counts arrive as one tuple (array).
class SumTupleEvaluator extends MultisetEvaluator {
  nextState(state, order, outcome, counts) {
    if (state === null) {
      return counts.map(() => 0);
    } else {
      return state.map((x, i) => x + outcome * counts[i]);
    }
  }
}

test("test_empty_deal", () => {
  const deal = new Deck(range(13), 4).deal([]);
  const result = new TrivialEvaluator().evaluate(deal);
  assert.ok(result.equals(new ip.Die([0])));
});

test("test_poker_straight", () => {
  const deal = new Deck(range(13), 4).deal(5);
  // best_run_evaluator: LargestStraightEvaluator() (no wraparound).
  const result = deal.largestStraight().eq(5);
  assert.ok(result.equals(ip.coin(9216, 2598960)));
});

test("test_two_hand_sum_same_size", () => {
  const deck = new Deck(range(4), 4);
  const deal1 = deck.deal(5);
  const result1 = deal1.sum();

  const deal2 = deck.deal([5, 5]);
  const result2 = new SumTupleEvaluator().evaluate(deal2);

  assert.equal(deal2.denominator(), result2.denominator());
  assert.ok(result1.equals(result2.marginals()[0], { simplify: true }));
  assert.ok(result2.marginals()[0].equals(result2.marginals()[1]));
});

test("test_two_hand_sum_diff_size", () => {
  const deck = new Deck(range(4), 4);

  const deal = deck.deal([2, 4]);
  const result = new SumTupleEvaluator().evaluate(deal);

  assert.equal(deal.denominator(), result.denominator());
  assert.ok(pyEq(result.marginals()[0].mul(2).mean(), result.marginals()[1].mean()));
});

test("test_add", () => {
  const deck = new Deck([1, 2, 3]).add(new Deck([3, 4]));
  assertDeckEquals(deck, new Deck([1, 2, 3, 3, 4]));
});

test("test_sub", () => {
  const deck = new Deck([1, 2, 3]).sub(new Deck([3, 4]));
  assertDeckEquals(deck, new Deck([1, 2]));
});

test("test_and", () => {
  const deck = new Deck([1, 2, 3]).and(new Deck([3, 4]));
  assertDeckEquals(deck, new Deck([3]));
});

test("test_or", () => {
  const deck = new Deck([1, 2, 3]).or(new Deck([3, 4]));
  assertDeckEquals(deck, new Deck([1, 2, 3, 4]));
});

test("test_mul", () => {
  const deck = new Deck([1, 2, 3]).mul(2);
  assertDeckEquals(deck, new Deck([1, 1, 2, 2, 3, 3]));
});

test("test_floordiv", () => {
  const deck = new Deck([1, 1, 1, 2, 2, 3]).floordiv(2);
  assertDeckEquals(deck, new Deck([1, 2]));
});

test("test_highest", () => {
  const deck = new Deck(range(10));
  const result = deck.deal(4).highest(2).expand().simplify();
  const expected = deck.deal(4).expand().map((x) => x.slice(2)).simplify();
  assertDieEquals(result, expected);
});

test("test_lowest", () => {
  const deck = new Deck(range(10));
  const result = deck.deal(4).lowest(2).expand().simplify();
  const expected = deck.deal(4).expand().map((x) => x.slice(0, -2)).simplify();
  assertDieEquals(result, expected);
});

test("test_append", () => {
  const base = new Deck([1, 2, 2, 3, 3, 3]);
  assertDeckEquals(base.append(1), new Deck([1, 1, 2, 2, 3, 3, 3]));
  assertDeckEquals(base.append(0), new Deck([0, 1, 2, 2, 3, 3, 3]));
  assertDeckEquals(base.append(1, 3), new Deck([1, 1, 1, 1, 2, 2, 3, 3, 3]));
  assertDeckEquals(base.append(0, 3), new Deck([0, 0, 0, 1, 2, 2, 3, 3, 3]));
});

test("test_remove", () => {
  const base = new Deck([1, 2, 2, 3, 3, 3]);
  assertDeckEquals(base.remove(0), base);
  assertDeckEquals(base.remove(3), new Deck([1, 2, 2]));
  assertDeckEquals(base.remove(3, 1), new Deck([1, 2, 2, 3, 3]));
  assertDeckEquals(base.remove(1, 3), new Deck([2, 2, 3, 3, 3]));
});
