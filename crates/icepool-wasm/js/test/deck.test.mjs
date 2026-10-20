// The parts of the API task C6f moved off the JSON-op bridge onto typed exports: Deck (construction, operators,
// map, statistics, sequence, the mapping protocol), Deal and MultiDeal (keeping, hands, slices, info, str/repr),
// multiset functions and JS evaluators over them, and Wallenius. Expected values are Python icepool's. Each also
// checks that it made no call through the bridge.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Deck, Die, Float, Fraction, MultisetExpression, Reroll, Wallenius, d, evaluator, multisetFunction } = ip;

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
const throwsNatively = (f, name) => native(() => assert.throws(f, pythonType(name)));

test("Deck: construction as Deck.__new__ (itemize, expand_arg, merge_duplicates)", () => {
  // A Map is a Python dict: `{1: 2, True: 1, 2.0: 3}` is `{1: 1, 2.0: 3}`.
  assert.equal(repr(() => new Deck(new Map([[1, 2], [true, 1], [new Float(2), 3]]))), "Deck({1: 1, 2.0: 3})");
  assert.equal(repr(() => new Deck("aab")), "Deck({'a': 2, 'b': 1})");
  assert.equal(repr(() => new Deck(d(3))), "Deck({1: 1, 2: 1, 3: 1})");
  const deck = new Deck([1, 2, 3], 4);
  assert.equal(repr(() => new Deck([deck, 5], [2, 1])), "Deck({1: 8, 2: 8, 3: 8, 5: 1})");
  // A tuple holding a die is their Cartesian product; one holding Reroll drops out, like Reroll.
  assert.equal(repr(() => new Deck([[1, d(2)]])), "Deck({(1, 1): 1, (1, 2): 1})");
  assert.equal(repr(() => new Deck([[1, Reroll]], [0])), "Deck({})");
  assert.equal(repr(() => new Deck([[1, Reroll], 2])), "Deck({2: 1})");
  throwsNatively(() => new Deck([1], [1.5]), "ValueError");
  assert.equal(repr(() => new Deck([1], [new Float(0)])), "Deck({})");
  throwsNatively(() => new Deck([1, 2], [1, -1]), "ValueError");
  // `Deck([deck])` is the deck itself.
  assert.equal(native(() => new Deck([deck])), deck);
  assert.equal(repr(() => new Deck()), "Deck({})");
});

test("Deck: operators with Counter(other) of anything iterable", () => {
  const deck = new Deck([1, 2]);
  assert.equal(repr(() => deck.add([2, 3])), "Deck({1: 1, 2: 2, 3: 1})");
  assert.equal(repr(() => deck.add(d(2))), "Deck({1: 2, 2: 2})");
  assert.equal(repr(() => deck.add(new Map([[2, 3]]))), "Deck({1: 1, 2: 4})");
  assert.equal(repr(() => new Deck(["a"]).add("ab")), "Deck({'a': 2, 'b': 1})");
  assert.equal(repr(() => deck.add(null)), "Deck({1: 1, 2: 1})");
  throwsNatively(() => deck.add(3), "TypeError");
  // A multiset expression is iterated into its items, dice, whose outcomes the deck takes (or not).
  assert.equal(repr(() => deck.add(d(3).pool(1))), "Deck({1: 2, 2: 2, 3: 1})");
  assert.equal(repr(() => deck.sub(d(3).pool(1))), "Deck({1: 1, 2: 1})");
  assert.equal(repr(() => new Deck([1, 1, 2]).xor([1, 3])), "Deck({1: 1, 2: 1, 3: 1})");
  assert.equal(repr(() => new Deck([1]).rsub([1, 1, 2])), "Deck({1: 1, 2: 1})");
  assert.equal(repr(() => deck.rmul(3)), "Deck({1: 3, 2: 3})");
  throwsNatively(() => deck.mul(1.5), "TypeError");
  assert.equal(repr(() => new Deck([1, 2, 2, 2]).mod(2)), "Deck({1: 1, 2: 1})");
  // `Pool(deck)` is refused, so a deck cannot be the right operand of an expression's operator.
  throwsNatively(() => d(3).pool(1).add(deck), "ValueError");
});

test("Deck: map", () => {
  assert.equal(repr(() => new Deck([1, 2, 2]).map((x) => x * 10)), "Deck({10: 1, 20: 2})");
  assert.equal(repr(() => new Deck([[1, 2], [3, 4]]).map((a, b) => a + b)), "Deck({3: 1, 7: 1})");
  assert.equal(repr(() => new Deck([1, 2, 2]).map(new Map([[2, "b"], [true, "a"]]))), "Deck({'a': 1, 'b': 2})");
  assert.equal(repr(() => new Deck([1, 2]).map((x) => new Deck([x, x + 10]))), "Deck({1: 1, 2: 1, 11: 1, 12: 1})");
  assert.equal(repr(() => new Deck([1, 2]).map((x) => (x === 1 ? Reroll : x))), "Deck({2: 1})");
});

test("Deck: statistics, the mapping protocol, ==, str", () => {
  const deck = new Deck([1, 2, 2, 3]);
  native(() => {
    assert.deepEqual(deck.mean(), new Fraction(2n, 1n));
    assert.deepEqual(deck.variance(), new Fraction(1n, 2n));
    assert.equal(deck.sd(), 0.7071067811865476);
    assert.deepEqual(deck.stat("mode"), [2]);
    assert.equal(deck.length, 3);
    assert.equal(deck.size(), 4n);
    assert.equal(deck.get(2), 2n);
    assert.deepEqual([...deck], [1, 2, 3]);
    assert.deepEqual(deck.values(), [1n, 2n, 1n]);
    assert.ok(deck.eq(new Deck(new Map([[1, 1], [2, 2], [3, 1]]))));
    assert.ok(deck.ne(new Deck([1])));
    // A die is not a deck.
    assert.ok(!deck.equals(new Die([1, 2, 2, 3])));
    assert.deepEqual(new Deck([1, 2]).toString().split("\n").slice(0, 3), ["Deck with denominator 2", "", "| Outcome | Quantity | Probability |"]);
  });
  throwsNatively(() => deck.get(5), "KeyError");
  throwsNatively(() => deck.get(ip.List.from([1])), "TypeError");
});

test("Deck: sequence (with Python's cache), padToDenominator, append, remove, split, zero", () => {
  const cards = new Deck(["a", "a", "b"]);
  assert.equal(repr(() => cards.sequence(2)), "Die({('a', 'a'): 2, ('a', 'b'): 2, ('b', 'a'): 2})");
  // Python only binds `result` for sequences longer than any computed so far.
  throwsNatively(() => cards.sequence(1), "UnboundLocalError");
  assert.equal(repr(() => cards.sequence(3)), "Die({('a', 'a', 'b'): 2, ('a', 'b', 'a'): 2, ('b', 'a', 'a'): 2})");
  throwsNatively(() => new Deck([1]).sequence(-1), "ValueError");
  const deck = new Deck([1, 2, 2, 3]);
  assert.equal(repr(() => deck.padToDenominator(6, 9)), "Deck({1: 1, 2: 2, 3: 1, 9: 2})");
  throwsNatively(() => deck.padToDenominator(2, 9), "ValueError");
  assert.equal(repr(() => deck.append(4, 2)), "Deck({1: 1, 2: 2, 3: 1, 4: 2})");
  assert.equal(repr(() => deck.append(2, -5)), "Deck({1: 1, 3: 1})");
  assert.equal(repr(() => deck.remove(2)), "Deck({1: 1, 3: 1})");
  assert.equal(repr(() => deck.remove(2, 1)), "Deck({1: 1, 2: 1, 3: 1})");
  assert.deepEqual(native(() => deck.split((x) => x >= 2).map((x) => x.repr())), ["Deck({2: 2, 3: 1})", "Deck({1: 1})"]);
  assert.deepEqual(native(() => deck.split([1, 3]).map((x) => x.repr())), ["Deck({1: 1, 3: 1})", "Deck({2: 2})"]);
  // A str selects by Python's substring `in`.
  assert.deepEqual(native(() => new Deck(["a", "ab", "c"]).split("abc").map((x) => x.repr())), ["Deck({'a': 1, 'ab': 1, 'c': 1})", "Deck({})"]);
  assert.equal(repr(() => deck.zero()), "Deck({0: 4})");
});

test("Deal: keeping, info, str and repr, evaluations", () => {
  const deal = new Deck([1, 2, 3], 4).deal(3);
  native(() => {
    assert.equal(deal.denominator(), 220n);
    assert.deepEqual(deal.highest(2).keepTuple(), [0, 1, 1]);
    assert.ok(deal.highest(2) instanceof ip.Deal);
    assert.equal(deal.highest(2).repr(), "Deal(Deck({1: 4, 2: 4, 3: 4}), hand_size=3)");
    assert.equal(deal.toString().split("\n")[0], "Deal of hand_size=3 from deck:");
    assert.ok(deal.deck().eq(new Deck([1, 2, 3], 4)));
  });
  assert.equal(repr(() => deal.highest(2).sum()), "Die({2: 4, 3: 24, 4: 52, 5: 88, 6: 52})");
  // A deal plus a pool: a compound keep.
  assert.equal(repr(() => deal.add(d(2).pool(1)).sum()), "Die({4: 4, 5: 28, 6: 72, 7: 116, 8: 116, 9: 72, 10: 28, 11: 4})");
  throwsNatively(() => new Deck([1]).deal(-1), "ValueError");
  throwsNatively(() => new Deck([1]).deal(2), "ValueError");
});

test("MultiDeal: hands, slices, info, str and repr, deal groups", () => {
  const md = new Deck([1, 2, 3, 4]).deal([1, 3]);
  native(() => {
    assert.deepEqual([md.handSizes(), md.totalCardsDealt(), md.length, md.denominator()], [[1, 3], 4, 2, 4n]);
    assert.equal(md.slice(1).length, 1);
    assert.equal(md.repr(), "MultiDeal(Deck({1: 1, 2: 1, 3: 1, 4: 1}), hand_groups=((1, 1), (3, 1)))");
    assert.equal(md.toString().split("\n")[0], "MultiDeal of hand_groups=((1, 1), (3, 1)) from deck:");
    assert.equal(new Deck(["A", "K"], 2).dealGroups([1, 2]).denominator(), 12n);
    // A non-int index falls off the end of `__getitem__`.
    assert.equal(md.hand("a"), null);
  });
  assert.equal(repr(() => md.hand(1).sum()), "Die({6: 1, 7: 1, 8: 1, 9: 1})");
  assert.equal(repr(() => md.slice(-1).hand(0).size()), "Die({3: 4})");
  throwsNatively(() => new Deck([1]).deal(null), "TypeError");
  assert.equal(repr(() => new Deck([1]).deal("")), "MultiDeal(Deck({1: 1}), hand_groups=())");
});

test("MultiDeal: multiset functions and JS evaluators over it (tuple counts)", () => {
  const diff = multisetFunction((hands) => hands.hand(0).sub(hands.hand(1)).size());
  assert.equal(repr(() => diff.call(new Deck([1, 2, 3, 4]).deal([2, 2]))), "Die({2: 6})");
  const both = multisetFunction((a, b) => [a.add(b).sum(), a.size()]);
  assert.equal(repr(() => both.call(new Deck([1, 2, 3]).deal([1, 2]), { star: true })), "Die({(6, 1): 3})");
  // Only a multiset tuple expression has a length to unpack.
  throwsNatively(() => both.call(d(3).pool(1), { star: true }), "TypeError");
  const counts = evaluator({ nextState: (s, o, x, c) => [...(s ?? []), c] });
  assert.equal(repr(() => counts.evaluate(new Deck([1, 2]).deal([1, 1]))), "Die({((0, 1), (1, 0)): 1, ((1, 0), (0, 1)): 1})");
  // An operator over a whole MultiDeal gets its tuple counts; `keep_outcomes` of one keeps nothing.
  assert.equal(repr(() => counts.evaluate(MultisetExpression.union(new Deck([1, 2]).deal([1, 1])))), "Die({((0, 1), (1, 0)): 1, ((1, 0), (0, 1)): 1})");
  assert.equal(repr(() => d(3).pool(2).keepOutcomes(new Deck([1, 2]).deal([1, 1])).sum()), "Die({0: 9})");
});

test("Wallenius natively", () => {
  assert.equal(repr(() => new Wallenius([[1, 2]]).deal(0).expand()), "Die({(): 1})");
  assert.equal(repr(() => new Wallenius(new Map([["a", 1], ["b", [2, 3]]])).deal(2).expand()), "Die({('a', 'b'): 66, ('b', 'b'): 144})");
  assert.equal(repr(() => new Wallenius([["a", 1], ["b", 2]]).deal(-1).expand()), "Die({})");
  throwsNatively(() => new Wallenius([[1, 2, 3]]), "ValueError");
  throwsNatively(() => new Wallenius([[1, ip.List.from([2])]]), "TypeError");
});
