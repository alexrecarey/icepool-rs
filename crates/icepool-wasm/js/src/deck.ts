// Decks and deals (population/deck.py, generator/deal.py, generator/multi_deal.py, wallenius.py) over the core's
// decks (src/api/deck.rs `WasmDeck`, `WasmMultiDeal`). As the Python facade keeps upstream's Python juggling, the
// TypeScript does Python's argument handling here and calls the typed exports for the work. A `Deal` is a keep
// generator of the core (src/api/multiset.rs) and a `MultiDeal` a multiset tuple expression: both are nodes of
// multiset.ts's expression tree, so every evaluation takes them natively.
//
// Arguments the program runner (crates/icepool-harness/src/deck.rs) does not model either (pools among a deck's
// cards, dice counted by an operator, non-int hand sizes, ...) raise its `NotImplementedError` (`unported`).

import type { WasmDeck, WasmGenerator, WasmMultiDeal } from "../pkg-node/icepool_wasm.js";
import { AgainExpression } from "./again.js";
import { IcepoolError } from "./errors.js";
import { Die, dictItems, isOne, itemize, pyBool, pyIter, unpack, type Callback, type MapResult, type Which } from "./die.js";
import { mappingGet, ops, tupleize } from "./functions.js";
import {
  KeepGenerator,
  MultisetExpression,
  Pool,
  generator,
  generatorOf,
  hasParameter,
  isMultisetExpression,
  keepTupleBounded,
  mixtureOf,
  registerDeckClasses,
  tupleIndex,
  tupleLen,
  type TupleNode,
} from "./multiset.js";
import { List, Reroll, Restart, Special, isPlainValue, pyTypeName, type Int, type Outcome, type PyFloat } from "./values.js";
import { wasm } from "./wasm.js";

/** What `new Deck(...)` takes: outcomes, dice or decks (with times), or a Map of outcomes to counts.
 *
 * In an array, duplicates add a card for each appearance, and a `Deck` is flattened into the result (a `times`
 * for it repeats the whole deck).
 *
 * @example
 * ```ts
 * import { Deck, type DeckOutcomes } from "icepool-wasm";
 * const cards: DeckOutcomes = ["A", "A", "K"];
 * new Deck(cards).items(); //=> [["A", 2n], ["K", 1n]]
 * new Deck(new Map([["A", 2], ["K", 1]])).items(); //=> [["A", 2n], ["K", 1n]]
 * ```
 */
export type DeckOutcomes = readonly (Outcome | Die | Deck)[] | Map<Outcome, Int>;

const typeError = (message: string) => new IcepoolError("WrongType", "TypeError", message);
const valueError = (message: string) => new IcepoolError("InvalidArgument", "ValueError", message);
/** What the port does not model (the program runner's `Unimplemented`). */
const unported = (what: string) => new IcepoolError("Unported", "NotImplementedError", what);

/** A hand size or group size of a `MultiDeal`: an exact int. (A bool would work in Python but show up as `True` in
 * the hand sizes and the repr, which the core does not keep.) */
function handInt(h: unknown): Int {
  if (isExactInt(h)) return h;
  throw unported("a MultiDeal hand size that is not an int");
}

/** Python's `isinstance(x, int)` (so also a bool). */
const isIntArg = (x: unknown): x is Int | boolean =>
  typeof x === "bigint" || typeof x === "boolean" || (typeof x === "number" && Number.isInteger(x));
/** An exact int (not a bool), as a hand size of a `MultiDeal` must be (the core keeps no `True` to show). */
const isExactInt = (x: unknown): x is Int => typeof x === "bigint" || (typeof x === "number" && Number.isInteger(x));
/** An int argument as the core takes it (a bool as 0 or 1). */
const intOf = (x: Int | boolean): Int => (typeof x === "boolean" ? Number(x) : x);
/** An int as Python returns it: a number when it is safe. */
const pyInt = (x: Int): Int => (Number.isSafeInteger(Number(x)) ? Number(x) : x);

/** `icepool.Deck`: `new Deck([1, 2, 3], 4)` (each card 4 times), `new Deck(new Map([["a", 2]]))`.
 *
 * Sampling without replacement (within a single evaluation): the quantities are duplicate cards. Deal from it with
 * {@link Deck.deal}. All quantities must be non-negative; outcomes with zero quantity are left out.
 *
 * @throws `IcepoolError` (`ValueError`) for an `Again` among the outcomes.
 *
 * @example
 * ```ts
 * const cards = new Deck([1, 2, 3], 4); // 12 cards, four of each
 * cards.items(); //=> [[1, 4n], [2, 4n], [3, 4n]]
 * cards.size(); //=> 12n
 * cards.deal(2).sum().probability(6).toString(); //=> "1/11"
 * // @ts-expect-error: an Again is not a card.
 * new Deck([1, Again]); //=> throws ValueError
 * ```
 */
export class Deck implements Iterable<Outcome>, Disposable {
  /** The core's deck; `null` once freed. */
  private declare core: WasmDeck | null;
  /** Python's `len(self._sequence_cache) - 1`: the most cards `sequence` has dealt so far. */
  private declare dealt: number;

  /** `Deck(outcomes=(), times=1)`: `times` multiplies each element of `outcomes`, one number for all of them or an
   * array as long as `outcomes`. */
  constructor(outcomes: DeckOutcomes = [], times: Int | readonly Int[] = 1) {
    // `contains_again(outcomes)`: a sequence's elements, a mapping's keys.
    const keys = outcomes instanceof Map ? [...outcomes.keys()] : Array.isArray(outcomes) ? outcomes : [];
    if (keys.some((x) => x instanceof AgainExpression)) throw valueError("Again cannot be used with Decks.");
    const made = makeDeck(outcomes, times);
    // `Deck([deck])` is the deck itself, as in Python.
    if (made instanceof Deck) return made;
    this.core = made;
    this.dealt = 0;
  }

  /** Wraps a core deck. @internal */
  static wrap(core: WasmDeck): Deck {
    const deck = Object.create(Deck.prototype) as Deck;
    deck.core = core;
    deck.dealt = 0;
    return deck;
  }

  /** The core deck, which must not have been freed. @internal */
  get w(): WasmDeck {
    if (this.core === null) throw new TypeError("icepool: this deck was freed");
    return this.core;
  }

  /** The cards as a die (the `Population` methods a deck shares with a die). */
  private population(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.population()));
  }

  /** Frees the deck's WASM memory now rather than when it is garbage-collected. Using it afterwards throws a
   * `TypeError`; freeing it again does nothing. Deals and decks made from it are not affected.
   *
   * @example
   * ```ts
   * const cards = new Deck([1, 2, 3]);
   * cards.free();
   * cards.size(); //=> throws TypeError
   * ```
   */
  free(): void {
    this.core?.free();
    this.core = null;
  }

  /** `free()`, for `using cards = new Deck(...);`. */
  [Symbol.dispose](): void {
    this.free();
  }

  /** `Deck.deal(hand_sizes)`: deals cards from this deck: a {@link Deal} of `handSize` cards, or a
   * {@link MultiDeal} for an array of hand sizes. For algorithmic reasons you commit to the number of cards up
   * front.
   *
   * @throws `IcepoolError` (`ValueError`) if more cards are dealt than the deck has, or a hand size is negative.
   *
   * @example
   * ```ts
   * const cards = new Deck([1, 2, 3], 4);
   * cards.deal(2).sum().probability(6).toString(); //=> "1/11"
   * cards.deal([1, 2]).handSizes(); //=> [1, 2]
   * new Deck([1, 2]).deal(3); //=> throws ValueError
   * ```
   */
  deal(handSize: Int): Deal;
  deal(handSizes: readonly Int[]): MultiDeal;
  deal(handSize: Int | readonly Int[]): Deal | MultiDeal {
    // `isinstance(hand_sizes, int)`: a `Deal`; anything else is an iterable of hand sizes, a `MultiDeal` with one
    // group `(hand_size, 1)` per hand.
    if (isIntArg(handSize)) {
      const w = this.w;
      return generator(wasm(() => w.deal(intOf(handSize)))) as Deal;
    }
    // A str is an iterable of its characters; anything else not iterable raises.
    const sizes: unknown = typeof handSize === "string" ? [...handSize] : handSize;
    if (Array.isArray(sizes)) {
      // `sum(hand_size * group_size ...)` starts from 0: `0 + 'a'` is a TypeError.
      if (sizes.some((h) => typeof h === "string")) throw typeError("unsupported operand type(s) for +: 'int' and 'str'");
      return this.multiDeal(sizes.flatMap((h) => [handInt(h), 1]));
    }
    if (isPlainValue(handSize)) throw typeError(`'${pyTypeName(handSize)}' object is not iterable`);
    throw unported("these MultiDeal hand sizes");
  }
  /** `Deck.deal_groups(*hand_groups)` (EXPERIMENTAL in icepool): deals cards into groups of hands, where the hands
   * in each group could be produced in any order. Each argument is `[handSize, groupSize]`: the number of cards in
   * each hand of the group and the number of hands in the group.
   *
   * @example
   * ```ts
   * // Two indistinguishable hands of two cards each.
   * const deal = new Deck(["A", "K", "Q"], 4).dealGroups([2, 2]);
   * deal.handSizes(); //=> [2, 2]
   * deal.denominator(); //=> 2970n
   * ```
   */
  dealGroups(...groups: [Int, Int][]): MultiDeal {
    // `for hand_size, group_size in self._hand_groups`: anything but a pair does not unpack.
    const pairs = groups.map((g) => pyIter(g));
    const odd = pairs.find((g) => g.length !== 2);
    if (odd !== undefined) throw valueError(`expected 2 values to unpack, got ${odd.length}`);
    return this.multiDeal(pairs.flat().map(handInt));
  }
  /** `MultiDeal(self, hand_groups)` of `[hand_size, group_size, ...]`. */
  private multiDeal(groups: Int[]): MultiDeal {
    const w = this.w;
    return MultiDeal.native({ kind: "multiDeal", md: wasm(() => w.multiDeal(groups)) });
  }
  /** `Deck.map(repl, star=None)`: maps each card of this deck to another outcome, keeping its quantity.
   *
   * @param f - A function returning the new outcome for each old outcome, or a Map from old outcomes to new ones
   *   (unmapped outcomes stay the same). The new outcomes may be decks, in which case one card is replaced with
   *   several (not recommended).
   * @param options - Keyword arguments.
   * @param options.star - Whether outcomes are unpacked into separate arguments of `f`; `null` guesses from `f`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).map((x) => 10 * x).items(); //=> [[10, 4n], [20, 4n], [30, 4n]]
   * new Deck([1, 2, 3]).map(new Map([[1, 5]])).items(); //=> [[2, 1n], [3, 1n], [5, 1n]]
   * ```
   */
  map(f: Callback<MapResult> | Map<Outcome, MapResult>, { star = null }: { star?: boolean | null } = {}): Deck {
    const outcomes = this.outcomes();
    let results: unknown[];
    if (f instanceof Map) {
      // `repl[outcome] if outcome in repl else outcome`: a dict lookup.
      const values = mappingGet(f, outcomes);
      results = outcomes.map((o) => values.get(o));
    } else if (typeof f === "function") {
      const fn = f as (...args: unknown[]) => unknown;
      // `infer_star(repl)`: more than one required positional parameter.
      const starred = star === null || star === undefined ? fn.length > 1 : pyBool(star);
      results = outcomes.map((o) => (starred ? fn(...unpack(o)) : fn(o)));
    } else {
      // Python treats anything else as a mapping (`outcome in repl`, `repl[outcome]`).
      throw new IcepoolError("Unported", "NotImplementedError", "deck.map with this repl");
    }
    // `Deck([transition_function(outcome) for outcome in self.outcomes()], times=self.quantities())`.
    return new Deck(results as DeckOutcomes, this.values());
  }

  // Binary operators: `self op Counter(other)`, then `Deck(+data)`.

  /** `deck + other`: both decks merged together (the quantities add). `other` is an array of outcomes or a deck.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).add([1, 4]).items(); //=> [[1, 5n], [2, 4n], [3, 4n], [4, 1n]]
   * ```
   */
  add(other: unknown): Deck { return this.counterOp("add", other); }
  /** `deck - other`: this deck with the other cards removed (but not below zero of each card).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).sub([1, 1, 1]).items(); //=> [[1, 1n], [2, 4n], [3, 4n]]
   * ```
   */
  sub(other: unknown): Deck { return this.counterOp("sub", other); }
  /** `deck & other`: the cards that both decks have.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).and([1, 1, 5]).items(); //=> [[1, 2n]]
   * ```
   */
  and(other: unknown): Deck { return this.counterOp("and", other); }
  /** `deck | other`: as many of each card as the deck that has more of them.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).or([1, 1, 1, 1, 1]).items(); //=> [[1, 5n], [2, 4n], [3, 4n]]
   * ```
   */
  or(other: unknown): Deck { return this.counterOp("or", other); }
  /** `deck ^ other`: of each card, the difference between the two decks' quantities.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).xor([1, 1, 1, 1, 1, 2]).items(); //=> [[1, 1n], [2, 3n], [3, 4n]]
   * ```
   */
  xor(other: unknown): Deck { return this.counterOp("xor", other); }
  /** `other + deck` (`__radd__`, the same as `__add__`: this deck's cards first).
   *
   * @example
   * ```ts
   * new Deck([1, 2]).radd([3]).items(); //=> [[1, 1n], [2, 1n], [3, 1n]]
   * ```
   */
  radd(other: unknown): Deck { return this.counterOp("add", other); }
  /** `other - deck` (`__rsub__`): the other cards with this deck's removed (but not below zero of each card).
   *
   * @example
   * ```ts
   * new Deck([1]).rsub([1, 1, 2]).items(); //=> [[1, 1n], [2, 1n]]
   * ```
   */
  rsub(other: unknown): Deck { return this.counterOp("rsub", other); }
  /** `other & deck` (`__rand__`, the same as `__and__`).
   *
   * @example
   * ```ts
   * new Deck([1, 2]).rand([2, 2]).items(); //=> [[2, 1n]]
   * ```
   */
  rand(other: unknown): Deck { return this.counterOp("and", other); }
  /** `other | deck` (`__ror__`, the same as `__or__`).
   *
   * @example
   * ```ts
   * new Deck([1, 2]).ror([2, 2]).items(); //=> [[1, 1n], [2, 2n]]
   * ```
   */
  ror(other: unknown): Deck { return this.counterOp("or", other); }
  /** `deck * n`: every quantity multiplied by `n`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).mul(2).items(); //=> [[1, 2n], [2, 2n], [3, 2n]]
   * ```
   */
  mul(n: AgainExpression): AgainExpression;
  mul(n: Die): Die;
  mul(n: Int): Deck;
  mul(n: Int | Die | AgainExpression): Deck | Die | AgainExpression { return this.quantityOp("mul", "*", n); }
  /** `n * deck` (`__rmul__`, the same as `__mul__`).
   *
   * @example
   * ```ts
   * new Deck([1, 2]).rmul(3).items(); //=> [[1, 3n], [2, 3n]]
   * ```
   */
  rmul(n: AgainExpression): AgainExpression;
  rmul(n: Die): Die;
  rmul(n: Int): Deck;
  rmul(n: Int | Die | AgainExpression): Deck | Die | AgainExpression { return this.quantityOp("mul", "r*", n); }
  /** `deck // n`: every quantity divided by `n`, rounding down.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], [2, 4, 6]).floordiv(2).items(); //=> [[1, 1n], [2, 2n], [3, 3n]]
   * ```
   */
  floordiv(n: AgainExpression): AgainExpression;
  floordiv(n: Die): Die;
  floordiv(n: Int): Deck;
  floordiv(n: Int | Die | AgainExpression): Deck | Die | AgainExpression { return this.quantityOp("floordiv", "//", n); }
  /** `deck % n`: every quantity modulo `n` (cards left with zero are dropped).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], [2, 4, 5]).mod(2).items(); //=> [[3, 1n]]
   * ```
   */
  mod(n: AgainExpression): AgainExpression;
  mod(n: Die): Die;
  mod(n: Int): Deck;
  mod(n: Int | Die | AgainExpression): Deck | Die | AgainExpression { return this.quantityOp("mod", "%", n); }
  /** `Deck.symmetric_difference(other)`: the same as {@link Deck.xor}. (Python's docstring for it repeats the one
   * of `union`, but it is `^`.)
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).symmetricDifference([1, 1, 1, 1, 1, 2]).items(); //=> [[1, 1n], [2, 3n], [3, 4n]]
   * ```
   */
  symmetricDifference(other: unknown): Deck {
    return this.xor(other);
  }
  /** `Deck.append(outcome, quantity=1)`: this deck with `quantity` more of `outcome` (a negative quantity removes
   * some, but not below zero).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).append(4, 2).items(); //=> [[1, 1n], [2, 1n], [3, 1n], [4, 2n]]
   * ```
   */
  append(outcome: Outcome, quantity: Int = 1): Deck {
    const w = this.w;
    return Deck.wrap(wasm(() => w.append(outcome, quantity)));
  }
  /** `Deck.remove(outcome, quantity=None)`: this deck with `quantity` less of `outcome` (`null`: all of it; a
   * negative quantity adds).
   *
   * @example
   * ```ts
   * const cards = new Deck([1, 2, 3], 4);
   * cards.remove(1).items(); //=> [[2, 4n], [3, 4n]]
   * cards.remove(1, 3).items(); //=> [[1, 1n], [2, 4n], [3, 4n]]
   * ```
   */
  remove(outcome: Outcome, quantity: Int | null = null): Deck {
    const w = this.w;
    return Deck.wrap(wasm(() => w.remove(outcome, quantity)));
  }
  /** `Deck.split(which, star=None)`: `[selected, rest]`, a deck of the selected cards and a deck of the others.
   *
   * @param which - The outcomes to select: an array of them, or a function returning whether to select one.
   * @param options - Keyword arguments.
   * @param options.star - Whether to unpack each outcome into the function's arguments; inferred from the
   *   function's `length` when not given.
   *
   * @example
   * ```ts
   * const [high, low] = new Deck([1, 2, 3, 4, 5, 6]).split((x: number) => x > 4);
   * high.items(); //=> [[5, 1n], [6, 1n]]
   * low.outcomes(); //=> [1, 2, 3, 4]
   * ```
   */
  split(which: Which, { star = null }: { star?: boolean | null } = {}): [Deck, Deck] {
    const mask = this.population().selection(which, star);
    const w = this.w;
    const [selected, rest] = wasm(() => w.split(mask));
    return [Deck.wrap(selected), Deck.wrap(rest)];
  }
  /** `Deck.zero()`: every card times zero, which must come to a single outcome; the deck keeps its size.
   *
   * @throws `IcepoolError` (`ValueError`) if the cards do not all come to the same zero.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).zero().items(); //=> [[0, 12n]]
   * ```
   */
  zero(): Deck {
    const w = this.w;
    return Deck.wrap(wasm(() => w.zero()));
  }
  /** `Deck.to_one_hot(outcomes=None)`: each card as a one-hot Vector over `outcomes` (default: this deck's
   * outcomes), with `true` in the card's position.
   *
   * @example
   * ```ts
   * const hot = new Deck([1, 2, 2]).toOneHot();
   * hot.outcomes().map(String); //=> ["Vector((False, True))", "Vector((True, False))"]
   * hot.values(); //=> [2n, 1n]
   * ```
   */
  toOneHot(outcomes: readonly Outcome[] | null = null): Deck {
    // `Population.to_one_hot`: the die of the cards' one-hot Vectors, as a deck (`self._new_type(data)`).
    const items = this.population().toOneHot(outcomes).items();
    return new Deck(items.map(([o]) => o), items.map(([, q]) => q));
  }
  // Python's Deck defines these with `functools.reduce(..., initial=self)`, which raises TypeError before Python 3.14
  // (the oracle's Python); kept for parity.
  /** `Deck.additive_union(*args)`: meant as all the decks merged together, like {@link Deck.add}. Raises
   * `TypeError` as Python's does before 3.14 (it passes a keyword argument to `functools.reduce`); use `add`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).additiveUnion([3]); //=> throws TypeError
   * new Deck([1, 2]).add([3]).outcomes(); //=> [1, 2, 3]
   * ```
   */
  additiveUnion(...others: unknown[]): Deck { return reduceWithInitial(others); }
  /** `Deck.difference(*args)`: meant as this deck with the other cards removed, like {@link Deck.sub}. Raises
   * `TypeError` as Python's does before 3.14; use `sub`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).difference([1]); //=> throws TypeError
   * new Deck([1, 2]).sub([1]).outcomes(); //=> [2]
   * ```
   */
  difference(...others: unknown[]): Deck { return reduceWithInitial(others); }
  /** `Deck.intersection(*args)`: meant as the cards all decks have, like {@link Deck.and}. Raises `TypeError` as
   * Python's does before 3.14; use `and`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).intersection([1]); //=> throws TypeError
   * new Deck([1, 2]).and([1]).outcomes(); //=> [1]
   * ```
   */
  intersection(...others: unknown[]): Deck { return reduceWithInitial(others); }
  /** `Deck.union(*args)`: meant to keep as many of each card as the deck that has most, like {@link Deck.or}.
   * Raises `TypeError` as Python's does before 3.14; use `or`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).union([1, 1]); //=> throws TypeError
   * new Deck([1, 2]).or([1, 1]).items(); //=> [[1, 2n], [2, 1n]]
   * ```
   */
  union(...others: unknown[]): Deck { return reduceWithInitial(others); }
  /** `Deck.pad_to_denominator(denominator, outcome)`: adds cards of `outcome` until the deck has `denominator`
   * cards.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).padToDenominator(15, 0).items(); //=> [[0, 3n], [1, 4n], [2, 4n], [3, 4n]]
   * ```
   */
  padToDenominator(denominator: Int, outcome: Outcome): Deck {
    const w = this.w;
    return Deck.wrap(wasm(() => w.padToDenominator(denominator, outcome)));
  }
  /** `Deck.sequence(deals)`: every sequence of `deals` cards dealt one by one, as a die of arrays. This is
   * extremely expensive computationally: if you don't care about order, use {@link Deck.deal} instead.
   *
   * Python caches the sequences computed so far and builds `result` only in the loop over the new ones, so asking
   * for no more cards than already computed (`sequence(0)` included) raises its `UnboundLocalError`; this copies
   * it.
   *
   * @throws `IcepoolError` (`ValueError`) for a negative number of deals.
   *
   * @example
   * ```ts
   * new Deck(["a", "a", "b"]).sequence(2).items(); //=> [[["a", "a"], 2n], [["a", "b"], 2n], [["b", "a"], 2n]]
   * ```
   */
  sequence(deals: Int): Die {
    if (!isIntArg(deals)) throw unported("non-int deals");
    const n = Number(intOf(deals));
    if (n < 0) throw valueError("The number of cards dealt cannot be negative.");
    // `for i in range(len(self._sequence_cache), deals + 1)` is the only binding of `result`.
    if (n <= this.dealt) {
      throw new IcepoolError(
        "InvalidArgument",
        "UnboundLocalError",
        "cannot access local variable 'result' where it is not associated with a value",
      );
    }
    // Beyond this the sequences could not be stored anyway.
    if (n > 16) throw unported("a very long deck sequence");
    const w = this.w;
    const result = Die.wrap(wasm(() => w.sequence(n)));
    this.dealt = n;
    return result;
  }
  /** The `[outcome, quantity]` pairs in ascending order of outcome.
   *
   * @example
   * ```ts
   * new Deck(["K", "A", "A"]).items(); //=> [["A", 2n], ["K", 1n]]
   * ```
   */
  items(): [Outcome, bigint][] {
    return this.population().items();
  }
  /** The outcomes (distinct cards) in ascending order.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2], 4).outcomes(); //=> [1, 2, 3]
   * ```
   */
  outcomes(): Outcome[] {
    return this.population().outcomes();
  }
  /** The same as {@link Deck.outcomes} (Python's Mapping name); prefer `outcomes`.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2]).keys(); //=> [1, 2, 3]
   * ```
   */
  keys(): Outcome[] {
    return this.outcomes();
  }
  /** The quantities, in the order of the outcomes.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 2, 3, 3, 3]).values(); //=> [1n, 2n, 3n]
   * ```
   */
  values(): bigint[] {
    return this.items().map(([, q]) => q);
  }
  /** `deck[outcome]`: the quantity of `outcome`.
   *
   * @throws `IcepoolError` (`KeyError`) if the deck has no such outcome.
   *
   * @example
   * ```ts
   * const cards = new Deck([1, 2, 3], 4);
   * cards.get(2); //=> 4n
   * cards.get(9); //=> throws KeyError
   * ```
   */
  get(outcome: Outcome): bigint {
    return this.population().get(outcome);
  }
  /** The number of cards (`deck.size()`, the same as the denominator).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).size(); //=> 12n
   * ```
   */
  size(): bigint {
    return this.population().denominator();
  }
  /** The number of distinct outcomes. */
  get length(): number {
    return this.population().size;
  }
  /** Iterates over the outcomes (Python's `iter(deck)`), so `[...deck]` is `deck.outcomes()`. */
  [Symbol.iterator](): Iterator<Outcome> {
    return this.outcomes()[Symbol.iterator]();
  }
  /** Python's `deck == other`: whether `other` is a deck with the same cards (their `hash_key`s).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 2]).equals(new Deck(new Map([[2, 2], [1, 1]]))); //=> true
   * new Deck([1, 2]).equals(new Deck([1, 2, 2])); //=> false
   * ```
   */
  equals(other: unknown): boolean {
    if (other === this) return true;
    if (!(other instanceof Deck)) return false;
    const [a, b] = [this.w, other.w];
    return wasm(() => a.equals(b));
  }
  /** `deck == other`, the same as {@link Deck.equals} (a deck's `==` is plain equality, unlike a die's).
   *
   * @example
   * ```ts
   * new Deck([1, 2]).eq(new Deck([2, 1])); //=> true
   * ```
   */
  eq(other: unknown): boolean {
    return this.equals(other);
  }
  /** `deck != other`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).ne(new Deck([1])); //=> true
   * ```
   */
  ne(other: unknown): boolean {
    return !this.equals(other);
  }
  /** A statistic of a die with these quantities, by its Python name (`mean`, `variance`, ...).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).stat("max_outcome"); //=> 3
   * ```
   */
  stat(name: string): Outcome {
    const population = this.population() as unknown as Record<string, unknown>;
    const method = DECK_STATS[name];
    if (method === undefined) throw unported(`die statistic ${name}`);
    return (population[method] as () => unknown).call(population) as Outcome;
  }
  /** `Deck.mean()`: the mean of the outcomes, weighted by quantity (a Fraction for integer outcomes).
   *
   * @example
   * ```ts
   * String(new Deck([1, 2, 3, 3]).mean()); //=> "9/4"
   * ```
   */
  mean(): Outcome { return this.stat("mean"); }
  /** `Deck.variance()`: the variance of the outcomes, weighted by quantity (a Fraction for integer outcomes).
   *
   * @example
   * ```ts
   * String(new Deck([1, 2, 3], 4).variance()); //=> "2/3"
   * ```
   */
  variance(): Outcome { return this.stat("variance"); }
  /** `Deck.standard_deviation()`: the standard deviation, a float ({@link PyFloat}).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).sd(); //=> 0.816496580927726
   * ```
   */
  sd(): PyFloat { return this.stat("sd") as PyFloat; }
  /** The lowest outcome.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2]).minOutcome(); //=> 1
   * ```
   */
  minOutcome(): Outcome { return this.stat("min_outcome"); }
  /** The highest outcome.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2]).maxOutcome(); //=> 3
   * ```
   */
  maxOutcome(): Outcome { return this.stat("max_outcome"); }
  /** The total quantity: the number of cards, as {@link Deck.size}.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).denominator(); //=> 12n
   * ```
   */
  denominator(): bigint { return this.size(); }
  /** Python's `repr(deck)`.
   *
   * @example
   * ```ts
   * new Deck(["A", "A", "K"]).repr(); //=> "Deck({'A': 2, 'K': 1})"
   * ```
   */
  repr(): string {
    const w = this.w;
    return wasm(() => w.repr());
  }
  /** Python's `str(deck)`: a table of the cards.
   *
   * @example
   * ```ts
   * new Deck(["A", "A", "K"]).toString().split("\n")[0]; //=> "Deck with denominator 3"
   * ```
   */
  toString(): string {
    const w = this.w;
    return wasm(() => w.format(""));
  }

  /** `self <op> other` for a deck operator by its Python name: `Counter(other)` of a deck or die (its quantities),
   * of a Map (its values, ints), of an array or str (its elements counted), or of a multiset expression (its
   * elements, iterated). */
  private counterOp(op: "add" | "sub" | "rsub" | "and" | "or" | "xor", other: unknown): Deck {
    const w = this.w;
    if (other instanceof Deck || other instanceof Die) {
      const population = other instanceof Deck ? other.population() : other;
      return Deck.wrap(wasm(() => w.counterPopulation(op, population.w)));
    }
    // `Counter(None)` is empty.
    if (other === null || other === undefined) return Deck.wrap(wasm(() => w.counterElements(op, [])));
    if (Array.isArray(other) || typeof other === "string") {
      const elements = [...other];
      if (elements.some((x) => x instanceof List)) throw new IcepoolError("Unhashable", "TypeError", "unhashable type: 'list'");
      // `Counter` of dice would count them as cards (which `Deck(+data)` then flattens): not modelled.
      if (!elements.every(isPlainValue)) throw unported("counting dice or other objects");
      return Deck.wrap(wasm(() => w.counterElements(op, elements)));
    }
    if (other instanceof Map && [...other.values()].every(isExactInt) && [...other.keys()].every(isPlainValue)) {
      return Deck.wrap(wasm(() => w.counterMapping(op, [...other.keys()], [...other.values()])));
    }
    if (other instanceof MultisetExpression) return counterOfExpression(this, op, other);
    // Neither a mapping nor an iterable.
    if (isPlainValue(other) || other instanceof Special || other instanceof AgainExpression) {
      throw typeError(`'${other instanceof AgainExpression ? "AgainExpression" : pyTypeName(other)}' object is not iterable`);
    }
    throw unported("Counter() of this object");
  }

  /** `deck * n`, `n * deck` (`symbol` `r*`), `deck // n`, `deck % n`: for an int `n` the quantity methods, else
   * `NotImplemented`: a die or an Again expression then answers with its own operator, anything else refuses too
   * (`TypeError`). */
  private quantityOp(op: "mul" | "floordiv" | "mod", symbol: string, n: unknown): Deck | Die | AgainExpression {
    if (!isIntArg(n)) {
      // `n * deck` is the die's or expression's `__mul__`, `deck * n` its `__rmul__` (and so on).
      if (n instanceof Die || n instanceof AgainExpression) {
        return (symbol === "r*" ? ops[op](n, this) : ops[op](this, n)) as Die | AgainExpression;
      }
      // A multiset expression's `__rmul__` takes an int only; a deck's and a multiset tuple expression's are none.
      if (isPlainValue(n) || Array.isArray(n) || isMultisetExpression(n) || n instanceof Deck) {
        throw typeError("unsupported operand type(s) for Deck");
      }
      throw unported(`this deck operand of ${symbol}`);
    }
    const w = this.w;
    return Deck.wrap(wasm(() => w.quantities(op, intOf(n))));
  }
}

/** The statistics `stat(name)` reads off the cards' die, by their Python names. */
const DECK_STATS: Record<string, string> = {
  mean: "mean",
  variance: "variance",
  standard_deviation: "standardDeviation",
  sd: "sd",
  skewness: "skewness",
  excess_kurtosis: "excessKurtosis",
  modal_quantity: "modalQuantity",
  median: "median",
  median_low: "medianLow",
  median_high: "medianHigh",
  min_outcome: "minOutcome",
  max_outcome: "maxOutcome",
  denominator: "denominator",
  // `deck_stat` gives `tuple(mode())`.
  mode: "mode",
  is_empty: "isEmpty",
};

/** `functools.reduce(op, args, initial=self)`: `reduce` takes no keyword arguments before Python 3.14, so these
 * always raise there, even without arguments (DIVERGENCES.md). */
function reduceWithInitial(_others: unknown[]): never {
  throw typeError("reduce() takes no keyword arguments");
}

/** `deck <op> expr` for a multiset expression `expr`. `Counter(expr)` iterates it through `__getitem__` (`expr[0]`,
 * `expr[1]`, ...: `keep(i)`, evaluated into a die) until an index raises `IndexError`, so its keys are dice. No die
 * is among the deck's cards, so each operator treats one as a card the deck lacks: `+`, `|` and `^` keep its
 * count, `-` and `&` leave it at zero or below, which `+data` drops. `Deck(+data)` then flattens the dice that are
 * left into their outcomes, after the deck's own cards. An expression Python would iterate forever is not modelled.
 */
function counterOfExpression(deck: Deck, op: string, expr: MultisetExpression): Deck {
  // Inside a multiset function `expr[i]` is a raw result, never an `IndexError`: Python never stops.
  if (hasParameter(expr)) throw unported("iterating an expression whose items are not dice");
  // A keep generator (or a mixture of them) checks its keep tuple first: an index past it is an `IndexError` even
  // if the dice are empty. Anything else gets the index only in the evaluation.
  const bounded = keepTupleBounded(expr);
  const dice: Die[] = [];
  for (let i = 0; ; i++) {
    let die: unknown;
    try {
      die = expr.keep(i);
    } catch (e) {
      if (e instanceof IcepoolError && e.pythonType === "IndexError") break;
      throw e;
    }
    // Evaluated with the index, an expression without outcomes at all is the empty die at every index, never an
    // `IndexError`: Python never stops.
    if (!(die instanceof Die)) throw unported("iterating an expression whose items are not dice");
    if (!bounded && die.size === 0) throw unported("iterating an expression without outcomes (Python never stops)");
    dice.push(die);
  }
  const keepsDice = op === "add" || op === "rsub" || op === "or" || op === "xor";
  const w = deck.w;
  // The deck's own cards first (`counter_items(op, Counter())`), then the dice.
  const own = Deck.wrap(wasm(() => w.counterElements(op, [])));
  return keepsDice ? new Deck([own, ...dice]) : own;
}

/** `Deck(outcomes, times)` after the Again check (`Deck.__new__`): `itemize`, then a single deck with time 1 is
 * that deck itself, and anything else goes through `expand_args_for_deck`. */
function makeDeck(outcomes: DeckOutcomes, times: unknown): WasmDeck | Deck {
  // A deck is a mapping of its outcomes to their quantities.
  const items = itemize((outcomes instanceof Deck ? new Map(outcomes.items()) : outcomes) as Map<unknown, Int>, times);
  if (items.outcomes.length === 1 && items.outcomes[0] instanceof Deck && isOne(items.times[0])) return items.outcomes[0];
  // Pools and multiset expressions as cards (or in a tuple card) are not modelled.
  const holdsExpression = (x: unknown): boolean =>
    x instanceof MultisetExpression || x instanceof MultisetTupleExpression || (Array.isArray(x) && x.some(holdsExpression));
  if (items.outcomes.some(holdsExpression)) throw unported("a pool or a multiset expression as an outcome");
  // `expand_args_for_deck`: each argument expanded (`expand_arg`), then merged by the core (`merge_duplicates`).
  return wasm((w) => {
    const builder = new w.DeckBuilder();
    try {
      items.outcomes.forEach((arg, i) => {
        const time = items.times[i];
        // A tuple is the Cartesian product of its elements: a die if it holds dice, `Reroll` if it holds one.
        const expanded = Array.isArray(arg) && !(arg instanceof List) ? tupleize(...(arg as Outcome[])) : arg;
        if (expanded instanceof Deck) builder.deck(expanded.w, time);
        else if (expanded instanceof Die) builder.die(expanded.w, time);
        else if (expanded === Reroll || expanded === Restart) builder.reroll(time);
        else builder.outcome(expanded, time);
      });
      return builder.build();
    } finally {
      builder.free();
    }
  });
}

/** `icepool.Deck(outcomes, times)` without `new`.
 *
 * @example
 * ```ts
 * deck(["A", "K", "Q"], 4).size(); //=> 12n
 * ```
 */
export function deck(outcomes: DeckOutcomes = [], times: Int | readonly Int[] = 1): Deck {
  return new Deck(outcomes, times);
}

/** `deck.deal(handSize)`: an unordered deal of a single hand from a {@link Deck}, a multiset expression over the
 * hand (`.sum()`, `.highest(...)`, evaluators, ...).
 *
 * It is permissible to deal zero cards from an empty deck, but not all evaluators will handle this case,
 * especially if they depend on the outcome type. Dealing zero cards from a non-empty deck does not have this issue.
 *
 * @example
 * ```ts
 * // Two cards from a deck of four 1s, four 2s and four 3s.
 * const hand = new Deck([1, 2, 3], 4).deal(2);
 * hand.denominator(); //=> 66n
 * hand.sum().probability(6).toString(); //=> "1/11"
 * hand.repr(); //=> "Deal(Deck({1: 4, 2: 4, 3: 4}), hand_size=2)"
 * ```
 */
export class Deal extends KeepGenerator {
  /** `(deck items, keep_tuple, hand_size, keep_size, denominator, outcomes)`: everything the other accessors read,
   * in one call.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(2).info(); //=> [[[1, 4], [2, 4], [3, 4]], [1, 1], 2, 2, 66, [1, 2, 3]]
   * ```
   */
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]] {
    const g = this.g;
    return wasm(() => {
      const deck = Deck.wrap(g.deck());
      const items = deck.items().map(([o, q]): [Outcome, Int] => [o, pyInt(q)]);
      return [items, g.keepTuple() as Int[], g.handSize(), pyInt(g.keepSize() as Int), pyInt(g.denominator() as Int), deck.outcomes()];
    });
  }
  /** `Deal.deck()`: the deck the cards are dealt from.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(2).deck().size(); //=> 12n
   * ```
   */
  deck(): Deck {
    const g = this.g;
    return Deck.wrap(wasm(() => g.deck()));
  }
  /** `Deal.hand_size()`: the number of cards dealt.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(2).handSize(); //=> 2
   * ```
   */
  handSize(): Int { return this.info()[2]; }
  /** `Deal.keep_size()`: the number of cards kept (after `highest`, `lowest`, ...).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(3).highest(2).keepSize(); //=> 2
   * ```
   */
  keepSize(): Int { return this.info()[3]; }
  /** `Deal.keep_tuple()`: how many times each sorted (ascending) card is counted.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(3).highest(2).keepTuple(); //=> [0, 1, 1]
   * ```
   */
  keepTuple(): Int[] { return this.info()[1]; }
  /** `Deal.denominator()`: the number of possible hands, `comb(deck size, hand size)`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal(2).denominator(); //=> 66n
   * ```
   */
  denominator(): bigint { return BigInt(this.info()[4]); }
  /** `Deal.outcomes()`: the outcomes of the deck, in ascending order.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2], 4).deal(2).outcomes(); //=> [1, 2, 3]
   * ```
   */
  outcomes(): Outcome[] { return this.info()[5]; }
  /** Python's `str(deal)`: the hand size and the deck's table.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).deal(2).toString().split("\n")[0]; //=> "Deal of hand_size=2 from deck:"
   * ```
   */
  override toString(): string { return this.strs()[1]; }
  /** Python's `repr(deal)`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).deal(2).repr(); //=> "Deal(Deck({1: 1, 2: 1, 3: 1}), hand_size=2)"
   * ```
   */
  repr(): string { return this.strs()[0]; }
  /** `[repr, str]`. */
  private strs(): string[] {
    const g = this.g;
    return wasm(() => g.dealStrs());
  }
  /** The core's deal (a deal is always a keep generator of the core). */
  private get g(): WasmGenerator {
    return generatorOf(this) as WasmGenerator;
  }
}

/** A multiset tuple expression: several hands (a `MultiDeal`, a tuple parameter of a multiset function, a slice
 * of either). `hand(i)` is one hand (Python's `x[i]`), `slice(...)` some of them.
 *
 * @example
 * ```ts
 * const hands: MultisetTupleExpression = new Deck([1, 2, 3], 4).deal([1, 2]);
 * hands.hand(1).sum().mean().toString(); //=> "4"
 * ```
 */
export class MultisetTupleExpression implements Iterable<MultisetExpression>, Disposable {
  /** The tree node; `null` once freed. */
  private tupleNode!: TupleNode | null;

  /** A tuple expression of this class. @internal */
  static native<T extends MultisetTupleExpression>(this: abstract new (...args: never[]) => T, node: TupleNode): T {
    const x = Object.create(this.prototype as object) as T;
    x.tupleNode = node;
    return x;
  }

  /** The tree node, which must not have been freed. @internal */
  get node(): TupleNode {
    if (this.tupleNode === null) throw new TypeError("icepool: this expression was freed");
    return this.tupleNode;
  }

  /** Lets go of the expression now (its deck's WASM memory is freed once nothing else holds it). Using it
   * afterwards throws a `TypeError`; freeing it again does nothing.
   *
   * @example
   * ```ts
   * const hands = new Deck([1, 2, 3], 4).deal([1, 2]);
   * hands.free();
   * hands.length; //=> throws TypeError
   * ```
   */
  free(): void {
    this.tupleNode = null;
  }

  /** `free()`, for `using hands = deck.deal([2, 2]);`. */
  [Symbol.dispose](): void {
    this.free();
  }

  /** `len(x)`: the number of hands.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2, 3]).slice(1).length; //=> 2
   * ```
   */
  get length(): number {
    return tupleLen(this);
  }
  /** Iterates over the hands (Python's `iter(x)`), so `const [a, b] = hands` unpacks two of them.
   *
   * @example
   * ```ts
   * const [a, b] = new Deck([1, 2, 3, 4]).deal([1, 3]);
   * b.size().items(); //=> [[3, 4n]]
   * ```
   */
  *[Symbol.iterator](): Iterator<MultisetExpression> {
    for (let i = 0; i < this.length; i++) yield this.hand(i);
  }
  /** `x[index]`: one hand, as a multiset expression.
   *
   * @example
   * ```ts
   * const hands = new Deck([1, 2, 3], 4).deal([1, 2]);
   * hands.hand(0).sum().mean().toString(); //=> "2"
   * ```
   */
  hand(index: Int): MultisetExpression {
    // An int index makes a `MultisetTupleIndex` (bounds are only checked when evaluated); anything else falls off
    // the end of `__getitem__`, which returns None.
    if (!isIntArg(index)) return null as unknown as MultisetExpression;
    return tupleIndex(this, saturated(intOf(index)));
  }
  /** `x[start:stop:step]`: some of the hands, as a multiset tuple expression (`null` as Python's omitted bound).
   *
   * @example
   * ```ts
   * const hands = new Deck([1, 2, 3], 4).deal([1, 2, 3]);
   * hands.slice(1).hand(0).size().outcomes(); //=> [2]
   * ```
   */
  slice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): MultisetTupleExpression {
    // The bounds are only looked at when it is evaluated or its `len()` taken.
    const bounds = [start ?? null, stop ?? null, step ?? null] as const;
    if (!bounds.every(isPlainValue)) throw unported("a slice bound that is not a value");
    return MultisetTupleExpression.native({ kind: "tupleSlice", child: this, bounds });
  }
}

/** An index beyond 64 bits is out of range anyway (an `IndexError` when evaluated): saturating keeps that. */
function saturated(i: Int): Int {
  const big = BigInt(i);
  const limit = 2n ** 63n;
  return big >= limit ? limit - 1n : big < -limit ? -limit : i;
}

/** The tuple parameter for argument `argIndex` of the function being traced, a multiset tuple expression of `len`
 * multisets, named `name` (`_make_param`). @internal */
export function tupleParameter(argIndex: number, len: number, name: string): MultisetTupleExpression {
  return MultisetTupleExpression.native({ kind: "tupleParameter", argIndex, len, name });
}

/** `deck.deal([size, ...])`: a deal of several hands from a {@link Deck}; `hand(i)` or `slice(...)` picks some.
 *
 * The cards within each hand are in sorted order. Hands may be organized into groups in which the hands are
 * initially indistinguishable ({@link Deck.dealGroups}).
 *
 * @throws `IcepoolError` (`ValueError`) if more cards are dealt than the deck has.
 *
 * @example
 * ```ts
 * const hands = new Deck([1, 2, 3], 4).deal([1, 2]);
 * hands.totalCardsDealt(); //=> 3
 * hands.hand(1).sum().mean().toString(); //=> "4"
 * new Deck([1, 2]).deal([1, 2]); //=> throws ValueError
 * ```
 */
export class MultiDeal extends MultisetTupleExpression {
  /** The core's multi-deal. @internal */
  get md(): WasmMultiDeal {
    return (this.node as { md: WasmMultiDeal }).md;
  }
  /** `(deck items, hand_sizes, total_cards_dealt, len, denominator, outcomes)`: everything the other accessors
   * read, in one call.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).info(); //=> [[[1, 4], [2, 4], [3, 4]], [1, 2], 3, 2, 660, [1, 2, 3]]
   * ```
   */
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]] {
    const md = this.md;
    return wasm(() => {
      const deck = Deck.wrap(md.deck());
      const items = deck.items().map(([o, q]): [Outcome, Int] => [o, pyInt(q)]);
      return [items, Array.from(md.handSizes()), md.totalCardsDealt(), md.length, pyInt(md.denominator() as Int), deck.outcomes()];
    });
  }
  /** `MultiDeal.deck()`: the deck the cards are dealt from.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).deck().size(); //=> 12n
   * ```
   */
  deck(): Deck {
    const md = this.md;
    return Deck.wrap(wasm(() => md.deck()));
  }
  /** `MultiDeal.hand_sizes()`: the number of cards dealt to each hand.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).handSizes(); //=> [1, 2]
   * ```
   */
  handSizes(): Int[] { return this.info()[1]; }
  /** `MultiDeal.total_cards_dealt()`: the total number of cards dealt.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).totalCardsDealt(); //=> 3
   * ```
   */
  totalCardsDealt(): Int { return this.info()[2]; }
  /** `MultiDeal.denominator()`: the number of ways to deal the hands.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).denominator(); //=> 660n
   * ```
   */
  denominator(): bigint { return BigInt(this.info()[4]); }
  /** `MultiDeal.outcomes()`: the outcomes of the deck, in ascending order.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2], 4).deal([1, 2]).outcomes(); //=> [1, 2, 3]
   * ```
   */
  outcomes(): Outcome[] { return this.info()[5]; }
  /** Python's `str(multi_deal)`: the hand groups and the deck's table.
   *
   * @example
   * ```ts
   * const firstLine = new Deck([1, 2, 3]).deal([1, 2]).toString().split("\n")[0];
   * firstLine; //=> "MultiDeal of hand_groups=((1, 1), (2, 1)) from deck:"
   * ```
   */
  override toString(): string { return this.strs()[1]; }
  /** Python's `repr(multi_deal)`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).deal([1, 2]).repr(); //=> "MultiDeal(Deck({1: 1, 2: 1, 3: 1}), hand_groups=((1, 1), (2, 1)))"
   * ```
   */
  repr(): string { return this.strs()[0]; }
  /** `[repr, str]`. */
  private strs(): string[] {
    const md = this.md;
    return wasm(() => md.strs());
  }
}

registerDeckClasses({ Deal, Deck, MultisetTupleExpression });

/** What `new Wallenius(...)` takes: `[outcome, weight]` pairs, or a Map of outcome -> weight or
 * `[weight, quantity]`.
 *
 * @example
 * ```ts
 * import { Wallenius, type Int, type Outcome, type WalleniusData } from "icepool-wasm";
 * const pairs: WalleniusData = [["a", 1], ["b", 2]];
 * const weights: WalleniusData = new Map<Outcome, Int | [Int, Int]>([["a", 1], ["b", [2, 2]]]); // two "b"s of weight 2
 * new Wallenius(pairs).deal(1).expand().items(); //=> [[["a"], 1n], [["b"], 2n]]
 * ```
 */
export type WalleniusData = readonly (readonly [Outcome, Int])[] | Map<Outcome, Int | readonly [Int, Int]>;

/** How deep `_wallenius_weights` may recurse in the port: Python's recursion limit stops it after about this many
 * levels (each level is several frames of `map`), at a point the port does not model. */
const MAX_WALLENIUS_DEPTH = 40;

/** `icepool.Wallenius(data)` (EXPERIMENTAL in icepool): Wallenius' noncentral hypergeometric distribution, sampling
 * without replacement with weights, where the whole weight of a card goes away when it is pulled. A Map is a Python
 * dict (keys equal by Python's `==`).
 *
 * @example
 * ```ts
 * import { Wallenius, type Int, type Outcome } from "icepool-wasm";
 * // One "a" of weight 1 and two "b"s of weight 2 each: draw two.
 * const urn = new Wallenius(new Map<Outcome, Int | [Int, Int]>([["a", 1], ["b", [2, 2]]]));
 * urn.deal(2).expand().items(); //=> [[["a", "b"], 28n], [["b", "b"], 32n]]
 * ```
 */
export class Wallenius {
  /** `_weight_decks`: one deck per weight, a dict keyed by weight (in insertion order). */
  private readonly weightDecks: [unknown, Deck][] = [];
  /** `_weight_die`: each weight, weighted by its total weight in the decks. */
  private readonly weightDie: Die;

  /** `Wallenius(data)`: `data` is either `[outcome, weight]` pairs (one card each) or a Map from outcomes to a
   * weight or `[weight, quantity]`. */
  constructor(data: WalleniusData) {
    for (const [outcome, weight, quantity] of walleniusCards(data)) {
      // A weight is a dict key: a list is unhashable.
      if (weight instanceof List) throw new IcepoolError("Unhashable", "TypeError", "unhashable type: 'list'");
      // Python would flatten a die among the outcomes into its cards, and multiply a die weight: not ported.
      if (!isPlainValue(outcome) || !isPlainValue(weight)) {
        throw new IcepoolError("Unported", "NotImplementedError", "icepool: a Wallenius outcome or weight that is not a plain value");
      }
      // `self._weight_decks.get(weight, icepool.Deck()).append(outcome, quantity)`.
      const keys = this.weightDecks.map(([w]) => w);
      const first = wasm((w) => w.dictKeys([...keys, weight]));
      const i = first[keys.length];
      const deck = (i < keys.length ? this.weightDecks[i][1] : new Deck()).append(outcome as Outcome, quantity as Int);
      if (i < keys.length) this.weightDecks[i][1] = deck;
      else this.weightDecks.push([weight, deck]);
    }
    // `Die({weight: weight * deck.denominator() for weight, deck in self._weight_decks.items()})`.
    const quantities = this.weightDecks.map(([w, deck]) => [w, ops.mul(w, deck.denominator())] as const);
    this.weightDie = new Die(new Map(quantities as [Outcome, Int][]));
  }
  /** `Wallenius.deal(hand_size)`: the multiset of `handSize` outcomes dealt, as a multiset expression (`.sum()`,
   * `.expand()`, ...).
   *
   * @example
   * ```ts
   * new Wallenius([["a", 1], ["b", 2]]).deal(1).expand().items(); //=> [[["a"], 1n], [["b"], 2n]]
   * ```
   */
  deal(handSize: Int): MultisetExpression {
    if (!isIntArg(handSize)) throw new IcepoolError("Unported", "NotImplementedError", "icepool: a non-int Wallenius hand size");
    // `if hand_size == 0: return icepool.Pool([])`.
    if (BigInt(intOf(handSize)) === 0n) return new Pool([]);
    // The recursion pulls one card per level until the hand is full or the cards run out (a negative hand never
    // fills).
    const cards = this.weightDecks.reduce((n, [, deck]) => n + deck.size(), 0n);
    const hand = BigInt(intOf(handSize));
    const depth = hand < 0n || hand > cards ? cards : hand;
    if (depth > BigInt(MAX_WALLENIUS_DEPTH)) throw new IcepoolError("Unported", "NotImplementedError", "icepool: a Wallenius hand this deep");
    const handWeights = walleniusWeights(this.weightDie, hand, new Map());
    // `hand_weights.map_to_pool(inner, star=False)`: a mixture of the deals of each sorted tuple of weights to pull
    // (`Counter(weights)`: how many cards of each weight), weighted by its chance.
    const data: [MultisetExpression, bigint][] = [];
    for (const [weights, quantity] of handWeights.items()) {
      let result: MultisetExpression | undefined;
      for (const [weight, count] of counter(weights as unknown[])) {
        const deal = this.deckOf(weight).deal(count);
        result = result === undefined ? deal : result.add(deal);
      }
      data.push([result as MultisetExpression, quantity]);
    }
    return mixtureOf(data);
  }
  /** `self._weight_decks[weight]`. */
  private deckOf(weight: unknown): Deck {
    const keys = this.weightDecks.map(([w]) => w);
    const i = wasm((w) => w.dictLookup(keys, [weight]))[0];
    if (i < 0) throw new IcepoolError("MissingKey", "KeyError", String(weight));
    return this.weightDecks[i][1];
  }
}

/** The cards `Wallenius(data)` puts in: `[outcome, weight, quantity]` for each `(outcome, weight)` pair, or for
 * each item of a mapping (`isinstance(v, int)`: a weight; anything else unpacks as `weight, quantity`). */
function walleniusCards(data: WalleniusData): [unknown, unknown, unknown][] {
  const pair = (x: unknown): [unknown, unknown] => {
    const items = Array.isArray(x) ? [...x] : typeof x === "string" ? [...x] : undefined;
    if (items === undefined) throw typeError(`cannot unpack non-iterable ${pyTypeName(x)} object`);
    if (items.length !== 2) {
      throw valueError(items.length < 2 ? `not enough values to unpack (expected 2, got ${items.length})` : "too many values to unpack (expected 2)");
    }
    return items as [unknown, unknown];
  };
  if (data instanceof Map) {
    // A Python dict: equal keys are one, spelt as the first and valued as the last.
    const [outcomes, values] = dictItems(data);
    return outcomes.map((outcome, i) => {
      const v = values[i];
      if (isIntArg(v)) return [outcome, v, 1];
      const [weight, quantity] = pair(v);
      return [outcome, weight, quantity];
    });
  }
  return Array.from(data, (item) => {
    const [outcome, weight] = pair(item);
    return [outcome, weight, 1];
  });
}

/** `Counter(weights).items()`: each weight with how many times it is among them, in first-seen order. */
function counter(weights: unknown[]): [unknown, number][] {
  const first = wasm((w) => w.dictKeys(weights));
  const counts = new Map<number, number>();
  first.forEach((f) => counts.set(f, (counts.get(f) ?? 0) + 1));
  return [...counts].map(([i, n]) => [weights[i], n]);
}

/** `_wallenius_weights(weight_die, hand_size)`: a die whose outcomes are sorted tuples of weights to pull. Each
 * weight is pulled with its chance in `weight_die`, which then loses that card's weight (`remove(weight, weight)`).
 * Python caches it by its arguments (`@cache`); `cache` does the same for one deal, by the die's `repr`. */
function walleniusWeights(weightDie: Die, handSize: bigint, cache: Map<string, Die>): Die {
  if (handSize === 0n) return new Die([[]]);
  const key = `${handSize} ${weightDie.repr()}`;
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  if (cache.size > 1 << 16 || weightDie.denominator() > BigInt(1 << 16)) {
    throw new IcepoolError("Unported", "NotImplementedError", "icepool: a large Wallenius");
  }
  // `(_wallenius_weights(weight_die.remove(weight, weight), hand_size - 1) + (weight, )).map(lambda x:
  // tuple(sorted(x)))`.
  const inner = (weight: Outcome): Die =>
    (walleniusWeights(weightDie.remove(weight, weight as Int), handSize - 1n, cache).add([weight]) as Die)
      .map((x: Outcome[]) => wasm((w) => w.pySorted(x)) as Outcome[]);
  const result = weightDie.map(inner);
  cache.set(key, result);
  return result;
}
