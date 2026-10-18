// What task C6i moved off the JSON-op bridge: the keep family of Die (highest, lowest, middle, keep, keepSlice),
// the free highest / lowest / middle, the to-pool methods (explodeToPool, rerollToPool, mapToPool) and the free
// mapToPool, and the arguments only the bridge modelled before: multiset expressions and decks given to map & co.,
// tupleize / vectorize and `new Die(...)`, and a deck's toOneHot. Expected values are Python icepool's
// (fixtures/c6i/edge-cases.jsonl has them as programs). Each also checks that it made no call through the bridge.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as ip from "../dist/node.js";
import { bridgeCalls } from "../dist/internal.js";

const { Deck, Die, Vector, d, highest, lowest, map, mapAndTime, mapToPool, meanTimeToAbsorb, middle, tupleize, vectorize } = ip;

/** Runs `f`, asserting that it called nothing through the bridge. */
function native(f) {
  const total = () => [...bridgeCalls.values()].reduce((a, b) => a + b, 0);
  const before = total();
  const result = f();
  assert.equal(total(), before, `bridge calls: ${JSON.stringify([...bridgeCalls])}`);
  return result;
}

/** A die's (or deck's) items with plain-number quantities, Vectors as their elements. */
const items = (x) => x.items().map(([o, q]) => [o instanceof Vector ? [...o] : o, Number(q)]);

/** Asserts that `f` throws Python's exception `pythonType` (and the message starts with `message`, if given). */
function throwsPython(f, pythonType, message) {
  assert.throws(() => native(f), (e) => {
    assert.equal(e.pythonType, pythonType, e.message);
    if (message !== undefined) assert.ok(e.message.startsWith(message), e.message);
    return true;
  });
}

test("Die.highest / lowest: the single die on the core, else a kept pool", () => {
  assert.deepEqual(items(native(() => d(6).highest(3))), [[1, 1], [2, 7], [3, 19], [4, 37], [5, 61], [6, 91]]);
  assert.deepEqual(
    items(native(() => d(6).highest(3, 2))),
    [[2, 1], [3, 3], [4, 7], [5, 12], [6, 19], [7, 27], [8, 34], [9, 36], [10, 34], [11, 27], [12, 16]],
  );
  assert.deepEqual(items(native(() => d(6).lowest(3, 1, 1))), [[1, 16], [2, 40], [3, 52], [4, 52], [5, 40], [6, 16]]);
  // No rolls: the sum of nothing.
  assert.deepEqual(items(native(() => d(6).lowest(0))), [[0, 1]]);
  throwsPython(() => d(6).highest(-1), "ValueError", "times cannot have negative values.");
  throwsPython(() => d(6).highest(3, -1), "ValueError");
  // A bool is an int.
  assert.ok(native(() => d(6).highest(true)).equals(d(6)));
});

test("Die.middle / keep / keepSlice", () => {
  const middle3 = [[1, 16], [2, 40], [3, 52], [4, 52], [5, 40], [6, 16]];
  assert.deepEqual(items(native(() => d(6).middle(3))), middle3);
  throwsPython(() => d(6).middle(4, 1), "IndexError");
  const top2 = [[2, 1], [3, 3], [4, 7], [5, 12], [6, 19], [7, 27], [8, 34], [9, 36], [10, 34], [11, 27], [12, 16]];
  assert.deepEqual(items(native(() => d(6).keep([0, 1, 1]))), top2);
  assert.deepEqual(items(native(() => d(6).keepSlice(3, 1))), top2);
  assert.deepEqual(items(native(() => d(6).keep(3, 0))), [[1, 91], [2, 61], [3, 37], [4, 19], [5, 7], [6, 1]]);
  throwsPython(() => d(6).keep(3), "ValueError", "If the number of rolls is an integer, an index argument must be provided.");
  throwsPython(() => d(6).keep([1, 1], 0), "ValueError", "Only one index sequence can be given.");
  throwsPython(() => d(6).keepSlice([1, 1], 0), "ValueError", "Only one index sequence can be given.");
});

test("free highest / lowest / middle", () => {
  assert.deepEqual(items(native(() => highest(d(6), d(8)))), [[1, 1], [2, 3], [3, 5], [4, 7], [5, 9], [6, 11], [7, 6], [8, 6]]);
  assert.deepEqual(
    items(native(() => highest(d(6), d(8), d(4), { keep: 2 }))),
    [[2, 1], [3, 3], [4, 7], [5, 12], [6, 18], [7, 23], [8, 27], [9, 28], [10, 26], [11, 20], [12, 15], [13, 8], [14, 4]],
  );
  // Keeping none: the sum of the zeros, with the denominator of all the dice.
  assert.deepEqual(items(native(() => highest(d(6), d(8), { keep: 0 }))), [[0, 48]]);
  assert.deepEqual(items(native(() => lowest([d(6), d(8)]))), [[1, 13], [2, 11], [3, 9], [4, 7], [5, 5], [6, 3]]);
  // One argument is an iterable: a die's outcomes.
  assert.deepEqual(items(native(() => highest(d(6)))), [[6, 1]]);
  throwsPython(() => highest(3), "TypeError", "object of type 'int' has no len()");
  assert.ok(native(() => highest(d(6), new Die([]))).isEmpty());
  // `middle` makes a pool of its one argument itself.
  throwsPython(() => middle(d(6)), "ValueError", "A Pool cannot be constructed with a Die argument.");
  assert.deepEqual(items(native(() => middle([], { default: 3 }))), [[3, 1]]);
  throwsPython(() => middle([]), "ValueError", "middle() arg is an empty sequence and no default was provided.");
});

test("explodeToPool / rerollToPool", () => {
  const exploded = native(() => d(6).explodeToPool(2, [6], { depth: 2 }).sum());
  assert.equal(exploded.denominator(), 6n ** 6n);
  assert.deepEqual(items(exploded).slice(0, 3), [[2, 1296], [3, 2592], [4, 3888]]);
  assert.deepEqual(
    items(native(() => d(6).rerollToPool(3, [1], { maxRerolls: 1 }).sum())),
    [[3, 1], [4, 4], [5, 10], [6, 25], [7, 49], [8, 82], [9, 120], [10, 159], [11, 180], [12, 183], [13, 168], [14, 135], [15, 90], [16, 54], [17, 27], [18, 9]],
  );
  assert.deepEqual(
    items(native(() => d(6).rerollToPool(3, [1, 2], { maxRerolls: 1, mode: "low" }).sum())),
    [[3, 1], [4, 4], [5, 11], [6, 23], [7, 41], [8, 65], [9, 97], [10, 130], [11, 159], [12, 183], [13, 183], [14, 159], [15, 120], [16, 72], [17, 36], [18, 12]],
  );
  // Without a limit: `die.reroll(outcomes, depth=depth).pool(rolls)`.
  assert.ok(native(() => d(6).rerollToPool(3, [1, 2], { maxRerolls: Infinity }).sum()).equals(d(6).reroll([1, 2], { depth: 1 }).rmatmul(3)));
  throwsPython(() => d(6).rerollToPool(3, [1, 2], { maxRerolls: 1, mode: "x" }).sum(), "ValueError", "Invalid reroll_priority 'x'.");
  throwsPython(() => d(6).explodeToPool(1, [6], { depth: 1.5 }), "TypeError", "'float' object cannot be interpreted as an integer");
});

test("mapToPool and Die.mapToPool", () => {
  assert.deepEqual(items(native(() => new Die([[1, 2], [3]]).mapToPool().sum())), [[3, 2]]);
  throwsPython(() => new Die([[1, 2], [3]]).mapToPool(null, { a: 1 }), "TypeError", "Die.map_to_pool.<locals>.<lambda>() got an unexpected keyword argument 'a'");
  // A die is no pool's contents.
  throwsPython(() => mapToPool((x) => d(x), d(2)), "ValueError", "A Pool cannot be constructed with a Die argument.");
  // Multiset expressions and decks among the arguments expand.
  assert.deepEqual(items(native(() => mapToPool((x) => [...x], d(2).pool(2)).sum())), [[2, 1], [3, 2], [4, 1]]);
  assert.deepEqual(items(native(() => mapToPool((x) => [x], new Deck([1, 2, 2])).sum())), [[1, 1], [2, 2]]);
  const pools = new Map([[1, [d(6)]], [2, [d(6), d(6)]]]);
  assert.equal(native(() => mapToPool(pools, d(2)).sum().mean()).toString(), "21/4");
  throwsPython(() => mapToPool(pools, d(2), d(2)), "ValueError", "If a mapping is provided for repl, len(args) must be 1.");
});

test("map & co. of multiset expressions and decks", () => {
  const pool = d(2).pool(2);
  assert.deepEqual(items(native(() => map((x) => x + 1, new Deck([1, 2, 2])))), [[2, 1], [3, 2]]);
  assert.deepEqual(items(native(() => map((x) => x[0] + x[1], pool))), [[2, 1], [3, 2], [4, 1]]);
  assert.deepEqual(items(native(() => map((a, b) => a + b, pool))), [[2, 1], [3, 2], [4, 1]]);
  assert.deepEqual(items(native(() => map((a, b) => a + b[0] + b[1], 1, pool))), [[3, 1], [4, 2], [5, 1]]);
  assert.deepEqual(items(native(() => map((a) => a, pool, { repeat: 2 }))), [[[1, 1], 1], [[1, 2], 2], [[2, 2], 1]]);
  assert.deepEqual(items(native(() => d(2).map((a, b) => a + b[0] + b[1], pool))), [[3, 1], [4, 3], [5, 3], [6, 1]]);
  assert.deepEqual(items(native(() => mapAndTime((a) => a, new Deck([1, 2]), { repeat: 2 }))), [[[1, 0], 1], [[2, 0], 1]]);
  assert.equal(native(() => meanTimeToAbsorb((a) => a, new Deck([1, 2]))).toString(), "0");
});

test("new Die(...), tupleize and vectorize with decks and multiset expressions", () => {
  const pool = d(2).pool(2);
  assert.deepEqual(items(native(() => new Die([new Deck([1, 2, 2]), 3]))), [[1, 1], [2, 2], [3, 3]]);
  assert.deepEqual(items(native(() => new Die([[new Deck([1, 2]), 1]]))), [[[1, 1], 1], [[2, 1], 1]]);
  throwsPython(() => new Die([[new Deck([1, 2]), d(2)]]), "TypeError", "Arguments to vector() of type Population must all be Die or all be Deck");
  // A pool is an outcome no die can sort, unless its weight drops it first.
  throwsPython(() => new Die([pool, 1]), "TypeError", "Items do not appear to be sortable.");
  assert.deepEqual(items(native(() => new Die([pool, 1], [0, 1]))), [[1, 1]]);
  const deckTuples = native(() => tupleize(new Deck([1, 2]), 1));
  assert.ok(deckTuples instanceof Deck);
  assert.deepEqual(items(deckTuples), [[[1, 1], 1], [[2, 1], 1]]);
  assert.deepEqual(items(native(() => vectorize(new Deck([1, 2]), new Deck([3, 3])))), [[[1, 3], 2], [[2, 3], 2]]);
  // Without a population, a pool is just an element of the tuple.
  assert.equal(native(() => tupleize(pool, 1))[0], pool);
  assert.deepEqual(items(native(() => tupleize(pool, d(2)))).slice(0, 2), [[[[1, 1], 1], 1], [[[1, 1], 2], 1]]);
});

test("Deck.toOneHot", () => {
  assert.deepEqual(items(native(() => new Deck([1, 2, 2]).toOneHot())), [[[false, true], 2], [[true, false], 1]]);
  const hot = native(() => new Deck([1, 2, 2]).toOneHot([1, 2, 3]));
  assert.ok(hot instanceof Deck);
  assert.deepEqual(items(hot), [[[false, true, false], 2], [[true, false, false], 1]]);
});
