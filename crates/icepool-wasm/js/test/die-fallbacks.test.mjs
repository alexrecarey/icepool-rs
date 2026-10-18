// The arguments task C6g moved off the JSON-op bridge: selections of any collection (`reroll`, `filter`, `split`,
// `explode`, `isIn`, `count`), reroll depths, `groupBy` with any keys and `groupByIndex`, `toOneHot` of any
// sequence, truth values, `==` whose data raises, and Python's operators on `Reroll` and on what has no operator.
// Expected values are Python icepool's (fixtures/c6g/edge-cases.jsonl has them as programs). Each also checks that
// it made no call through the bridge.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Deck, Die, Float, Reroll, Restart, Symbols, Vector, d, map, ops, truth } = ip;

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
const throwsNatively = (f, name) => native(() => assert.throws(f, pythonType(name)));

test("selections: `outcome in collection` for any collection", () => {
  // A Vector is a sequence, a deck and a Map are mappings (their keys), `Reroll` in a list matches nothing.
  assert.deepEqual(items(() => d(6).reroll(new Vector([1, 2]), { depth: 1 })), [[1, 2n], [2, 2n], [3, 8n], [4, 8n], [5, 8n], [6, 8n]]);
  assert.deepEqual(items(() => d(6).reroll(new Deck([1, 6, 6]), { depth: 1 })), [[1, 2n], [2, 8n], [3, 8n], [4, 8n], [5, 8n], [6, 2n]]);
  assert.deepEqual(items(() => d(6).reroll([1, Reroll], { depth: 1 })), [[1, 1n], [2, 7n], [3, 7n], [4, 7n], [5, 7n], [6, 7n]]);
  assert.deepEqual(items(() => d(4).isIn(new Map([[1, 0], [new Float(4), 0]]))), [[false, 2n], [true, 2n]]);
  assert.deepEqual(items(() => d(6).count(3, d(2))), [[0, 64n], [1, 96n], [2, 48n], [3, 8n]]);
  const [low, high] = native(() => d(4).split(new Die([2, 3])));
  assert.deepEqual([low.outcomes(), high.outcomes()], [[2, 3], [1, 4]]);
  // Anything else is not iterable, which Python only finds out if there is an outcome to test.
  throwsNatively(() => d(6).reroll(Reroll, { depth: 1 }), "TypeError");
  throwsNatively(() => d(4).filter(null, { depth: 1 }), "TypeError");
  throwsNatively(() => d(4).explode(5, { depth: 1 }), "TypeError");
  const [a, b] = native(() => new Die([]).split(5));
  assert.deepEqual([a.items(), b.items()], [[], []]);
  // A str is a substring test, which only takes a str.
  throwsNatively(() => d(6).count(2, "x"), "TypeError");
  // A starred function unpacks any iterable outcome: a Vector's elements, a Symbols' characters.
  assert.deepEqual(items(() => new Die([new Vector([1, 2]), new Vector([3, 4])]).map((x, y) => x + y, { star: true })), [[3, 1n], [7, 1n]]);
  throwsNatively(() => d(3).split((x) => x > 1, { star: true }), "TypeError");
});

test("Symbols in a selection: `Symbols.__getitem__` never raises KeyError (DIVERGENCES.md)", () => {
  // So `x in symbols` holds for anything with a len(), and raises for anything without.
  assert.deepEqual(items(() => new Die(["a", "c"]).isIn(new Symbols("ab"))), [[true, 2n]]);
  throwsNatively(() => d(2).isIn(new Symbols("ab")), "TypeError");
});

test("reroll and filter: the outcomes first, then Python's checks on the depth", () => {
  throwsNatively(() => d(6).reroll([1], { depth: "x" }), "TypeError");
  throwsNatively(() => d(6).reroll([1], { depth: -1 }), "ValueError");
  throwsNatively(() => new Die([]).reroll(null, { depth: 1 }), "IndexError");
  throwsNatively(() => new Die([]).reroll(4, { depth: "x" }), "TypeError");
  // `False` is the int 0.
  assert.deepEqual(items(() => d(4).filter(d(2), { depth: false })), [[1, 1n], [2, 1n], [3, 1n], [4, 1n]]);
  // Every outcome rerolled with a finite depth: Python divides by zero.
  throwsNatively(() => d(6).reroll([1, 2, 3, 4, 5, 6], { depth: 2 }), "ZeroDivisionError");
});

test("groupBy: keys as a Python dict's (equal keys are one, dice and Reroll too); groupByIndex", () => {
  const byMapping = native(() => d(6).groupBy(new Map([[1, new Float(2)], [2, true], [3, 1]])));
  assert.deepEqual([...byMapping].map(([k, v]) => [String(k), v.outcomes()]), [["2.0", [1]], ["true", [2, 3]], ["4", [4]], ["5", [5]], ["6", [6]]]);
  const byDice = native(() => d(4).groupBy(new Map([[1, d(2)], [2, d(2)], [3, Reroll], [4, Reroll]])));
  assert.deepEqual([...byDice].map(([, v]) => v.outcomes()), [[1, 2], [3, 4]]);
  assert.ok([...byDice.keys()][0].equals(d(2)));
  assert.equal([...byDice.keys()][1], Reroll);
  throwsNatively(() => d(3).groupBy(4), "AttributeError");
  throwsNatively(() => d(4).groupBy(new Map([[1, ip.List.from([1])]])), "TypeError");
  const byFirst = native(() => new Die([new Vector([1, 2]), new Vector([1, 3]), new Vector([2, 2])]).groupByIndex(0));
  assert.deepEqual([...byFirst].map(([k, v]) => [k, v.size]), [[1, 2], [2, 1]]);
  throwsNatively(() => new Die(["ab", "ba"]).groupByIndex("x"), "TypeError");
});

test("toOneHot of any sequence: len(), `in`, then `index` (mappings have none)", () => {
  const hot = (die, outcomes) => native(() => die.toOneHot(outcomes).items().map(([v, q]) => [String(v), q]));
  assert.deepEqual(hot(d(3), new Vector([3, 1])), [["Vector((False, False))", 1n], ["Vector((False, True))", 1n], ["Vector((True, False))", 1n]]);
  assert.deepEqual(hot(new Die(["b", "bc", "z"]), "abc"), [["Vector((False, False, False))", 1n], ["Vector((False, True, False))", 2n]]);
  // A mapping is fine while no outcome is in it.
  assert.deepEqual(hot(d(3), new Map([[5, 0]])), [["Vector((False,))", 3n]]);
  throwsNatively(() => d(3).toOneHot(new Map([[1, 0]])), "AttributeError");
  throwsNatively(() => d(3).toOneHot(d(6)), "AttributeError");
  throwsNatively(() => d(2).toOneHot(new Deck([1, 6])), "AttributeError");
  // `symbols.index` is `symbols['index']` (Symbols.__getattr__), which is not callable.
  throwsNatively(() => new Die(["a"]).toOneHot(new Symbols("ab")), "TypeError");
  throwsNatively(() => d(4).toOneHot(5), "TypeError");
  assert.deepEqual(hot(new Die([]), 5), []);
});

test("truth values: a die refuses, a deck is a mapping, `==` keeps its data's error for later", () => {
  throwsNatively(() => truth(d(6)), "TypeError");
  assert.equal(native(() => truth(new Deck([]))), false);
  assert.equal(native(() => truth(new Deck([1]))), true);
  const unequal = native(() => new Die([new Vector([1])]).ne(new Die([new Vector([1, 2])])));
  assert.equal(native(() => truth(unequal)), true);
  throwsNatively(() => unequal.repr(), "ValueError");
  throwsNatively(() => d(2).equals(unequal), "ValueError");
  assert.equal(native(() => unequal.equals("x")), false);
  throwsNatively(() => d(6).quantityWhere([1, 2]), "TypeError");
});

test("operators: Reroll has only ==, `str % die` and `Symbols // die` are the left operand's", () => {
  throwsNatively(() => ops.add(Reroll, 1), "TypeError");
  throwsNatively(() => ops.lt(1, Reroll), "TypeError");
  assert.equal(native(() => ops.eq(Restart, Restart)), true);
  assert.equal(native(() => ops.ne(Reroll, Restart)), true);
  assert.equal(native(() => ops.eq(1, Reroll)), false);
  throwsNatively(() => ops.abs(Reroll), "TypeError");
  throwsNatively(() => ops.floor(Reroll), "TypeError");
  throwsNatively(() => ops.matmul(new Float(1.5), "a"), "TypeError");
  throwsNatively(() => ops.round(Reroll), "TypeError");
  throwsNatively(() => ops.round(3, d(2)), "TypeError");
  assert.equal(native(() => ops.mod("xy", new Die([]))), "xy");
  assert.equal(native(() => ops.floordiv(new Symbols("aabbb"), new Die(["a", "b", "b"]))), 1);
  throwsNatively(() => ops.floordiv(new Symbols("aab"), new Die([])), "ZeroDivisionError");
  // `(d2, 1) + d2`: `Die.__radd__` makes a die of tuples, and a tuple plus an int raises.
  throwsNatively(() => ops.add([d(2), 1], d(2)), "TypeError");
});

test("map and Deck.map: Python's errors for what is neither a function nor a Map", () => {
  throwsNatively(() => map("x", d(2)), "TypeError");
  throwsNatively(() => map(4), "TypeError");
  throwsNatively(() => map(new Map([[1, 5]]), d(2), d(2)), "ValueError");
  throwsNatively(() => map(4, 1, { repeat: 2 }), "TypeError");
  throwsNatively(() => new Deck([1, 2]).map((a, b) => a + b, { star: true }), "TypeError");
  assert.equal(native(() => new Deck([[1, 2], [3, 4]]).map((a, b) => a + b).repr()), "Deck({3: 1, 7: 1})");
  throwsNatively(() => new Deck([1, 2, 2]).split(null), "TypeError");
});
