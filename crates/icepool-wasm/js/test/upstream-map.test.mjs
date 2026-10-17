// Translates upstream tests/map_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, approx, assertDieEquals, pyEq } from "./upstream.mjs";

const { Die, Deck, Fraction, d, coin } = ip;
const d6 = d(6);
const d10 = d(10);

/** Python's `a == b` between dice or decks, as a boolean. */
const dieEq = (a, b) => a.eq(b).truth();
/** Python's `a == b` between decks: same items. */
const deckEq = (a, b) => assert.deepEqual(a.items(), b.items());

const expectedD6x1 = new Die([...Array(6).fill([1, 2, 3, 4, 5]).flat(), 7, 8, 9, 10, 11, 12]);

test("test_map_dict", () => {
  const die = d6.map(new Map([[6, d6.add(6)]]));
  approx(die.probabilities(), expectedD6x1.probabilities());
});

test("test_map_func", () => {
  const die = d6.map((x) => (x === 6 ? d6.add(6) : 6 - x));
  approx(die.probabilities(), expectedD6x1.probabilities());
});

test("test_map_mix", () => {
  const result = d6.map((x) => (x >= 3 ? x : ip.Reroll));
  const expected = d(4).add(2);
  assertDieEquals(result, expected);
});

test("test_map_depth", () => {
  const result = d(8).sub(1).map((x) => Math.floor(x / 2), { repeat: 2 }).simplify();
  const expected = d(2).sub(1);
  assertDieEquals(result, expected);
});

test("test_map_star", () => {
  const a = new Die([[0, 0]]).map((x, y) => [x, y], { repeat: 1 });
  const b = new Die([[0, 0]]).map((x, y) => [x, y], { repeat: 2 });
  const c = new Die([[0, 0]]).map((x, y) => [x, y], { repeat: Infinity });
  assert.ok(dieEq(a, b));
  assert.ok(dieEq(b, c));
});

function collatz(x) {
  if (x === 1) return 1;
  else if (x % 2) return x * 3 + 1;
  else return Math.floor(x / 2);
}

test("test_map_fixed_point", () => {
  // Collatz conjecture.
  const result = d(100).map(collatz, { repeat: Infinity }).simplify();
  const expected = new Die([1]);
  assertDieEquals(result, expected);
});

test("test_map_fixed_point_1_cycle", () => {
  function repl(outcome) {
    if (outcome >= 10) return outcome;
    return new Die([0, 1]).radd(outcome);
  }
  const result = new Die([0]).map(repl, { repeat: Infinity }).simplify();
  assertDieEquals(result, new Die([10]));
});

test("test_map_and_time", () => {
  // How many coin flips until two heads?
  const initial = new Die([0]);
  const result = initial.mapAndTime((x) => (x >= 2 ? x : coin(1, 2).radd(x)), { repeat: 100 });
  approx(Number(result.marginal(1).mean()), 4.0);
});

test("test_mean_time_to_absorb", () => {
  const initial = new Die([0]);
  const result = initial.meanTimeToAbsorb((x) => (x >= 2 ? x : coin(1, 2).radd(x)));
  assert.ok(pyEq(result, 4));
});

test("test_random_walk", () => {
  function repl(x) {
    if (Math.abs(x) >= 2) return x;
    return new Die([x - 1, x + 1]);
  }
  const result = new Die([0]).map(repl, { repeat: Infinity }).simplify();
  assertDieEquals(result, new Die([-2, 2]));
});

test("test_random_walk_extra_arg", () => {
  function repl(x, step) {
    if (Math.abs(x) >= 2) return x;
    else return x + step;
  }
  const result = ip.map(repl, 0, new Die([-1, 1]), { repeat: Infinity }).simplify();
  const result2 = new Die([0]).map(repl, new Die([-1, 1]), { repeat: Infinity }).simplify();
  const expected = new Die([-2, 2]);
  assert.ok(dieEq(result, expected));
  assert.ok(dieEq(result2, expected));
});

test("test_random_walk_biased", () => {
  function repl(x) {
    if (Math.abs(x) >= 2) return x;
    return new Die([x - 1, x + 1], [1, 2]);
  }
  const result = new Die([0]).map(repl, { repeat: Infinity }).simplify();
  assertDieEquals(result, new Die([-2, 2], [1, 4]));
});

test("test_random_walk_biased_extra_arg", () => {
  function repl(x, step) {
    if (Math.abs(x) >= 2) return x;
    else return x + step;
  }
  const result = ip.map(repl, 0, new Die([-1, 1, 1]), { repeat: Infinity }).simplify();
  const result2 = new Die([0]).map(repl, new Die([-1, 1, 1]), { repeat: Infinity }).simplify();
  const expected = new Die([-2, 2], [1, 4]);
  assert.ok(dieEq(result, expected));
  assert.ok(dieEq(result2, expected));
});

test("test_is_in", () => {
  const result = Die.sum(2, d6).isIn([2, 12]);
  const expected = coin(2, 36);
  assertDieEquals(result, expected);
});

test("test_count", () => {
  const result = d6.count(2, [2, 4]);
  const expected = Die.sum(2, coin(2, 6));
  assertDieEquals(result, expected);
});

const range = (n) => [...Array(n).keys()];

test("test_deck_map", () => {
  const result = new Deck(range(13)).map((x) => x * 2).deal(2).sum();
  const expected = new Deck(range(13)).deal(2).sum().mul(2);
  assertDieEquals(result, expected);
});

test("test_deck_map_size_increase", () => {
  // Not recommended, but technically well-formed.
  const result = new Deck(range(13)).map(new Map([[12, new Deck(range(12))]]));
  const expected = new Deck(range(12), 2);
  deckEq(result, expected);
});

test("test_mean_time_to_sum_d6", () => {
  const cdf = [];
  for (let i = 0; i < 11; i++) cdf.push(Die.sum(i, d6).ge(10));
  const expected = ip.fromCumulative(range(11), cdf).mean();
  assert.ok(pyEq(d6.meanTimeToSum(10), expected));
});

test("test_time_to_sum_d6", () => {
  assert.ok(pyEq(d6.meanTimeToSum(10), d6.timeToSum(10, 11).mean()));
});

test("test_time_to_sum_d6_auto_max_time", () => {
  assert.ok(pyEq(d6.meanTimeToSum(10), d6.timeToSum(10).mean()));
});

test("test_mean_time_to_sum_z6", () => {
  const cdf = [];
  for (let i = 0; i < 11; i++) cdf.push(Die.sum(i, d(5)).ge(10));
  const expected = ip.ops.mul(ip.fromCumulative(range(11), cdf).mean(), new Fraction(6, 5));
  assert.ok(pyEq(d6.sub(1).meanTimeToSum(10), expected));
});

test("test_mean_time_to_sum_coin", () => {
  assert.ok(pyEq(coin(1, 2).meanTimeToSum(10), 20));
});

test("test_fractional_coin", () => {
  assert.ok(dieEq(coin(new Fraction(1, 3)), d(3).eq(1)));
});

test("test_stochastic_round", () => {
  assert.ok(pyEq(Die.sum(6, d6).truediv(2).stochasticRound().mean(), 10.5));
  assert.ok(pyEq(Die.sum(6, d6).truediv(new Fraction(3)).stochasticRound().mean(), 7));
});

test("test_map_and_time_extra_args", () => {
  function testFunction(current, roll) {
    return Math.min(current + roll, 10);
  }
  const result = new Die([0]).mapAndTime(testFunction, d6, { repeat: 10 });
  assert.ok(pyEq(result.marginal(1).mean(), d6.meanTimeToSum(10)));
});

test("test_map_and_time_extra_args_with_self_loops", () => {
  function testFunction(current, roll) {
    return Math.min(current + roll, 10);
  }
  const result = new Die([0]).mapAndTime(testFunction, d6.sub(1), { repeat: 100 });
  approx(result.marginal(1).mean(), d6.sub(1).meanTimeToSum(10));
});

test("test_map_time_limit", () => {
  function testFunction(current, roll) {
    return Math.min(current + roll, 10);
  }
  assert.ok(dieEq(
    new Die([0]).map(testFunction, d6, { repeat: 10 }),
    new Die([0]).map(testFunction, d6, { repeat: 20 }),
  ));
});

test("test_group_by", () => {
  const result = d10.groupBy((x) => x % 3);
  assert.equal(result.size, 3);
  assert.ok(dieEq(result.get(0), new Die([3, 6, 9])));
  assert.ok(dieEq(result.get(1), new Die([1, 4, 7, 10])));
  assert.ok(dieEq(result.get(2), new Die([2, 5, 8])));
});

test("test_group_by_index", () => {
  const initial = new Die(["aardvark", "alligator", "asp", "blowfish", "cat", "crocodile"]);
  const result = initial.groupByIndex(0);
  assert.ok(dieEq(result.get("a"), new Die(["aardvark", "alligator", "asp"])));
  assert.ok(dieEq(result.get("b"), new Die(["blowfish"])));
  assert.ok(dieEq(result.get("c"), new Die(["cat", "crocodile"])));
});

test("test_kwargs", () => {
  // Python's keyword-only `die` arrives as a trailing options object (a default keeps f.length at 1).
  function testFn(x, { die } = {}) {
    return Die.sum(x, die);
  }
  assert.ok(dieEq(d6.map(testFn, { die: d6 }), Die.sum(d6, d6)));
});
