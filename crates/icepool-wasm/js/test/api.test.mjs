// The JS API itself: what the fixtures cannot show (JS types at the boundary, callbacks, exceptions, evaluators
// with object states, garbage collection). Expected values are Python icepool's (computed with the oracle).

import { test } from "node:test";
import assert from "node:assert/strict";
import v8 from "node:v8";
import vm from "node:vm";
import * as ip from "../node.js";

const { d, Die, Pool, Deck, map, Again, Reroll, Restart, Break, Float, Fraction, Vector, Symbols, Ellipsis } = ip;

test("quantities are BigInts, exact beyond 2**53", () => {
  assert.deepEqual(d(6).add(d(6)).items().slice(0, 3), [[2, 1n], [3, 2n], [4, 3n]]);
  const big = Die.sum(30, d(6));
  assert.equal(big.denominator(), 6n ** 30n);
  assert.equal(big.quantity(105), 9378595792117360310832n);
  assert.equal(big.quantity(30), 1n);
  assert.equal(big.values().reduce((a, b) => a + b), 6n ** 30n);
  assert.equal(typeof big.modalQuantity(), "bigint");
});

test("ints beyond 2**53 are BigInts both ways; safe ones are numbers", () => {
  const huge = 2n ** 70n;
  const die = new Die([huge, 1]);
  assert.deepEqual(die.outcomes(), [1, huge]);
  assert.deepEqual(die.add(huge).outcomes(), [huge + 1n, 2n * huge]);
  assert.equal(new Die([2 ** 53 + 2]).outcomes()[0], 2n ** 53n + 2n);
});

test("floats keep their type: integral ones come out as Float", () => {
  const half = d(4).truediv(2);
  assert.deepEqual(half.outcomes().map(Number), [0.5, 1, 1.5, 2]);
  assert.ok(half.outcomes()[1] instanceof Float);
  // Going back in, a Float is a float: Die([1.0]) is not Die([1]) (its repr says so).
  assert.equal(new Die([new Float(1)]).repr(), "Die({1.0: 1})");
  assert.equal(new Die([1]).repr(), "Die({1: 1})");
  // Through a callback too: the identity map keeps every outcome's type.
  assert.ok(half.map((x) => x).equals(half));
  assert.equal(new Die([-0.5]).round().repr(), "Die({0: 1})");
  assert.equal(new Die([new Float(-0)]).repr(), "Die({-0.0: 1})");
  assert.equal(ip.pyFloatRepr(1e16), "1e+16");
  assert.equal(ip.pyFloatRepr(1.5e-5), "1.5e-05");
  assert.equal(ip.pyFloatRepr(123.25), "123.25");
});

test("statistics: Fractions where Python gives Fractions", () => {
  const m = d(6).mean();
  assert.ok(m instanceof Fraction);
  assert.deepEqual([m.numerator, m.denominator], [7n, 2n]);
  assert.equal(Number(m), 3.5);
  assert.equal(d(6).sd(), Math.sqrt(35 / 12));
  assert.deepEqual(d(6).probability(">=", 5), new Fraction(1n, 3n));
  assert.equal(d(6).probability(">=", 5, { percent: true }), 33.33333333333333);
  assert.deepEqual(d(6).quantities("<="), [1n, 2n, 3n, 4n, 5n, 6n]);
});

test("map takes JS callbacks: extra args, star inference, sentinels, repeat, kwargs", () => {
  assert.deepEqual(map((a, b) => a * b, d(2), d(2)).items(), [[1, 1n], [2, 2n], [4, 1n]]);
  assert.deepEqual(d(3).map((x, k) => x + k, 10).outcomes(), [11, 12, 13]);
  // (a, b) => ... on a die of pairs: star is inferred, as Python's infer_star does.
  const pairs = new Die([[1, 2], [3, 4]]);
  assert.deepEqual(pairs.map((a, b) => b - a).items(), [[1, 2n]]);
  assert.deepEqual(pairs.map((t) => t[0], { star: false }).outcomes(), [1, 3]);
  assert.deepEqual(d(4).map((x) => (x === 1 ? Reroll : x)).outcomes(), [2, 3, 4]);
  // A callback may return dice and tuples holding dice.
  assert.deepEqual(d(2).map((x) => (x === 2 ? d(2) : x)).items(), [[1, 3n], [2, 1n]]);
  assert.deepEqual(d(2).map((x) => [x, d(2)]).denominator(), 4n);
  // repeat: an absorbing chain (Break stops a state).
  const capped = map((total, roll) => (total >= 3 ? new Break() : total + roll), 0, d(2), { repeat: Infinity });
  assert.deepEqual(capped.outcomes(), [3, 4]);
  assert.deepEqual(capped.items(), [[3, 5n], [4, 3n]]);
  // Other options are keyword arguments, given to the callback as a trailing object. Give that parameter a
  // default, so that `f.length` (what star inference counts, as Python counts positional parameters) is 1.
  assert.deepEqual(map((x, { k } = {}) => x + k, d(2), { k: 5 }).outcomes(), [6, 7]);
  // Python's Restart outside repeat drops out like Reroll.
  assert.deepEqual(d(3).map((x) => (x < 2 ? Restart : x)).outcomes(), [2, 3]);
});

test("predicates and keys are JS functions too", () => {
  assert.deepEqual(d(6).reroll((x) => x <= 2, { depth: Infinity }).outcomes(), [3, 4, 5, 6]);
  assert.deepEqual(d(6).reroll([1, 2], { depth: 1 }).quantities(), [2n, 2n, 8n, 8n, 8n, 8n]);
  const [low, high] = d(6).split((x) => x <= 3);
  assert.deepEqual([low.outcomes(), high.outcomes()], [[1, 2, 3], [4, 5, 6]]);
  const groups = d(6).groupBy((x) => x % 2);
  assert.deepEqual([...groups.keys()], [1, 0]);
  assert.deepEqual(groups.get(0).outcomes(), [2, 4, 6]);
  assert.equal(d(6).quantityWhere((x) => x > 4), 2n);
  assert.deepEqual(d(6).explode([6], { depth: 1 }).maxOutcome(), 12);
});

test("errors: Python exceptions as IcepoolError, JS exceptions rethrown as they were", () => {
  assert.throws(() => d(0), (e) => e instanceof ip.IcepoolError && e.pythonType === "ValueError");
  assert.throws(() => d(6).add("a"), (e) => e.pythonType === "TypeError");
  assert.throws(() => d(6).reroll([1]), (e) => e.pythonType === "TypeError"); // depth is required
  const mine = new RangeError("mine");
  assert.throws(() => d(6).map(() => { throw mine; }), (e) => e === mine);
  // Thrown deep inside an evaluation, and after a nested call into the core.
  const ev = ip.evaluator({ nextState: () => { d(2).add(1); throw mine; } });
  assert.throws(() => ev.evaluate([1, 2]), (e) => e === mine);
  // The core is still fine afterwards.
  assert.equal(d(6).denominator(), 6n);
});

test("evaluators in JS: states, Reroll, die outcomes, kwargs, extraOutcomes, UnsupportedOrder", () => {
  class SumOfSquares extends ip.MultisetEvaluator {
    initialState() { return 0; }
    nextState(state, order, outcome, count) { return state + outcome * outcome * count; }
  }
  assert.deepEqual(new SumOfSquares().evaluate(new Pool([d(2), d(2)])).items(), [[2, 1n], [5, 2n], [8, 1n]]);

  // Object states are keyed by their JSON: equal objects merge.
  const counter = ip.evaluator({
    nextState: (s, order, outcome, count) => ({ n: (s?.n ?? 0) + count, seen: (s?.seen ?? 0) + (count > 0 ? 1 : 0) }),
    finalOutcome: (s) => [s.n, s.seen],
  });
  assert.deepEqual(counter.evaluate(new Pool([d(3), d(3)])).items(), [[[2, 1], 3n], [[2, 2], 6n]]);

  // Reroll from nextState; a die as the final outcome.
  const noDoubles = ip.evaluator({
    nextState: (s, order, outcome, count) => (count > 1 ? Reroll : (s ?? 0) + count),
    finalOutcome: (s) => d(s),
  });
  assert.deepEqual(noDoubles.evaluate(new Pool([d(3), d(3)])).items(), [[1, 3n], [2, 3n]]);

  // evaluate()'s options reach initialState / finalOutcome, after the sizes.
  const scaled = ip.evaluator({
    initialState: (order, outcomes, size, { start }) => start,
    nextState: (s, order, outcome, count) => s + count,
    finalOutcome: (s, order, outcomes, size, { scale }) => s * scale,
  });
  assert.deepEqual(scaled.evaluate([1, 2, 2], { start: 1, scale: 10 }).items(), [[40, 1n]]);

  // extraOutcomes: the gaps between the outcomes are seen too.
  const gaps = ip.evaluator({
    extraOutcomes: (outcomes) => {
      const out = [];
      for (let i = outcomes[0]; i <= outcomes[outcomes.length - 1]; i++) out.push(i);
      return out;
    },
    nextState: (s, order, outcome, count) => (s ?? 0) + (count === 0 ? 1 : 0),
  });
  assert.deepEqual(gaps.evaluate([1, 4]).items(), [[2, 1n]]);

  // UnsupportedOrder makes the driver try the other order.
  const descendingOnly = ip.evaluator({
    initialState(order) {
      if (order > 0) throw new ip.UnsupportedOrder();
      return [];
    },
    nextState: (s, order, outcome, count) => (count > 0 ? [...s, outcome] : s),
  });
  assert.deepEqual(descendingOnly.evaluate([1, 2, 3]).outcomes(), [[3, 2, 1]]);
});

test("Again, pools, decks", () => {
  assert.deepEqual(new Die([1, 2, Again.add(2)], { againDepth: 1 }).items(), [[1, 3n], [2, 3n], [3, 1n], [4, 2n]]);
  const pool = new Pool([d(6), d(6), d(6)]);
  assert.deepEqual(pool.highest(2).sum().quantity(12), 16n);
  assert.deepEqual(pool.keep([Ellipsis, 1]).sum().equals(d(6).highest(3, 1)), true);
  assert.deepEqual(pool.at(-1).equals(d(6).highest(3, 1)), true);
  assert.deepEqual(pool.add([1, 1]).size().outcomes(), [5]);
  assert.deepEqual(ip.dPool([4, 6]).sum().equals(d(4).add(d(6))), true);
  const deck = new Deck([1, 2, 3], 2);
  assert.equal(deck.size(), 6n);
  assert.deepEqual(deck.deal(2).sum().items(), [[2, 1n], [3, 4n], [4, 5n], [5, 4n], [6, 1n]]);
  assert.deepEqual(deck.deal([1, 1]).hand(0).sum().items(), [[1, 10n], [2, 10n], [3, 10n]]);
  assert.equal(ip.highest(d(6), d(8)).maxOutcome(), 8);
});

test("Vector and Symbols", () => {
  const v = new Vector([1, 2]).add(new Vector([10, 20]));
  assert.deepEqual(v.items, [11, 22]);
  assert.equal(ip.truth(new Vector([1, 2]).lt(new Vector([1, 3]))), true);
  assert.deepEqual(ip.vectorize(d(2), 0).outcomes().map((x) => x.items), [[1, 0], [2, 0]]);
  const s = new Symbols("aab").add(new Symbols("b"));
  assert.equal(s.toString(), "aabb");
  assert.equal(s.get("a"), 2);
  assert.deepEqual(new Symbols("ab").sub(new Symbols("b")).keys(), ["a", "b"]); // zero counts stay keys
});

test("DieWithTruth: eq/ne give a die and a truth value", () => {
  const eq = d(6).eq(d(6));
  assert.ok(eq instanceof ip.DieWithTruth);
  assert.equal(eq.truth(), true);
  assert.deepEqual(eq.items(), [[false, 30n], [true, 6n]]);
  assert.equal(d(6).ne(d(6)).truth(), false);
});

test("handles are released when their JS objects are collected", async () => {
  v8.setFlagsFromString("--expose-gc");
  const gc = vm.runInNewContext("gc");
  const before = ip.liveHandles();
  (() => {
    for (let i = 0; i < 2000; i++) d(6).add(i);
  })();
  for (let i = 0; i < 10 && ip.liveHandles() > before + 100; i++) {
    gc();
    await new Promise((r) => setTimeout(r, 10));
  }
  assert.ok(ip.liveHandles() < before + 100, `${ip.liveHandles() - before} handles still live`);
});
