// The parts of the API task C6c moved off the JSON-op bridge onto typed exports: Again and `Die(...)`'s argument
// handling, explode / ifElse / map and its repeated forms, the free functions, sampling, Vector and Symbols
// methods. Expected values are Python icepool's. Each also checks that it made no call through the bridge.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Again, Die, Reroll, Vector, Symbols, d } = ip;

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

test("Again: a Map with an Again key, times, count mode (upstream's test_owod and friends)", () => {
  const owod = native(() => new Die(new Map([[0, 7], [1, 2], [Again.add(1), 1]]), { againDepth: 3 }));
  assert.deepEqual(owod.items(), [[0, 7000n], [1, 2700n], [2, 270n], [3, 27n], [4, 3n]]);
  assert.equal(repr(() => new Die([1, Again.add(1)], [2, 1], { againDepth: 1 })), "Die({1: 6, 2: 3})");
  assert.equal(repr(() => new Die(new Map([[1, 2], [Again.add(1), 1]]), { againCount: 2 })), "Die({1: 18, 2: 6, 3: 3})");
  // Equal Again keys are one key (the first spelling, the last value), as in a Python dict.
  assert.equal(repr(() => new Die(new Map([[Again.add(1), 1], [2, 3], [Again.add(1), 2]]))), "Die({2: 19, 3: 6})");
  assert.throws(() => new Die([1, Again], { againCount: 1, againDepth: 1 }), pythonType("ValueError"));
  assert.throws(() => new Die([1, Again.sub(1)], { againCount: 1 }), pythonType("ValueError"));
  assert.throws(() => new Die([1, Again], { againEnd: ip.Restart }), pythonType("ValueError"));
});

test("Again expressions: truth values, equals, isAdditive", () => {
  assert.equal(ip.truth(Again.add(1).eq(Again.add(1))), true);
  assert.equal(ip.truth(Again.add(1).ne(Again.add(2))), true);
  assert.equal(ip.truth(Again.eq(1)), false);
  assert.throws(() => ip.truth(Again.add(1)), pythonType("ValueError"));
  assert.ok(Again.add(1).equals(Again.add(1)));
  assert.ok(!Again.equals(d(6)));
  // `die <op> Again` is the expression's reflected operator, as in Python.
  assert.ok(d(6).add(Again) instanceof ip.AgainExpression);
  assert.ok(Again.rmatmul(d(6)).isAdditive && !Again.rmatmul(d(6).sub(2)).isAdditive);
  assert.throws(() => ip.ops.floor(Again), pythonType("TypeError"));
});

test("Die(...): Python dict keys, tuples holding dice, times", () => {
  // 1, true and 1.0 are one dict key: the first spelling, the last value.
  assert.equal(repr(() => new Die(new Map([[1, 2], [true, 3], [new ip.Float(1), 4]]))), "Die({1: 4})");
  assert.equal(repr(() => new Die([[d(2), 1]])), "Die({(1, 1): 1, (2, 1): 1})");
  // A tuple holding Reroll drops out, like Reroll itself.
  assert.equal(repr(() => new Die([[1, Reroll], 2])), "Die({2: 1})");
  assert.throws(() => new Die([1, 2], [1]), pythonType("ValueError"));
  assert.throws(() => new Die([1, 2], [1, -1]), pythonType("ValueError"));
  assert.throws(() => new Die([1], 1.5), pythonType("TypeError"));
});

test("explode, ifElse, map with a Map and again options", () => {
  assert.equal(repr(() => d(6).explode([5, 6], { depth: 1 })), "Die({1: 6, 2: 6, 3: 6, 4: 6, 6: 1, 7: 2, 8: 2, 9: 2, 10: 2, 11: 2, 12: 1})");
  assert.equal(repr(() => d(4).ifElse("hi", d(2), { againDepth: 1 })), "Die({'hi': 4})");
  assert.equal(repr(() => d(4).map(new Map([[1, 4], [true, 9]]))), "Die({2: 1, 3: 1, 4: 1, 9: 1})");
  const exploding = native(() => d(3).map((x) => (x === 3 ? ip.ops.add(x, Again) : x), { againDepth: 2 }));
  assert.equal(exploding.repr(), "Die({1: 9, 2: 9, 4: 3, 5: 3, 7: 1, 8: 1, 9: 1})");
});

test("map with repeat, mapAndTime, meanTimeToAbsorb, timeToSum on the core", () => {
  const step = (x, { k }) => Math.min(x + k, 5);
  assert.equal(repr(() => ip.map(step, 0, { repeat: 3, k: 2, star: false })), "Die({5: 1})");
  const timed = native(() => ip.mapAndTime((x, { k }) => (x < 5 ? Math.min(x + k, 5) : x), 0, { repeat: 4, k: 2, star: false }));
  assert.equal(timed.repr(), "Die({(5, 3): 1})");
  assert.equal(native(() => ip.meanTimeToAbsorb((x) => (x >= 2 ? x : d(2).add(x)), 0)).toString(), "3/2");
  assert.deepEqual(native(() => d(6).timeToSum(10, 3, Reroll)).items(), [[2, 36n], [3, 99n]]);
  // A starred function needs states it can unpack.
  assert.throws(() => ip.map((a, b) => a, 0, { repeat: 1 }), pythonType("TypeError"));
});

test("the free functions", () => {
  assert.equal(repr(() => ip.coin(0.3, 1, { maxDenominator: 5 })), "Die({False: 2, True: 1})");
  assert.equal(repr(() => ip.stochasticRound(2.5)), "Die({2: 1, 3: 1})");
  assert.equal(repr(() => ip.tupleize(d(2), "x")), "Die({(1, 'x'): 1, (2, 'x'): 1})");
  assert.deepEqual(native(() => ip.tupleize(1, 2)), [1, 2]);
  assert.equal(repr(() => ip.fromCumulative([1, 2, 3], [1, 3, 6])), "Die({1: 1, 2: 2, 3: 3})");
  assert.equal(repr(() => d(2).sequence(2)), "Die({(1, 1): 1, (1, 2): 1, (2, 1): 1, (2, 2): 1})");
  assert.deepEqual(native(() => d(2).sequence(0)), []);
  assert.deepEqual(native(() => ip.consecutive([3, 1], [6])), [1, 2, 3, 4, 5, 6]);
  assert.deepEqual(native(() => ip.sortedUnion([3, 1], [2])), [1, 2, 3]);
  const harmonized = native(() => ip.harmonizeDenominators([d(2), d(3)], [1, 2]));
  assert.deepEqual(harmonized.map((x) => x.repr()), ["Die({1: 3, 2: 3})", "Die({1: 4, 2: 4, 3: 4})"]);
  assert.equal(repr(() => ip.reduce((a, b) => Math.max(a, b), [d(2), d(2)])), "Die({1: 1, 2: 3})");
  const sums = native(() => ip.accumulate((a, b) => a + b, [d(2), 1]));
  assert.deepEqual(sums.map((x) => x.repr()), ["Die({1: 1, 2: 1})", "Die({2: 1, 3: 1})"]);
  assert.throws(() => ip.reduce((a) => a, []), pythonType("StopIteration"));
  assert.throws(() => ip.pointwiseMax(), pythonType("TypeError"));
  assert.equal(native(() => ip.maxOutcome([d(6), d(8)])), 8);
  assert.equal(native(() => ip.formatProbabilityInverse(new ip.Fraction(1n, 36n))), "1 in 36");
});

test("sample: Python's seeded Mersenne Twister", () => {
  // Python: random.seed(7); [d6.sample() for _ in range(4)]
  assert.deepEqual(native(() => d(6).sample({ seed: 7, n: 4 })), [3, 2, 4, 6]);
  assert.deepEqual(native(() => d(6).sample({ seed: 7, n: 0 })), []);
});

test("Vector and Symbols methods", () => {
  assert.equal(native(() => new Vector([1, 2]).append(3).repr()), "Vector((1, 2, 3))");
  assert.equal(native(() => new Vector([1]).concatenate("ab").repr()), "Vector((1, 'a', 'b'))");
  assert.equal(native(() => new Vector([1]).concatenate(new Symbols("ba")).repr()), "Vector((1, 'a', 'b'))");
  assert.equal(native(() => String(Symbols.from(new Map([["a", 2], ["b", -1]])))), "aa - b");
  assert.equal(native(() => new Symbols("aab").countSubset("ab")), 1);
  assert.throws(() => new Symbols("a").method("no_such_method"), pythonType("AttributeError"));
});
