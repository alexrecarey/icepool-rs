// Translates upstream tests/statistics_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, raises, pyEq } from "./upstream.mjs";

const { Die, Deck, Fraction } = ip;
const d6 = ip.d(6);

test("test_d6_mean", () => {
  assert.ok(pyEq(d6.mean(), 3.5));
});

test("test_d8_variance", () => {
  assert.ok(pyEq(ip.d(8).variance(), 63 / 12));
});

test("test_kolmogorov_smirnov_standard_dice", () => {
  assert.ok(pyEq(ip.d(10).kolmogorovSmirnov(ip.d(20)), new Fraction(1, 2)));
});

test("test_kolmogorov_smirnov_flat_number", () => {
  assert.ok(pyEq(new Die([10]).kolmogorovSmirnov(new Die([10])), 0));
  assert.ok(pyEq(new Die([10]).kolmogorovSmirnov(new Die([9])), 1));
});

test("test_d6_median", () => {
  assert.ok(pyEq(d6.medianLow(), 3));
  assert.ok(pyEq(d6.medianHigh(), 4));
  assert.ok(pyEq(d6.median(), 3.5));
});

test("test_d7_median", () => {
  const d7 = ip.d(7);
  assert.ok(pyEq(d7.medianLow(), 4));
  assert.ok(pyEq(d7.medianHigh(), 4));
  assert.ok(pyEq(d7.median(), 4));
});

test("test_min_quantile", () => {
  assert.ok(pyEq(d6.quantileLow(0), 1));
  assert.ok(pyEq(d6.quantileHigh(0), 1));
});

test("test_max_quantile", () => {
  assert.ok(pyEq(d6.quantileLow(100), 6));
  assert.ok(pyEq(d6.quantileHigh(100), 6));
});

test("test_entropy_with_zeros", () => {
  assert.ok(pyEq(new Die(new Map([[1, 1n], [2, 0n], [3, 1n]])).entropy(), 1.0));
});

for (const comparison of ["==", "!=", "<=", "<", ">=", ">"]) {
  test(`test_percent[${comparison}]`, () => {
    const die = d6.rmatmul(3);
    // Python: Fraction * 100.0 is a float, as Number(...) * 100 is here.
    assert.equal(
      Number(die.probability(comparison, 4, { percent: true })),
      Number(die.probability(comparison, 4)) * 100.0,
    );
  });
}

// Python's Deck == Deck compares the cards and their counts.
const sameDeck = (a, b) => assert.deepEqual(a.items(), b.items());

test("test_pad_to_denominator_add", () => {
  const deck = new Deck([0, 1, 2, 3]).padToDenominator(6, 0);
  sameDeck(deck, new Deck([0, 0, 0, 1, 2, 3]));
});

test("test_pad_to_denominator_remove", () => {
  const deck = new Deck([0, 0, 0, 1, 2, 3]).padToDenominator(4, 0);
  sameDeck(deck, new Deck([0, 1, 2, 3]));
});

test("test_pad_to_denominator_zero", () => {
  const deck = new Deck([0, 0, 0, 1, 2, 3]).padToDenominator(3, 0);
  sameDeck(deck, new Deck([1, 2, 3]));
});

test("test_pad_to_denominator_negative_error", () => {
  raises(() => new Deck([0, 0, 0, 1, 2, 3]).padToDenominator(2, 0), "ValueError");
});
