// What documenting the API (C6b) found to differ from Python icepool (task C6h), each checked against the oracle:
// error classes, Symbols' str, result types, keyword arguments, error messages and the methods JS lacked. Expected
// values are Python icepool's.

import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import * as internal from "../dist/internal.js";

const { d, Die, pool, Again, AgainExpression, Float, Vector, Symbols, Order, MultisetEvaluator } = ip;

/** The error `f` throws. */
function thrown(f) {
  try {
    f();
  } catch (e) {
    return e;
  }
  assert.fail("expected an exception");
}
/** Asserts `f` throws Python's `pythonType` with exactly this message. */
function raises(f, pythonType, message) {
  const e = thrown(f);
  assert.equal(e.name, pythonType);
  assert.equal(e.message, message);
}

test("errors keep their subclass, natively and through the bridge", () => {
  const conflicting = d(6).pool(1).forceOrder(Order.Ascending).forceOrder(Order.Descending);
  assert.ok(thrown(() => conflicting.sum()) instanceof ip.ConflictingOrderError);
  // The bridge's errors (in the fixture tests) are made like the native ones (rpc.ts used to build a plain
  // IcepoolError).
  const forced = (expr, order) => internal.call("mx_force_order", { expr, order });
  const bridgedConflicting = forced(forced(d(6).pool(1), Order.Ascending), Order.Descending);
  const bridged = thrown(() => internal.call("mx_evaluate", { expr: bridgedConflicting, method: "sum" }));
  assert.ok(bridged instanceof ip.ConflictingOrderError);
  assert.ok(bridged instanceof ip.OrderError);
  assert.equal(bridged.name, "ConflictingOrderError");
});

test("UnsupportedOrder is an OrderException, not an OrderError, as in Python", () => {
  const e = new ip.UnsupportedOrder("no");
  assert.ok(e instanceof ip.OrderException);
  assert.ok(!(e instanceof ip.OrderError));
  assert.ok(new ip.OrderError("UninferrableSize", "OrderError") instanceof ip.OrderException);
});

test("String(Symbols) is Python's str, however the Symbols was made", () => {
  assert.equal(String(new Symbols("ba")), "ab");
  assert.equal(String(new Symbols("a-b")), "a - b");
  assert.equal(String(new Symbols("b-a")), "b - a");
  assert.equal(String(new Symbols("aab").add("b")), "aabb");
  assert.equal(thrown(() => String(new Symbols("a--b"))).name, "ValueError");
});

test("result types: integral floats are Float, Again operands give Again expressions, plain products", () => {
  assert.ok(d(6).skewness() instanceof Float);
  assert.equal(Number(d(8).entropy()), 3);
  assert.ok(d(4).probabilities(null, { percent: true })[0] instanceof Float);
  assert.ok(d(6).add(Again) instanceof AgainExpression);
  assert.ok(d(6).eq(Again) instanceof AgainExpression);
  assert.ok(d(6).matmul(Again) instanceof AgainExpression);
  assert.ok(ip.ops.mul(d(6), Again) instanceof AgainExpression);
  assert.deepEqual(d(6).sequence(0), []);
  assert.deepEqual(ip.tupleize(1, 2), [1, 2]);
  assert.ok(ip.vectorize(1, 2) instanceof Vector);
  assert.equal(ip.ops.mul(3, 4), 12);
});

test("sortPairKeepWhile / DropWhile: Python's 'continue' / 'break', and its default 'break'", () => {
  assert.deepEqual(pool([6, 4, 3]).sortPairKeepWhile(">", [5, 2], Order.Descending, "continue").expand().items(), [[[3, 4, 6], 1n]]);
  assert.deepEqual(pool([6, 4, 3]).sortPairDropWhile(">", [5, 2], Order.Descending, "break").expand().items(), [[[3], 1n]]);
  // The default shows in the error for a right operand whose size cannot be inferred.
  const right = d(6).pool(1).keepOutcomes((x) => x > 2);
  raises(() => d(6).pool(2).sortPairKeepWhile(">", right).sum(), "OrderError",
    "The size of the right operand must be inferrable to sort_pair(>, extra=break).");
});

test("required keyword-only arguments: Python's TypeError without them", () => {
  raises(() => d(6).reroll([1]), "TypeError", "Die.reroll() missing 1 required keyword-only argument: 'depth'");
  raises(() => d(6).filter([1]), "TypeError", "Die.filter() missing 1 required keyword-only argument: 'depth'");
  raises(() => ip.mapAndTime((x) => x, d(6)), "TypeError", "map_and_time() missing 1 required keyword-only argument: 'repeat'");
  raises(() => d(6).mapAndTime((x) => x), "TypeError", "Die.map_and_time() missing 1 required keyword-only argument: 'repeat'");
  raises(() => ip.highest(), "TypeError", "highest() missing 1 required positional argument: 'arg0'");
  raises(() => ip.lowest(), "TypeError", "lowest() missing 1 required positional argument: 'arg0'");
  raises(() => ip.middle(), "TypeError", "middle() missing 1 required positional argument: 'arg0'");
  // One argument is an iterable of them, which may be empty.
  raises(() => ip.highest([]), "ValueError", "highest() arg is an empty sequence and no default was provided.");
  raises(() => ip.lowest([]), "ValueError", "lowest() arg is an empty sequence and no default was provided.");
  assert.equal(ip.highest([d(4), d(4)]).repr(), "Die({1: 1, 2: 3, 3: 5, 4: 7})");
});

test("keyword arguments reach f as a trailing options object, through the bridge too", () => {
  const nOf = (n, { sides }) => Array(n).fill(d(sides));
  assert.equal(String(ip.mapToPool(nOf, d(2), { sides: 4, star: false }).sum().mean()), "15/4");
  assert.equal(String(d(2).mapToPool(nOf, { sides: 4, star: false }).sum().mean()), "15/4");
  // A Map, and Die.map_to_pool's identity, take none: Python's lambdas raise when called.
  raises(() => ip.mapToPool(new Map([[1, [1]], [2, [2]]]), d(2), { foo: 1 }).sum(), "TypeError",
    "transition_and_star.<locals>.<lambda>() got an unexpected keyword argument 'foo'");
  raises(() => d(6).mapToPool(null, { foo: 1 }).sum(), "TypeError",
    "Die.map_to_pool.<locals>.<lambda>() got an unexpected keyword argument 'foo'");
  raises(() => ip.mapToPool(new Map([[1, [1]]]), d(2), d(2), { foo: 1 }), "ValueError",
    "If a mapping is provided for repl, len(args) must be 1.");
  raises(() => ip.map(new Map([[1, 2]]), d(6), { foo: 1 }), "TypeError",
    "transition_and_star.<locals>.<lambda>() got an unexpected keyword argument 'foo'");
  // A Die never calls the function: no error (as Python's).
  assert.ok(new Die([]).mapToPool(null, { foo: 1 }) instanceof ip.MultisetExpression);
});

test("error messages as Python's", () => {
  raises(() => d(6).nearest("==", 5), "ValueError", "Invalid comparison ==");
  raises(() => d(6).nearest("!=", 5), "ValueError", "Invalid comparison !=");
  const unsortable = "Items do not appear to be sortable.\nTip: Sequences containing dice or decks are not sortable.\n" +
    "Use tupleize() or vectorize() to transform\na sequence of dice into a die with sequence outcomes\n" +
    "according to the Cartesian product.";
  raises(() => new Die([new ip.Complex(1, 2)]), "TypeError", unsortable);
  raises(() => new Die([1, "a"]), "TypeError", unsortable);
  raises(() => new ip.Deck([new ip.Complex(1, 2)]), "TypeError", unsortable);
});

test("MultisetExpression.sum(map), largestCount({ wild... }), allStraightsReduceCounts", () => {
  const successes = (x) => (x === 6 ? 2 : x === 5 ? 1 : 0);
  assert.equal(d(6).pool(2).sum(successes).repr(), "Die({0: 16, 1: 8, 2: 9, 3: 2, 4: 1})");
  assert.equal(pool([1, 2, 6]).sum(new Map([[6, 0]])).repr(), "Die({3: 1})");
  raises(() => pool([1, 2, 6]).sum(5), "AttributeError", "'int' object has no attribute 'items'");

  assert.equal(pool([1, 3, 3, 5]).largestCount({ wild: [1] }).repr(), "Die({3: 1})");
  assert.equal(pool([1, 6]).largestCount({ wildHigh: (x) => x === 6 }).repr(), "Die({2: 1})");
  assert.equal(d(6).pool(2).largestCount({ wild: [6] }).repr(), "Die({1: 20, 2: 16})");
  assert.equal(d(4).pool(3).largestCount({ wildLow: new Map([[1, 0]]), wildHigh: [4] }).repr(),
    "Die({2: 18, 3: 46})");
  // frozenset(str): its characters, so no int outcome is wild.
  assert.equal(pool([3, 5, 1]).largestCount({ wild: "35" }).repr(), "Die({1: 1})");
  raises(() => pool([3, 5, 1]).largestCount({ wild: 3 }), "TypeError", "'int' object is not iterable");

  assert.equal(pool([1, 2, 2, 3, 5]).allStraightsReduceCounts().repr(), "Die({((3, 2), (1, 1)): 1})");
  assert.equal(pool([1, 2, 2, 3, 5]).allStraightsReduceCounts((a, b) => a + b).repr(), "Die({((3, 4), (1, 1)): 1})");
  // `reducer=None` is called once a straight continues.
  raises(() => pool([1, 2, 2, 3, 5]).allStraightsReduceCounts(null), "TypeError", "'NoneType' object is not callable");
  assert.equal(pool([1, 3, 5]).allStraightsReduceCounts(null).repr(), "Die({((1, 1), (1, 1), (1, 1)): 1})");
  // They run natively, not on the bridge.
  const total = () => [...internal.bridgeCalls.values()].reduce((a, b) => a + b, 0);
  const before = total();
  d(6).pool(2).sum(successes);
  d(6).pool(2).largestCount({ wild: [6] });
  pool([1, 2, 3]).allStraightsReduceCounts();
  assert.equal(total(), before);
});

test("MultisetEvaluator.consecutive and a weighted multisetMixture", () => {
  class Gaps extends MultisetEvaluator {
    nextState(n, order, outcome, count) {
      return (n ?? 0) + (count === 0 ? 1 : 0);
    }
    extraOutcomes(outcomes) {
      return this.consecutive(outcomes);
    }
  }
  assert.deepEqual(new Gaps().evaluate([1, 4, 4, 6]).items(), [[3, 1n]]);
  assert.deepEqual(new Gaps().consecutive([true, 3]), [1, 2, 3]);
  assert.deepEqual(new Gaps().consecutive([]), []);
  raises(() => new Gaps().consecutive([1, "a"]), "TypeError", "consecutive cannot be used with outcomes of type other than 'int'.");

  const weighted = ip.multisetMixture(new Map([[d(6).pool(1), 1], [d(6).pool(2), 3]]));
  assert.equal(String(weighted.sum().mean()), "49/8");
  assert.equal(ip.multisetMixture(new Map([[pool([1]), 2], [pool([2, 2]), 1]])).sum().repr(), "Die({1: 2, 4: 1})");
});
