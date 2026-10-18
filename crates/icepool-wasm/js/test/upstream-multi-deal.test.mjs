// Translates upstream icepool tests/multi_deal_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, pyEq } from "./upstream.mjs";

const { Deck, multisetFunction } = ip;
const range = (n) => Array.from({ length: n }, (_, i) => i);

test("test_bare_expression", () => {
  const deck = new Deck(range(6), 4);
  assert.ok(deck.deal(3).expand().equals(deck.deal([3, 4]).hand(0).expand(), { simplify: true }));
});

test("test_intersection_size", () => {
  const intersectionSize = multisetFunction((hands) => {
    const [a, b] = hands;
    return a.and(b).size();
  });
  const deck = new Deck(range(20));
  assert.ok(pyEq(intersectionSize.call(deck.deal([5, 5])).probability(0), 1));
});

test("test_union_size", () => {
  const unionSize = multisetFunction((hands) => {
    const [a, b] = hands;
    return a.or(b).size();
  });
  const deck = new Deck(range(20));
  assert.ok(pyEq(unionSize.call(deck.deal([5, 5])).probability(10), 1));
});
