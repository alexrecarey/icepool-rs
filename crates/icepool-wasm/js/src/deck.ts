// Decks and deals (population/deck.py, generator/deal.py, generator/multi_deal.py, wallenius.py).
// INTERIM (task C6): held through the bridge (rpc.ts) until they move onto typed exports.

import { list, type Callback, type Die, type MapResult } from "./die.js";
import { KeepGenerator, type MultisetExpression } from "./multiset.js";
import { HANDLE, Handle, call, registerKind, wrapHandle } from "./rpc.js";
import { List, type Int, type Outcome } from "./values.js";

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
export class Deck extends Handle implements Iterable<Outcome> {
  /** `Deck(outcomes, times=1)`: `times` multiplies each element of `outcomes`, one number for all of them or an
   * array as long as `outcomes`. */
  constructor(outcomes: DeckOutcomes, times: Int | readonly Int[] = 1) {
    super(HANDLE, -1);
    const made =
      outcomes instanceof Map
        ? call("deck", { outcomes: List.from(outcomes.keys()), times: List.from(outcomes.values()) })
        : call("deck", { outcomes: list(outcomes), times: list(times) });
    this.adopt(made, Deck);
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
    return Array.isArray(handSize)
      ? (call("multi_deal", { deck: this, hand_sizes: list(handSize) }) as MultiDeal)
      : (call("deal", { deck: this, hand_size: handSize }) as Deal);
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
    return call("deal_groups", { deck: this, hand_groups: List.from(groups.map(list)) }) as MultiDeal;
  }
  /** `Deck.map(repl, star=None)`: maps each card of this deck to another outcome, keeping its quantity.
   *
   * @param f - A function returning the new outcome for each old outcome. The new outcomes may be decks, in which
   *   case one card is replaced with several (not recommended).
   * @param options.star - Whether outcomes are unpacked into separate arguments of `f`; `null` guesses from `f`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).map((x) => 10 * x).items(); //=> [[10, 4n], [20, 4n], [30, 4n]]
   * ```
   */
  map(f: Callback<MapResult>, { star = null }: { star?: boolean | null } = {}): Deck {
    return call("deck_map", { deck: this, repl: f, star }) as Deck;
  }
  /** `deck + other`: both decks merged together (the quantities add). `other` is an array of outcomes or a deck.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).add([1, 4]).items(); //=> [[1, 5n], [2, 4n], [3, 4n], [4, 1n]]
   * ```
   */
  add(other: unknown): Deck { return deckOp(this, "+", other); }
  /** `deck - other`: this deck with the other cards removed (but not below zero of each card).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).sub([1, 1, 1]).items(); //=> [[1, 1n], [2, 4n], [3, 4n]]
   * ```
   */
  sub(other: unknown): Deck { return deckOp(this, "-", other); }
  /** `deck & other`: the cards that both decks have.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).and([1, 1, 5]).items(); //=> [[1, 2n]]
   * ```
   */
  and(other: unknown): Deck { return deckOp(this, "&", other); }
  /** `deck | other`: as many of each card as the deck that has more of them.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).or([1, 1, 1, 1, 1]).items(); //=> [[1, 5n], [2, 4n], [3, 4n]]
   * ```
   */
  or(other: unknown): Deck { return deckOp(this, "|", other); }
  /** `deck ^ other`: of each card, the difference between the two decks' quantities.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).xor([1, 1, 1, 1, 1, 2]).items(); //=> [[1, 1n], [2, 3n], [3, 4n]]
   * ```
   */
  xor(other: unknown): Deck { return deckOp(this, "^", other); }
  /** `deck * n`: every quantity multiplied by `n`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).mul(2).items(); //=> [[1, 2n], [2, 2n], [3, 2n]]
   * ```
   */
  mul(n: Int): Deck { return call("deck_mul", { deck: this, operator: "*", n }) as Deck; }
  /** `deck // n`: every quantity divided by `n`, rounding down.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], [2, 4, 6]).floordiv(2).items(); //=> [[1, 1n], [2, 2n], [3, 3n]]
   * ```
   */
  floordiv(n: Int): Deck { return call("deck_mul", { deck: this, operator: "//", n }) as Deck; }
  /** `deck % n`: every quantity modulo `n` (cards left with zero are dropped).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], [2, 4, 5]).mod(2).items(); //=> [[3, 1n]]
   * ```
   */
  mod(n: Int): Deck { return call("deck_mul", { deck: this, operator: "%", n }) as Deck; }
  /** `Deck.symmetric_difference(other)`: the same as {@link Deck.xor}. (Python's docstring for it repeats the one
   * of `union`, but it is `^`.)
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).symmetricDifference([1, 1, 1, 1, 1, 2]).items(); //=> [[1, 1n], [2, 3n], [3, 4n]]
   * ```
   */
  symmetricDifference(other: unknown): Deck {
    return call("deck_method", { deck: this, name: "symmetric_difference", arg: list(other) }) as Deck;
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
    return call("pop_append", { population: this, outcome, quantity }) as Deck;
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
    return call("pop_remove", { population: this, outcome, quantity }) as Deck;
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
    return call("to_one_hot", { population: this, outcomes: list(outcomes) }) as Deck;
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
  additiveUnion(...others: unknown[]): Deck { return deckMethod(this, "additive_union", others); }
  /** `Deck.difference(*args)`: meant as this deck with the other cards removed, like {@link Deck.sub}. Raises
   * `TypeError` as Python's does before 3.14; use `sub`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).difference([1]); //=> throws TypeError
   * new Deck([1, 2]).sub([1]).outcomes(); //=> [2]
   * ```
   */
  difference(...others: unknown[]): Deck { return deckMethod(this, "difference", others); }
  /** `Deck.intersection(*args)`: meant as the cards all decks have, like {@link Deck.and}. Raises `TypeError` as
   * Python's does before 3.14; use `and`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).intersection([1]); //=> throws TypeError
   * new Deck([1, 2]).and([1]).outcomes(); //=> [1]
   * ```
   */
  intersection(...others: unknown[]): Deck { return deckMethod(this, "intersection", others); }
  /** `Deck.union(*args)`: meant to keep as many of each card as the deck that has most, like {@link Deck.or}.
   * Raises `TypeError` as Python's does before 3.14; use `or`.
   *
   * @example
   * ```ts
   * new Deck([1, 2]).union([1, 1]); //=> throws TypeError
   * new Deck([1, 2]).or([1, 1]).items(); //=> [[1, 2n], [2, 1n]]
   * ```
   */
  union(...others: unknown[]): Deck { return deckMethod(this, "union", others); }
  /** `Deck.pad_to_denominator(denominator, outcome)`: adds cards of `outcome` until the deck has `denominator`
   * cards.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).padToDenominator(15, 0).items(); //=> [[0, 3n], [1, 4n], [2, 4n], [3, 4n]]
   * ```
   */
  padToDenominator(denominator: Int, outcome: Outcome): Deck {
    return call("deck_pad_to_denominator", { deck: this, denominator, outcome }) as Deck;
  }
  /** `Deck.sequence(deals)`: every sequence of `deals` cards dealt one by one, as a die of arrays. This is
   * extremely expensive computationally: if you don't care about order, use {@link Deck.deal} instead.
   *
   * @throws `IcepoolError` (`ValueError`) for a negative number of deals.
   *
   * @example
   * ```ts
   * new Deck(["a", "a", "b"]).sequence(2).items(); //=> [[["a", "a"], 2n], [["a", "b"], 2n], [["b", "a"], 2n]]
   * ```
   */
  sequence(deals: Int): Die {
    return call("deck_sequence", { deck: this, deals }) as Die;
  }
  /** The `[outcome, quantity]` pairs in ascending order of outcome.
   *
   * @example
   * ```ts
   * new Deck(["K", "A", "A"]).items(); //=> [["A", 2n], ["K", 1n]]
   * ```
   */
  items(): [Outcome, bigint][] {
    const info = call("deck_info", { deck: this }) as [[Outcome, Int][], Int, number];
    return info[0].map(([o, q]) => [o, BigInt(q)]);
  }
  /** The outcomes (distinct cards) in ascending order.
   *
   * @example
   * ```ts
   * new Deck([3, 1, 2], 4).outcomes(); //=> [1, 2, 3]
   * ```
   */
  outcomes(): Outcome[] {
    return call("deck_method", { deck: this, name: "keys", arg: null }) as Outcome[];
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
    return (call("deck_method", { deck: this, name: "values", arg: null }) as Int[]).map(BigInt);
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
    return BigInt(call("deck_method", { deck: this, name: "getitem", arg: outcome }) as Int);
  }
  /** The number of cards (`deck.size()`, the same as the denominator).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).size(); //=> 12n
   * ```
   */
  size(): bigint {
    return BigInt((call("deck_info", { deck: this }) as [unknown, Int, number])[1]);
  }
  /** The number of distinct outcomes. */
  get length(): number {
    return (call("deck_info", { deck: this }) as [unknown, Int, number])[2];
  }
  /** Iterates over the outcomes (Python's `iter(deck)`), so `[...deck]` is `deck.outcomes()`. */
  [Symbol.iterator](): Iterator<Outcome> {
    return this.outcomes()[Symbol.iterator]();
  }
  /** A statistic of a die with these quantities, by its Python name (`mean`, `variance`, ...).
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).stat("max_outcome"); //=> 3
   * ```
   */
  stat(name: string): Outcome {
    return call("deck_stat", { deck: this, stat: name }) as Outcome;
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
  /** `Deck.standard_deviation()`: the standard deviation, as a number.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).sd(); //=> 0.816496580927726
   * ```
   */
  sd(): number { return this.stat("sd") as number; }
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
  denominator(): bigint { return BigInt(this.stat("denominator") as Int); }
}

function deckMethod(deck: Deck, name: string, others: unknown[]): Deck {
  return call("deck_method", { deck, name, arg: List.from(others.map(list)) }) as Deck;
}

function deckOp(deck: Deck, operator: string, other: unknown): Deck {
  return call("deck_op", { operator, a: deck, b: list(other) }) as Deck;
}

/** `icepool.Deck(outcomes, times)` without `new`.
 *
 * @example
 * ```ts
 * deck(["A", "K", "Q"], 4).size(); //=> 12n
 * ```
 */
export function deck(outcomes: DeckOutcomes, times: Int | readonly Int[] = 1): Deck {
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
    return call("deal_info", { deal: this }) as [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
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
  override toString(): string { return (call("deal_str", { deal: this }) as [string, string])[1]; }
  /** Python's `repr(deal)`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).deal(2).repr(); //=> "Deal(Deck({1: 1, 2: 1, 3: 1}), hand_size=2)"
   * ```
   */
  repr(): string { return (call("deal_str", { deal: this }) as [string, string])[0]; }
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
export class MultisetTupleExpression extends Handle {
  /** `x[index]`: one hand, as a multiset expression.
   *
   * @example
   * ```ts
   * const hands = new Deck([1, 2, 3], 4).deal([1, 2]);
   * hands.hand(0).sum().mean().toString(); //=> "2"
   * ```
   */
  hand(index: Int): MultisetExpression {
    return call("mx_tuple_index", { expr: this, index }) as MultisetExpression;
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
    return call("mx_tuple_slice", { expr: this, start, stop, step }) as MultisetTupleExpression;
  }
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
  /** `(deck items, hand_sizes, total_cards_dealt, len, denominator, outcomes)`: everything the other accessors
   * read, in one call.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3], 4).deal([1, 2]).info(); //=> [[[1, 4], [2, 4], [3, 4]], [1, 2], 3, 2, 660, [1, 2, 3]]
   * ```
   */
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]] {
    return call("multi_deal_info", { multi_deal: this }) as [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
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
  override toString(): string { return (call("deal_str", { deal: this }) as [string, string])[1]; }
  /** Python's `repr(multi_deal)`.
   *
   * @example
   * ```ts
   * new Deck([1, 2, 3]).deal([1, 2]).repr(); //=> "MultiDeal(Deck({1: 1, 2: 1, 3: 1}), hand_groups=((1, 1), (2, 1)))"
   * ```
   */
  repr(): string { return (call("deal_str", { deal: this }) as [string, string])[0]; }
}

registerKind("deck", (h) => wrapHandle(Deck.prototype, h));
registerKind("deal", (h) => new Deal(HANDLE, h));
registerKind("multi_deal", (h) => new MultiDeal(HANDLE, h));
registerKind("tuple_expr", (h) => new MultisetTupleExpression(HANDLE, h));

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
  private readonly mapping: boolean;
  private readonly data: unknown;

  /** `Wallenius(data)`: `data` is either `[outcome, weight]` pairs (one card each) or a Map from outcomes to a
   * weight or `[weight, quantity]`. */
  constructor(data: WalleniusData) {
    this.mapping = data instanceof Map;
    this.data = data instanceof Map ? List.from(Array.from(data, ([k, v]) => [k, v])) : list(data);
    // Python checks the data in the constructor: a hand of 0 builds the weights and stops there.
    call("wallenius_deal_expr", { data: this.data, mapping: this.mapping, hand_size: 0 });
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
    return call("wallenius_deal_expr", { data: this.data, mapping: this.mapping, hand_size: handSize }) as MultisetExpression;
  }
}
