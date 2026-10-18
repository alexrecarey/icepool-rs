// The parts of the API task C6e moved off the JSON-op bridge: JS `MultisetEvaluator`s (states, keyword arguments,
// Reroll and Restart, final dice, UnsupportedOrder, exceptions, re-entrant calls) and `multisetFunction` (traced
// from the TypeScript expressions: single and joint results, constants, nested functions, JS evaluators in a body,
// binding errors). Expected values are Python icepool's. Each also checks that it made no call through the bridge,
// except where a body holds a deal (task C6f), which still goes through it.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Deck, MultisetEvaluator, Order, Reroll, Restart, UnsupportedOrder, d, evaluator, multisetFunction, pool } = ip;

/** Runs `f`, asserting that it called nothing through the bridge. */
function native(f) {
  const total = () => [...bridgeCalls.values()].reduce((a, b) => a + b, 0);
  const before = total();
  const result = f();
  assert.equal(total(), before, `bridge calls: ${JSON.stringify([...bridgeCalls])}`);
  return result;
}
const items = (f) => native(() => f().items());
const pythonType = (name) => (e) => e instanceof ip.IcepoolError && e.pythonType === name;
const d6 = d(6);

class Size extends MultisetEvaluator {
  nextState(state, order, outcome, count) {
    return (state ?? 0) + count;
  }
}

test("JS evaluators: subclasses, evaluator(), keyword arguments, mixtures and filters as inputs", () => {
  class CountSixes extends MultisetEvaluator {
    nextState(state, order, outcome, count) {
      return (state ?? 0) + (outcome === 6 ? count : 0);
    }
  }
  assert.deepEqual(items(() => new CountSixes().evaluate(d6.pool(3))), [[0, 125n], [1, 75n], [2, 15n], [3, 1n]]);
  assert.deepEqual(items(() => new Size().evaluate([1, 2, 2])), [[3, 1n]]);
  // evaluate()'s keyword arguments reach initialState and finalOutcome as a trailing options object.
  const atLeast = evaluator({
    initialState: (order, outcomes, size, { start }) => start,
    nextState: (n, order, outcome, count) => n + count,
    finalOutcome: (n, order, outcomes, size, { target }) => n >= target,
  });
  assert.equal(native(() => atLeast.evaluate(d6.pool(3).keepOutcomes([5, 6]), { start: 0, target: 2 }).probability(true).toString()), "7/27");
  // A mixture is evaluated once per inner, the results mixed by weight.
  assert.deepEqual(items(() => new Size().evaluate(ip.multisetMixture([d6.pool(1), d6.pool(2)]))), [[1, 36n], [2, 36n]]);
  assert.deepEqual(items(() => new Size().evaluate(d6.pool(3).keepOutcomes((x) => x > 4))), [[0, 64n], [1, 96n], [2, 48n], [3, 8n]]);
});

test("JS evaluators: Reroll and Restart, final dice, None, plain object states", () => {
  const noSixes = (special) => evaluator({ nextState: (s, order, o, c) => (o === 6 && c > 0 ? special : (s ?? 0) + o * c) });
  assert.ok(native(() => noSixes(Reroll).evaluate(d6.pool(2)).equals(d(5).add(d(5)))));
  assert.ok(native(() => noSixes(Restart).evaluate(d6.pool(2)).equals(d(5).add(d(5)))));
  // A final outcome may be a die.
  const finalDie = evaluator({
    initialState: () => 0,
    nextState: (s, order, o, c) => s + (o === 6 ? c : 0),
    finalOutcome: (s) => (s > 0 ? d(s) : 0),
  });
  assert.deepEqual(items(() => finalDie.evaluate(d6.pool(2))), [[0, 50n], [1, 21n], [2, 1n]]);
  assert.throws(() => evaluator({ nextState: () => 0, finalOutcome: () => null }).evaluate(d6.pool(1)), pythonType("TypeError"));
  // Plain objects are states keyed by their JSON; each comes back as a fresh object.
  const seen = [];
  const objects = evaluator({
    initialState: () => ({ n: 0 }),
    nextState: (s, order, o, c) => {
      seen.push(s);
      return { n: s.n + c };
    },
    finalOutcome: (s) => s.n,
  });
  assert.deepEqual(items(() => objects.evaluate(d6.pool(2))), [[2, 36n]]);
  assert.ok(seen.every((s) => Object.getPrototypeOf(s) === Object.prototype) && new Set(seen).size === seen.length);
});

test("JS evaluators: UnsupportedOrder asks for the other order; exceptions come out as themselves", () => {
  const ascendingOnly = evaluator({
    initialState: (order) => {
      if (order !== Order.Ascending) throw new UnsupportedOrder();
      return null;
    },
    nextState: (s, order, o, c) => (c ? o : s),
  });
  assert.deepEqual(items(() => ascendingOnly.evaluate(d6.pool(2))), [[1, 1n], [2, 3n], [3, 5n], [4, 7n], [5, 9n], [6, 11n]]);
  assert.throws(() => ascendingOnly.evaluate(d6.pool(2).forceOrder(Order.Descending)), pythonType("ConflictingOrderError"));
  // Outside the evaluation's try (Python's extra_outcomes), UnsupportedOrder propagates as itself.
  const unsupported = new UnsupportedOrder("no");
  const extra = evaluator({ nextState: () => 0, extraOutcomes: () => { throw unsupported; } });
  assert.throws(() => extra.evaluate(d6.pool(1)), (e) => e === unsupported);
  const boom = new RangeError("boom");
  for (const method of ["initialState", "nextState", "finalOutcome", "extraOutcomes"]) {
    const ev = evaluator({ nextState: () => 0, [method]: () => { throw boom; } });
    assert.throws(() => ev.evaluate(d6.pool(2)), (e) => e === boom, method);
  }
});

test("JS evaluators: re-entrant calls into the API from a callback", () => {
  const inner = new Size();
  const outer = evaluator({ nextState: (s, order, o, c) => (s ?? 0) + Number(inner.evaluate([o, o]).mean()) * c });
  assert.deepEqual(items(() => outer.evaluate(d(3).pool(2))), [[4, 9n]]);
  // An exception from a nested evaluation passes through the outer one as itself.
  const boom = new Error("nested");
  const throwing = evaluator({ nextState: () => { throw boom; } });
  const nesting = evaluator({ nextState: () => throwing.evaluate([1]) });
  assert.throws(() => nesting.evaluate(d6.pool(1)), (e) => e === boom);
});

test("multisetFunction: single and joint results, constants, nested functions, JS evaluators in a body", () => {
  const diff = multisetFunction((a, b) => a.sub(b).size());
  assert.equal(native(() => diff.call(d6.pool(3), d6.pool(2)).mean().toString()), "319/144");
  const joint = multisetFunction((a) => [a.sum(), 5, d(2)]);
  assert.deepEqual(items(() => joint.call([1, 2])), [[[3, 5, 1], 1n], [[3, 5, 2], 1n]]);
  const inner = multisetFunction((a, b) => a.add(b).size());
  const outer = multisetFunction((a) => [inner.call(a, a), new Size().evaluate(a)]);
  assert.deepEqual(items(() => outer.call(d6.pool(2))), [[[4, 2], 36n]]);
  const kw = multisetFunction((a, { target } = {}) => a.keepOutcomes(target).size(), { kwonly: ["target"] });
  assert.deepEqual(items(() => kw.call(d6.pool(2), { target: [5, 6] })), [[0, 16n], [1, 16n], [2, 4n]]);
  // Parameters print as the program runner names them; a str result is iterated as Python iterates it.
  const strs = multisetFunction((a, b) => [String(a.add(b)), "xy"]);
  assert.deepEqual(items(() => strs.call([1], [2])), [[["(p0 + p1)", "xy"], 1n]]);
  const technoir = multisetFunction((action, push, hurt) => {
    const survivingPush = push.dropOutcomes(hurt);
    return [action.dropOutcomes(hurt).add(survivingPush).unique(2).highestOutcomeAndCount(), survivingPush.size()];
  });
  native(() => technoir.call(d6.pool(2), d6.pool(1), d6.pool(2)));
});

test("multisetFunction: Python's errors for binding, results and recursion", () => {
  const kw = multisetFunction((a, { target } = {}) => a.keepOutcomes(target).size(), { kwonly: ["target"] });
  assert.throws(() => kw.call(d6.pool(1), d6.pool(1), { target: [1] }), pythonType("TypeError"));
  assert.throws(() => kw.call(d6.pool(1), { target: [1], other: 1 }), pythonType("TypeError"));
  assert.throws(() => kw.call(d6.pool(1)), pythonType("TypeError"));
  // Only a MultiDeal can be starred.
  assert.throws(() => kw.call(d6.pool(1), { star: true, target: [1] }), pythonType("TypeError"));
  assert.throws(() => multisetFunction((a) => a.add(a)).call(d6.pool(1)), pythonType("TypeError"));
  assert.throws(() => multisetFunction((a) => []).call(d6.pool(1)), pythonType("ValueError"));
  assert.throws(() => multisetFunction((a) => 5).call(d6.pool(1)), pythonType("TypeError"));
  const boom = new Error("traced");
  assert.throws(() => multisetFunction((a) => { throw boom; }).call([1]), (e) => e === boom);
  // A function calling itself without end: Python's RecursionError, not a stack overflow.
  const f = multisetFunction((a) => f.call(a));
  assert.throws(() => f.call(d(2).pool(1)), pythonType("RecursionError"));
});

test("multisetFunction: a body holding a deal goes through the bridge (task C6f)", () => {
  const withDeal = multisetFunction((a) => a.add(new Deck([1, 2]).deal(1)).sum());
  const before = bridgeCalls.get("mf_call_kw") ?? 0;
  assert.deepEqual(withDeal.call(pool([3])).items(), [[4, 1n], [5, 1n]]);
  assert.equal(bridgeCalls.get("mf_call_kw"), before + 1);
});
