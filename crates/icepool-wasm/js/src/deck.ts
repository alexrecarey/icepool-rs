// Decks and deals (population/deck.py, generator/deal.py, generator/multi_deal.py, wallenius.py).
// INTERIM (task C6): held through the bridge (rpc.ts) until they move onto typed exports.

import { list, type Callback, type Die, type MapResult } from "./die.js";
import { KeepGenerator, type MultisetExpression } from "./multiset.js";
import { HANDLE, Handle, call, registerKind, wrapHandle } from "./rpc.js";
import { List, type Int, type Outcome } from "./values.js";

/** What `new Deck(...)` takes: outcomes, dice or decks (with times), or a Map of outcomes to counts. */
export type DeckOutcomes = readonly (Outcome | Die | Deck)[] | Map<Outcome, Int>;

/** `icepool.Deck`: `new Deck([1, 2, 3], 4)` (each card 4 times), `new Deck(new Map([["a", 2]]))`. */
export class Deck extends Handle implements Iterable<Outcome> {
  constructor(outcomes: DeckOutcomes, times: Int | readonly Int[] = 1) {
    super(HANDLE, -1);
    const made =
      outcomes instanceof Map
        ? call("deck", { outcomes: List.from(outcomes.keys()), times: List.from(outcomes.values()) })
        : call("deck", { outcomes: list(outcomes), times: list(times) });
    this.adopt(made, Deck);
  }
  /** A Deal of `handSize` cards, or a MultiDeal for an array of hand sizes. */
  deal(handSize: Int): Deal;
  deal(handSizes: readonly Int[]): MultiDeal;
  deal(handSize: Int | readonly Int[]): Deal | MultiDeal {
    return Array.isArray(handSize)
      ? (call("multi_deal", { deck: this, hand_sizes: list(handSize) }) as MultiDeal)
      : (call("deal", { deck: this, hand_size: handSize }) as Deal);
  }
  /** `deck.deal_groups(...[handSize, groupSize])`. */
  dealGroups(...groups: [Int, Int][]): MultiDeal {
    return call("deal_groups", { deck: this, hand_groups: List.from(groups.map(list)) }) as MultiDeal;
  }
  map(f: Callback<MapResult>, { star = null }: { star?: boolean | null } = {}): Deck {
    return call("deck_map", { deck: this, repl: f, star }) as Deck;
  }
  add(other: unknown): Deck { return deckOp(this, "+", other); }
  sub(other: unknown): Deck { return deckOp(this, "-", other); }
  and(other: unknown): Deck { return deckOp(this, "&", other); }
  or(other: unknown): Deck { return deckOp(this, "|", other); }
  xor(other: unknown): Deck { return deckOp(this, "^", other); }
  mul(n: Int): Deck { return call("deck_mul", { deck: this, operator: "*", n }) as Deck; }
  floordiv(n: Int): Deck { return call("deck_mul", { deck: this, operator: "//", n }) as Deck; }
  mod(n: Int): Deck { return call("deck_mul", { deck: this, operator: "%", n }) as Deck; }
  symmetricDifference(other: unknown): Deck {
    return call("deck_method", { deck: this, name: "symmetric_difference", arg: list(other) }) as Deck;
  }
  padToDenominator(denominator: Int, outcome: Outcome): Deck {
    return call("deck_pad_to_denominator", { deck: this, denominator, outcome }) as Deck;
  }
  /** Every sequence of `deals` cards dealt one by one, as a die of arrays. */
  sequence(deals: Int): Die {
    return call("deck_sequence", { deck: this, deals }) as Die;
  }
  items(): [Outcome, bigint][] {
    const info = call("deck_info", { deck: this }) as [[Outcome, Int][], Int, number];
    return info[0].map(([o, q]) => [o, BigInt(q)]);
  }
  outcomes(): Outcome[] {
    return call("deck_method", { deck: this, name: "keys", arg: null }) as Outcome[];
  }
  keys(): Outcome[] {
    return this.outcomes();
  }
  values(): bigint[] {
    return (call("deck_method", { deck: this, name: "values", arg: null }) as Int[]).map(BigInt);
  }
  get(outcome: Outcome): bigint {
    return BigInt(call("deck_method", { deck: this, name: "getitem", arg: outcome }) as Int);
  }
  /** The number of cards (`deck.size()`). */
  size(): bigint {
    return BigInt((call("deck_info", { deck: this }) as [unknown, Int, number])[1]);
  }
  /** The number of distinct outcomes. */
  get length(): number {
    return (call("deck_info", { deck: this }) as [unknown, Int, number])[2];
  }
  [Symbol.iterator](): Iterator<Outcome> {
    return this.outcomes()[Symbol.iterator]();
  }
  /** A statistic of a die with these quantities, by its Python name (`mean`, `variance`, ...). */
  stat(name: string): Outcome {
    return call("deck_stat", { deck: this, stat: name }) as Outcome;
  }
  mean(): Outcome { return this.stat("mean"); }
  variance(): Outcome { return this.stat("variance"); }
  sd(): number { return this.stat("sd") as number; }
  minOutcome(): Outcome { return this.stat("min_outcome"); }
  maxOutcome(): Outcome { return this.stat("max_outcome"); }
  denominator(): bigint { return BigInt(this.stat("denominator") as Int); }
}

function deckOp(deck: Deck, operator: string, other: unknown): Deck {
  return call("deck_op", { operator, a: deck, b: list(other) }) as Deck;
}

/** `icepool.Deck(outcomes, times)` without `new`. */
export function deck(outcomes: DeckOutcomes, times: Int | readonly Int[] = 1): Deck {
  return new Deck(outcomes, times);
}

/** `deck.deal(handSize)`: a multiset expression over the hand. */
export class Deal extends KeepGenerator {
  /** `(deck items, keep_tuple, hand_size, keep_size, denominator, outcomes)` */
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]] {
    return call("deal_info", { deal: this }) as [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
  }
  handSize(): Int { return this.info()[2]; }
  keepSize(): Int { return this.info()[3]; }
  denominator(): bigint { return BigInt(this.info()[4]); }
  outcomes(): Outcome[] { return this.info()[5]; }
  override toString(): string { return (call("deal_str", { deal: this }) as [string, string])[1]; }
  repr(): string { return (call("deal_str", { deal: this }) as [string, string])[0]; }
}

/** A multiset tuple expression: several hands (a `MultiDeal`, a tuple parameter of a multiset function, a slice
 * of either). `hand(i)` is one hand (Python's `x[i]`), `slice(...)` some of them. */
export class MultisetTupleExpression extends Handle {
  hand(index: Int): MultisetExpression {
    return call("mx_tuple_index", { expr: this, index }) as MultisetExpression;
  }
  slice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): MultisetTupleExpression {
    return call("mx_tuple_slice", { expr: this, start, stop, step }) as MultisetTupleExpression;
  }
}

/** `deck.deal([size, ...])`: several hands; `hand(i)` or `slice(...)` picks some. */
export class MultiDeal extends MultisetTupleExpression {
  /** `(deck items, hand_sizes, total_cards_dealt, len, denominator, outcomes)` */
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]] {
    return call("multi_deal_info", { multi_deal: this }) as [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
  }
  handSizes(): Int[] { return this.info()[1]; }
  totalCardsDealt(): Int { return this.info()[2]; }
  denominator(): bigint { return BigInt(this.info()[4]); }
  outcomes(): Outcome[] { return this.info()[5]; }
  override toString(): string { return (call("deal_str", { deal: this }) as [string, string])[1]; }
  repr(): string { return (call("deal_str", { deal: this }) as [string, string])[0]; }
}

registerKind("deck", (h) => wrapHandle(Deck.prototype, h));
registerKind("deal", (h) => new Deal(HANDLE, h));
registerKind("multi_deal", (h) => new MultiDeal(HANDLE, h));
registerKind("tuple_expr", (h) => new MultisetTupleExpression(HANDLE, h));

/** What `new Wallenius(...)` takes: `[outcome, weight]` pairs, or a Map of outcome -> weight or
 * `[weight, quantity]`. */
export type WalleniusData = readonly (readonly [Outcome, Int])[] | Map<Outcome, Int | readonly [Int, Int]>;

/** `icepool.Wallenius(data)` (EXPERIMENTAL in icepool): Wallenius' noncentral hypergeometric distribution, sampling
 * without replacement with weights, where the whole weight of a card goes away when it is pulled. A Map is a Python
 * dict (keys equal by Python's `==`). */
export class Wallenius {
  private readonly mapping: boolean;
  private readonly data: unknown;

  constructor(data: WalleniusData) {
    this.mapping = data instanceof Map;
    this.data = data instanceof Map ? List.from(Array.from(data, ([k, v]) => [k, v])) : list(data);
    // Python checks the data in the constructor: a hand of 0 builds the weights and stops there.
    call("wallenius_deal_expr", { data: this.data, mapping: this.mapping, hand_size: 0 });
  }
  /** The multiset of `handSize` outcomes dealt, as a multiset expression (`.sum()`, `.expand()`, ...). */
  deal(handSize: Int): MultisetExpression {
    return call("wallenius_deal_expr", { data: this.data, mapping: this.mapping, hand_size: handSize }) as MultisetExpression;
  }
}
