// The JS API itself: what the fixtures cannot show (JS types at the boundary, callbacks, exceptions, evaluators
// with object states, garbage collection). Expected values are Python icepool's (computed with the oracle).

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
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

test("multisetFunction: JS functions traced like @multiset_function", () => {
  const { multisetFunction, evaluator, IcepoolError } = ip;
  // Joint results are tuples (arrays).
  const twoWay = multisetFunction((a, b) => [a.sub(b).size(), b.sub(a).size()]);
  assert.ok(twoWay instanceof ip.MultisetFunction);
  assert.deepEqual(twoWay.call(d(4).pool(2), d(4).pool(1)).items(), [[[1, 0], 28n], [[2, 1], 36n]]);
  // A MultiDeal input: star inferred (2 parameters, 1 input), or one tuple parameter whose hands are indexed.
  const hands = new Deck([1, 2, 3, 4]).deal([2, 1]);
  assert.deepEqual(multisetFunction((a, b) => a.sub(b).size()).call(hands).items(), [[2, 12n]]);
  const handsSum = multisetFunction((h) => {
    assert.ok(h instanceof ip.MultisetTupleExpression);
    return h.hand(0).add(h.hand(1)).sum();
  });
  assert.deepEqual(handsSum.evaluate(hands).items(), [[6, 3n], [7, 3n], [8, 3n], [9, 3n]]);
  // Keyword-only arguments arrive as a trailing options object.
  const countIn = multisetFunction((a, { target } = {}) => a.keepOutcomes(target).size(), { kwonly: ["target"] });
  assert.deepEqual(countIn.call(d(6).pool(3), { target: [5, 6] }).items(), [[0, 64n], [1, 96n], [2, 48n], [3, 8n]]);
  assert.throws(() => countIn.call(d(6).pool(3)), (e) => e instanceof IcepoolError && e.pythonType === "TypeError");
  // Nested multiset functions, and a `...rest` parameter.
  const uniq = multisetFunction((a) => a.unique().size());
  const outer = multisetFunction((a, b) => [uniq.call(a.add(b)), a.and(b).size()]);
  assert.deepEqual(outer.call(d(3).pool(2), d(3).pool(1)).items(), [[[1, 1], 3n], [[2, 0], 6n], [[2, 1], 12n], [[3, 0], 6n]]);
  const restF = multisetFunction((a, ...rest) => (rest.length ? a.add(rest[0]).sum() : a.sum()), { rest: true });
  assert.deepEqual(restF.call(d(2).pool(1), d(2).pool(1)).items(), [[2, 1n], [3, 2n], [4, 1n]]);
  // A JS evaluator inside the body, and a constant in a joint result.
  const evens = evaluator({ nextState: (s, order, o, c) => (s ?? 0) + (o % 2 === 0 ? c : 0) });
  const withEvens = multisetFunction((a, b) => [evens.evaluate(a.or(b)), 7]);
  assert.deepEqual(withEvens.call(d(4).pool(2), d(4).pool(1)).items(), [[[0, 7], 8n], [[1, 7], 32n], [[2, 7], 22n], [[3, 7], 2n]]);
  // The JS evaluator is still registered when the function is evaluated again.
  assert.deepEqual(withEvens.call(d(4).pool(2), d(4).pool(1)).items()[3], [[3, 7], 2n]);
  // Python's errors, and the body's own exception, rethrown as it was.
  assert.throws(() => multisetFunction((a) => a.add(a)).call(d(6).pool(2)), (e) => e.pythonType === "TypeError");
  const oops = new RangeError("oops");
  assert.throws(() => multisetFunction((a) => { throw oops; }).call(d(6).pool(2)), (e) => e === oops);
  assert.throws(() => multisetFunction((a) => a.sum()).call(d(6).pool(2), d(6).pool(2)), (e) => e.pythonType === "TypeError");
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

test("fromRv takes a JS random variable: cdf with kwargs, a pdf makes it continuous", () => {
  const calls = [];
  // `cdf(xs, scale=1)`: `[min(max(x / (4 * scale), 0.0), 1.0) for x in xs]`.
  const rv = {
    cdf(xs, { scale = 1 } = {}) {
      calls.push(Array.from(xs));
      return Array.from(xs, (x) => Math.min(Math.max(x / (4 * scale), 0), 1));
    },
  };
  assert.deepEqual(ip.fromRv(rv, [0, 1, 2, 3, 4], 100).items(), [[1, 25n], [2, 25n], [3, 25n], [4, 25n]]);
  assert.deepEqual(ip.fromRv(rv, [0, 1, 2, 3, 4], 100, { scale: 2 }).items(), [[1, 12n], [2, 13n], [3, 13n], [4, 12n]]);
  // Continuous (Python's `hasattr(rv, 'pdf')`): the CDF at the midpoints, once.
  calls.length = 0;
  const continuous = { pdf: () => 0, cdf: rv.cdf };
  assert.deepEqual(ip.fromRv(continuous, [1, 2, 3], 10).items(), [[1, 4n], [2, 2n], [3, 4n]]);
  assert.deepEqual(calls, [[1.5, 2.5]]);
  const mine = new RangeError("mine");
  assert.throws(() => ip.fromRv({ cdf: () => { throw mine; } }, [1, 2], 10), (e) => e === mine);
  assert.throws(() => ip.fromRv({}, [1, 2], 10), TypeError);
});

test("Wallenius: data -> deal(n) -> evaluations; a Map's keys are Python dict keys", () => {
  // `Wallenius({1: 1, True: 3, 2: (2, 2)})` is `{1: 3, 2: (2, 2)}` in Python; the JS Map has three keys.
  const w = new ip.Wallenius(new Map([[1, 1], [true, 3], [2, [2, 2]]]));
  assert.deepEqual(w.deal(2).expand().items(), [[[1, 2], 108n], [[2, 2], 32n]]);
  assert.deepEqual(new ip.Wallenius([[1, 1], [2, 3]]).deal(1).sum().items(), [[1, 1n], [2, 3n]]);
  // Python checks the data in the constructor.
  assert.throws(() => new ip.Wallenius([[1, 2, 3]]), (e) => e.pythonType === "ValueError");
});

test("Die methods: sequence, keep, meanTimeToSum, stochasticRound, the to-pool methods", () => {
  assert.deepEqual(d(2).sequence(2).items(), [[[1, 1], 1n], [[1, 2], 1n], [[2, 1], 1n], [[2, 2], 1n]]);
  assert.deepEqual(d(2).sequence(0), []);
  assert.deepEqual(d(6).keep([0, 0, 1]).items().slice(0, 3), [[1, 1n], [2, 7n], [3, 19n]]);
  assert.deepEqual(d(6).keep(3, -1).items().slice(0, 3), [[1, 1n], [2, 7n], [3, 19n]]);
  assert.deepEqual(d(6).keepSlice(4, 1).items().slice(0, 3), [[3, 1n], [4, 4n], [5, 10n]]);
  assert.deepEqual(d(6).meanTimeToSum(10), new Fraction(33495175n, 10077696n));
  assert.deepEqual(new Die([0.5, 1.25]).stochasticRound().items(), [[0, 2n], [1, 5n], [2, 1n]]);
  const nOf = (x) => ip.List.from(Array(x).fill(x));
  assert.deepEqual(d(3).mapToPool(nOf).sum().items(), [[1, 1n], [4, 1n], [9, 1n]]);
  assert.deepEqual(ip.mapToPool(nOf, d(3)).sum().items(), [[1, 1n], [4, 1n], [9, 1n]]);
  assert.deepEqual(d(3).explodeToPool(2, null, { depth: 1 }).sum().quantities(), [9n, 18n, 9n, 6n, 12n, 12n, 7n, 2n, 3n, 2n, 1n]);
  assert.deepEqual(d(3).rerollToPool(2, [1], { maxRerolls: 1 }).sum().items(), [[2, 1n], [3, 3n], [4, 8n], [5, 10n], [6, 5n]]);
});

test("a Map is a Python dict where icepool takes a mapping: keys equal by Python's ==", () => {
  // `{1: 10, True: 20, 2.0: 30}` is `{1: 20, 2.0: 30}`.
  const m = new Map([[1, 10], [true, 20], [new Float(2), 30]]);
  assert.deepEqual(d(3).map(m).items(), [[3, 1n], [20, 1n], [30, 1n]]);
  assert.deepEqual(d(4).reroll(new Map([[1, null]]), { depth: 1 }).items(), [[1, 1n], [2, 5n], [3, 5n], [4, 5n]]);
  const groups = d(4).groupBy(new Map([[1, "lo"], [2, "lo"], [new Float(3), "hi"], [4, "hi"]]));
  assert.deepEqual([...groups.keys()], ["lo", "hi"]);
  assert.deepEqual(groups.get("hi").outcomes(), [3, 4]);
  // A tuple key: equal arrays are one key, though a JS Map keeps them apart.
  const pairs = new Die([[1, 2], [2, 1]]).map(new Map([[[1, 2], [0, 0]], [[1, 2], [9, 9]]]));
  assert.deepEqual(pairs.outcomes(), [[2, 1], [9, 9]]);
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

test("every export is declared in icepool.d.ts", () => {
  const dts = fs.readFileSync(new URL("../icepool.d.ts", import.meta.url), "utf8");
  const declared = new Set(
    [...dts.matchAll(/^export (?:declare )?(?:class|function|const|type|interface) (\w+)/gm)].map((m) => m[1]),
  );
  assert.deepEqual(Object.keys(ip).filter((name) => !declared.has(name)), []);
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

test("release() frees a handle at once; a released object throws a TypeError", () => {
  const die = d(6).add(1);
  const sum = die.add(die);
  const live = ip.liveHandles();
  die.release();
  assert.equal(ip.liveHandles(), live - 1);
  die.release(); // again: nothing
  assert.throws(() => die.add(1), TypeError);
  assert.throws(() => die.toWire(), TypeError);
  assert.throws(() => d(6).add(die), TypeError);
  assert.deepEqual(sum.outcomes().slice(0, 2), [4, 5]); // made from it, unaffected
  const left = ip.liveHandles();
  sum[Symbol.dispose]();
  assert.equal(ip.liveHandles(), left - 1);
  Again.release(); // the shared Again stays
  assert.deepEqual(new Die([1, Again.add(1).add(1)]).outcomes(), [1, 3, 4]);
});

test("a long loop that releases what it makes holds nothing afterwards", () => {
  // The robustness review's loop: a map with a callback holding a big default, then a pool sum.
  const before = ip.liveHandles();
  const big = Array.from({ length: 10000 }, (_, i) => i);
  for (let i = 0; i < 20000; i++) {
    const die = d(6);
    const mapped = die.map((o) => (big.length ? o : o));
    const pool = die.pool(3);
    const sum = pool.sum();
    for (const h of [die, mapped, pool, sum]) h.release();
  }
  assert.equal(ip.liveHandles(), before);
});

test("release() frees a JS function the core holds, which the garbage collector cannot", async () => {
  // The function refers to its own multiset function, so the core's reference to the function (from the global
  // table of callbacks) keeps the handle reachable: only release() breaks the cycle.
  v8.setFlagsFromString("--expose-gc");
  const gc = vm.runInNewContext("gc");
  let captured;
  (() => {
    const payload = { data: new Array(1000).fill(1) };
    captured = new WeakRef(payload);
    const f = ip.multisetFunction((a) => (payload && f ? a.size() : a.size()));
    assert.deepEqual(f.call(d(6).pool(2)).outcomes(), [2]);
    f.release();
  })();
  // Not `deref()` meanwhile: it keeps the object alive until the end of the current job.
  for (let i = 0; i < 3; i++) {
    await new Promise((r) => setTimeout(r, 10));
    gc();
  }
  assert.equal(captured.deref(), undefined, "the function was not freed");
});

test("deep nesting raises RecursionError rather than overflowing the WASM stack", () => {
  const isRecursionError = (e) => e instanceof ip.IcepoolError && e.pythonType === "RecursionError";
  let again = Again;
  assert.throws(() => { for (let i = 0; i < 5000; i++) again = again.add(1); }, isRecursionError);
  assert.deepEqual(new Die([1, Again.add(1).add(1)]).outcomes(), [1, 3, 4]);
  let expr = d(2).pool(1);
  assert.throws(() => { for (let i = 0; i < 5000; i++) expr = expr.or(d(2).pool(1)); }, isRecursionError);
  let pools = d(2).pool(1);
  assert.throws(() => { for (let i = 0; i < 5000; i++) pools = pools.additiveUnion(d(2).pool(1)); }, isRecursionError);
  // A multiset function calling itself: Python's recursion limit, or JS's own stack.
  const f = ip.multisetFunction((a) => f.call(a));
  assert.throws(() => f.call(d(2).pool(1)), (e) => isRecursionError(e) || e instanceof RangeError);
});

test("sizes no memory holds raise MemoryError rather than trapping", () => {
  const isMemoryError = (e) => e instanceof ip.IcepoolError && e.pythonType === "MemoryError";
  assert.throws(() => d(2n ** 63n - 1n), isMemoryError);
  assert.throws(() => d(2 ** 33), isMemoryError);
  assert.throws(() => d(2).sequence(2n ** 62n), isMemoryError);
  assert.equal(d(6).denominator(), 6n);
});

test("a trap is a PanicException, and the instance stays usable", () => {
  // A JS callback throwing a RuntimeError stands in for a panic deep in the core: it abandons the WASM frames
  // below it without running their destructors, just as a trap does.
  const trap = () => { throw new WebAssembly.RuntimeError("simulated trap"); };
  const isPanic = (e) => e instanceof ip.IcepoolError && e.pythonType === "PanicException";
  const kept = d(4);
  assert.throws(() => d(6).map(trap), isPanic);
  // From a nested call into the core: the outer call is abandoned too.
  assert.throws(() => d(6).map(() => d(2).map(trap)), isPanic);
  // Abandoned 40 calls deep in a multiset function calling itself, many times over: the levels and frames those
  // calls held would add up past the recursion limit if they were not reset.
  for (let i = 0; i < 100; i++) {
    let n = 0;
    const f = ip.multisetFunction((a) => (++n > 40 ? trap() : f.call(a)));
    assert.throws(() => f.call(d(2).pool(1)), isPanic);
  }
  const g = ip.multisetFunction((a, b) => a.add(b).sum());
  assert.deepEqual(g.call(d(2).pool(1), d(2).pool(1)).items(), d(2).add(d(2)).items());
  assert.equal(kept.add(1).denominator(), 4n);
});

test("run and calculate, the raw exports' wrapped versions", async () => {
  assert.deepEqual(JSON.parse(ip.run('{"op":"d","sides":2}')), { ok: true, die: [[1, 1], [2, 1]] });
  assert.equal(JSON.parse(ip.run('{"op":"d","sides":0}')).error, "ValueError");
  assert.throws(() => ip.calculate("not an object"));
  // No panic is reachable from a program, so a stand-in module traps instead, bound to a fresh copy of core.js:
  // the trap must leave the shadow stack pointer where the call found it and reset the core's state.
  const core = await import("../core.js?trap");
  const sp = new WebAssembly.Global({ value: "i32", mutable: true }, 1000);
  let recovered = 0;
  const trap = () => { sp.value = 600; throw new WebAssembly.RuntimeError("unreachable"); };
  core.bind({ stack_pointer: () => sp, recover: () => (recovered++, "a panic"), run: trap, calculate: trap });
  for (const f of [() => core.run("{}"), () => core.calculate({})]) {
    assert.throws(f, (e) => e instanceof core.IcepoolError && e.pythonType === "PanicException" && /a panic/.test(e.message));
    assert.equal(sp.value, 1000);
  }
  assert.equal(recovered, 2);
});
