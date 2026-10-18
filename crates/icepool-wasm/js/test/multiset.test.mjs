// The parts of the API task C6d moved off the JSON-op bridge: pools (construction, the Map form, dPool / zPool,
// die.pool), keep generators and their keep family, multiset operators and count adjustments, keepOutcomes /
// dropOutcomes, pairing, mapCounts, mixtures, weightless generators, the built-in evaluations, str() and
// Pool.info. Expected values are Python icepool's. Each also checks that it made no call through the bridge.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Ellipsis, KeepGenerator, MultisetExpression, Order, Pool, d, pool } = ip;

/** Runs `f`, asserting that it called nothing through the bridge. */
function native(f) {
  const total = () => [...bridgeCalls.values()].reduce((a, b) => a + b, 0);
  const before = total();
  const result = f();
  assert.equal(total(), before, `bridge calls: ${JSON.stringify([...bridgeCalls])}`);
  return result;
}
const repr = (f) => native(() => f().repr());
const pythonType = (name) => (e) => e instanceof ip.IcepoolError && e.pythonType === name;
const d4 = d(4);
const d6 = d(6);

test("pools: construction, the Map form, dice put in zero times, info", () => {
  const [keepTuple, keepSize, rawSize, denominator, outcomes, dice] = native(() => d6.pool(3).highest(2).info());
  assert.deepEqual([keepTuple, keepSize, rawSize, denominator, outcomes], [[0, 1, 1], 2, 3, 216, [1, 2, 3, 4, 5, 6]]);
  assert.ok(dice[0][0].equals(d6) && dice[0][1] === 3);
  const p = native(() => new Pool(new Map([[d6, 2], [3, 1]])));
  assert.deepEqual(native(() => [p.keepTuple(), p.rawSize(), p.denominator(), p.outcomes()]), [[1, 1, 1], 3, 36n, [1, 2, 3, 4, 5, 6]]);
  // A die put in zero times still contributes its outcomes.
  assert.deepEqual(native(() => new Pool([d4, d(8)], [2, 0]).outcomes()), [1, 2, 3, 4, 5, 6, 7, 8]);
  assert.equal(repr(() => d6.pool([0, 1, 1]).sum()), "Die({2: 1, 3: 3, 4: 7, 5: 12, 6: 19, 7: 27, 8: 34, 9: 36, 10: 34, 11: 27, 12: 16})");
  assert.throws(() => new Pool(d6), pythonType("ValueError"));
  assert.throws(() => d6.pool(-1), pythonType("ValueError"));
  assert.throws(() => new Pool([ip.Again]), pythonType("TypeError"));
});

test("dPool and zPool", () => {
  assert.equal(repr(() => ip.dPool([4, 6]).sum()), "Die({2: 1, 3: 2, 4: 3, 5: 4, 6: 4, 7: 4, 8: 3, 9: 2, 10: 1})");
  assert.equal(repr(() => ip.zPool(new Map([[3, 2]])).sum()), "Die({0: 1, 1: 2, 2: 3, 3: 2, 4: 1})");
  // If empty, the pool will be considered to consist of zero zeros.
  assert.equal(repr(() => ip.dPool([]).sum()), "Die({0: 1})");
});

test("keeping: keep tuples, at, slices, middle", () => {
  assert.equal(repr(() => d6.pool(3).keep([-1, Ellipsis, 1]).sum()), "Die({0: 6, 1: 30, 2: 48, 3: 54, 4: 48, 5: 30})");
  assert.equal(repr(() => d6.pool(3).at(1)), "Die({1: 16, 2: 40, 3: 52, 4: 52, 5: 40, 6: 16})");
  assert.equal(repr(() => d4.pool(3).slice(1).sum()), "Die({2: 1, 3: 3, 4: 7, 5: 12, 6: 16, 7: 15, 8: 10})");
  assert.equal(repr(() => d4.pool(3).middle(1).sum()), "Die({1: 10, 2: 22, 3: 22, 4: 10})");
  assert.equal(repr(() => d4.pool(3).middle(2, { tie: "high" }).sum()), "Die({2: 1, 3: 3, 4: 7, 5: 12, 6: 16, 7: 15, 8: 10})");
  assert.throws(() => d4.pool(3).middle(2), pythonType("IndexError"));
  assert.throws(() => d4.pool(3).keep([1, 1]), pythonType("IndexError"));
  // On an expression, keeping is a `MultisetKeep` operator.
  assert.equal(repr(() => d4.pool(2).or(d4.pool(1)).slice(null, 1).sum()), "Die({1: 37, 2: 19, 3: 7, 4: 1})");
});

test("combining pools: merged pools, compound keeps, operators", () => {
  const merged = native(() => d6.pool(2).add(d4.pool(1)));
  assert.ok(merged instanceof Pool);
  assert.equal(repr(() => merged.highest(1).sum()), "Die({1: 1, 2: 7, 3: 19, 4: 37, 5: 36, 6: 44})");
  const compound = native(() => d4.pool(2).highest(1).add(d4.pool(1)));
  assert.ok(compound instanceof KeepGenerator && !(compound instanceof Pool));
  assert.equal(repr(() => compound.lowest(1).sum()), "Die({1: 19, 2: 21, 3: 17, 4: 7})");
  assert.equal(repr(() => d4.pool(3).sub(d4.pool(2)).size()), "Die({1: 256, 2: 564, 3: 204})");
  assert.equal(repr(() => d4.pool(2).and([1, 2]).expand()), "Die({(): 4, (1,): 5, (1, 2): 2, (2,): 5})");
  assert.equal(repr(() => pool([1]).rfloordiv([1, 1, 2])), "Die({2: 1})");
  // An Again expression on the right: its reflected operator, as in Python.
  assert.ok(native(() => d4.pool(1).add(ip.Again)) instanceof ip.AgainExpression);
});

test("count adjustments", () => {
  assert.deepEqual(native(() => d6.pool(2).neg().keepTuple()), [-1, -1]);
  assert.equal(repr(() => d4.pool(2).mul(2).sum()), "Die({4: 1, 6: 2, 8: 3, 10: 4, 12: 3, 14: 2, 16: 1})");
  assert.equal(repr(() => d4.pool(3).unique().size()), "Die({1: 4, 2: 36, 3: 24})");
  assert.equal(repr(() => d4.pool(3).keepCounts(">=", 2).size()), "Die({0: 24, 2: 36, 3: 4})");
  assert.equal(repr(() => d4.pool(3).keepCounts(">=", 2).floordiv(2).size()), "Die({0: 24, 1: 40})");
  assert.equal(repr(() => d4.pool(3).mod(2).size()), "Die({1: 40, 3: 24})");
  assert.throws(() => d4.pool(3).keepCounts("<>", 2), pythonType("ValueError"));
  assert.throws(() => d4.pool(3).mul(1.5), pythonType("TypeError"));
});

test("keepOutcomes, dropOutcomes, pairing, versus, mapCounts", () => {
  assert.equal(repr(() => d4.pool(2).keepOutcomes((x) => x % 2 === 0).size()), "Die({0: 4, 1: 8, 2: 4})");
  assert.equal(repr(() => d4.pool(2).dropOutcomes(pool([1, 2])).size()), "Die({0: 4, 1: 8, 2: 4})");
  assert.equal(repr(() => d4.pool(2).sortPair(">", d4.pool(2)).size()), "Die({0: 130, 1: 80, 2: 46})");
  assert.equal(repr(() => d4.pool(2).maxPairKeep(">", d4.pool(1), "high").size()), "Die({0: 30, 1: 34})");
  assert.equal(repr(() => d4.pool(2).maxPairDrop("==", [1]).size()), "Die({1: 7, 2: 9})");
  assert.equal(repr(() => d4.pool(2).versusAll(">", d4.pool(1)).size()), "Die({0: 30, 1: 20, 2: 14})");
  assert.equal(repr(() => d4.pool(2).versusAny(">", [2]).size()), "Die({0: 4, 1: 8, 2: 4})");
  const product = (o, a, b) => a * b;
  assert.equal(repr(() => MultisetExpression.mapCounts(d4.pool(2), [1, 1], { function: product }).size()), "Die({0: 9, 2: 6, 4: 1})");
  assert.throws(() => d4.pool(2).maxPairKeep("<", d4.pool(1), "middle"), pythonType("ValueError"));
});

test("a callback's own exception comes out of the evaluation as itself", () => {
  class Mine extends Error {}
  const boom = () => { throw new Mine("boom"); };
  assert.throws(() => d4.pool(2).keepOutcomes(boom).size(), (e) => e instanceof Mine);
  assert.throws(() => MultisetExpression.mapCounts(d4.pool(2), { function: boom }).sum(), (e) => e instanceof Mine);
  // A callback may call into the API again.
  const nested = (x) => d(x).maxOutcome() > 2;
  assert.equal(repr(() => d4.pool(2).keepOutcomes(nested).size()), "Die({0: 4, 1: 8, 2: 4})");
});

test("mixtures and weightless generators", () => {
  const inners = [d4.pool(1), d4.pool(2), d4.pool(1)];
  assert.equal(repr(() => ip.multisetMixture(inners).highest(1).sum()), "Die({1: 9, 2: 11, 3: 13, 4: 15})");
  assert.equal(native(() => d4.pool(3).weightless().expand().denominator()), 20n);
});

test("the built-in evaluations", () => {
  assert.equal(repr(() => d4.pool(2).productOfCounts()), "Die({0: 16})");
  assert.equal(repr(() => d4.pool(2).largestCountAndOutcome()),
    "Die({(1, 2): 2, (1, 3): 4, (1, 4): 6, (2, 1): 1, (2, 2): 1, (2, 3): 1, (2, 4): 1})");
  assert.equal(repr(() => d4.pool(2).largestStraightAndOutcome("low")),
    "Die({(1, 1): 5, (1, 2): 3, (1, 3): 1, (1, 4): 1, (2, 1): 2, (2, 2): 2, (2, 3): 2})");
  assert.equal(repr(() => pool([1, 1, 3]).allCounts("all")), "Die({(2, 1): 1})");
  assert.equal(repr(() => d4.pool(3).countSubset([1], { emptyDivisor: 0 })), "Die({0: 27, 1: 27, 2: 9, 3: 1})");
  assert.equal(repr(() => pool([1]).lt(d4.pool(2))), "Die({False: 9, True: 7})");
  assert.equal(repr(() => d4.pool(2).isdisjoint(d4.pool(1))), "Die({False: 28, True: 36})");
  assert.equal(repr(() => pool([1, 3, 2]).expand(Order.Descending)), "Die({(3, 2, 1): 1})");
  assert.throws(() => d4.pool(2).largestStraightAndOutcome("middle"), pythonType("ValueError"));
});

test("argsort and the lexicographic comparisons", () => {
  assert.equal(repr(() => pool([1, 3]).argsort([2], { tie: "drop" })), "Die({(0, 1, 0): 1})");
  assert.equal(repr(() => pool([1, 3]).argsort([2], [3], { tie: "left" })), "Die({(0, 2, 1, 0): 1})");
  assert.equal(repr(() => pool([1, 3]).argsortGrouped([3], [2])), "Die({((0, 1), (2,), (0,)): 1})");
  assert.equal(repr(() => pool([1, 3]).leximin("<", [1, 4])), "Die({1: 1})");
  assert.equal(repr(() => pool([1, 3]).leximax("cmp", [1, 4])), "Die({-1: 1})");
  assert.equal(native(() => d6.pool(2).leximax(">", d6.pool(2)).mean().toString()), "205/432");
  assert.throws(() => pool([1]).argsort([2], { tie: "middle" }), pythonType("ValueError"));
  assert.throws(() => pool([1]).leximin("<>", [2]), pythonType("ValueError"));
});

test("str() of pools and expressions", () => {
  assert.equal(native(() => String(pool([1, 2]))), "Pool of 2 dice with keep_tuple=(1, 1)\n  Die({1: 1}) : 1,\n  Die({2: 1}) : 1,\n");
  const union = () => pool([1]).or(pool([2]));
  const unionStr = "(Pool of 1 dice with keep_tuple=(1,)\n  Die({1: 1}) : 1,\n | Pool of 1 dice with keep_tuple=(1,)\n  Die({2: 1}) : 1,\n)";
  assert.equal(native(() => String(union().unique(2).keepCounts("==", 1).slice(null, 1))),
    `${unionStr}.unique(2).keep_counts('==', 1)[slice(None, 1, None)]`);
  assert.equal(native(() => String(union().keep([1, Ellipsis]))), `${unionStr}[(1, Ellipsis)]`);
});

test("a native expression handed to the bridge is made there once, by replaying its method", () => {
  // `Deal` is still the bridge's: an operator over a deal and a pool goes through it.
  const p = d4.pool(2).highest(1);
  const deal = new ip.Deck([1, 2, 3, 4]).deal(1);
  assert.equal(p.add(deal).sum().repr(), d4.pool(2).highest(1).add(new ip.Deck([1, 2, 3, 4]).deal(1)).sum().repr());
  const before = bridgeCalls.get("pool_highest") ?? 0;
  p.add(deal).size();
  assert.equal(bridgeCalls.get("pool_highest") ?? 0, before);
});
