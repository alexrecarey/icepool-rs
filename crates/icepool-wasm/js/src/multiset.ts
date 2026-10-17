// Pools and multiset expressions (generator/pool.py, generator/keep.py, expression/multiset_expression.py).
// INTERIM (task C6): held through the bridge (rpc.ts) until they move onto typed exports.

import { list, opts, type Comparison, type Die, type Operand } from "./die.js";
import { HANDLE, Handle, call, registerKind, wrapHandle } from "./rpc.js";
import { List, Order, type Ellipsis, type Int, type OrderValue, type Outcome } from "./values.js";

/** Where a multiset expression is expected, an array of outcomes is a pool of them. */
export type MultisetArg = MultisetExpression | readonly Outcome[];

/** How `sort_pair` treats unpaired elements. */
export type SortPairExtra = "early" | "late" | "low" | "high" | "equal" | "keep" | "drop";

/** Any `MultisetExpression`: a pool, a deal, or an operator over them. Evaluation methods return dice. */
export class MultisetExpression extends Handle {
  // Multiset operators: `+` `-` `&` `|` `^` (the other side may be an array of outcomes).
  additiveUnion(...others: MultisetArg[]): Expression {
    return mxMethod("additive_union", [this, ...others]);
  }
  difference(...rest: (MultisetArg | { keepNegativeCounts?: boolean })[]): Expression {
    const [others, { keepNegativeCounts = false }] = opts<{ keepNegativeCounts: boolean }>(rest);
    return call("mx_method", {
      name: "difference",
      args: List.of(this, ...others.map(list)),
      keep_negative_counts: keepNegativeCounts,
    }) as Expression;
  }
  intersection(...others: MultisetArg[]): Expression {
    return mxMethod("intersection", [this, ...others]);
  }
  union(...others: MultisetArg[]): Expression {
    return mxMethod("union", [this, ...others]);
  }
  symmetricDifference(other: MultisetArg): Expression {
    return call("mx_binary", { a: this, o: "^", b: list(other) }) as Expression;
  }
  add(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "+", b: list(other) }) as MultisetExpression;
  }
  sub(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "-", b: list(other) }) as MultisetExpression;
  }
  and(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "&", b: list(other) }) as MultisetExpression;
  }
  or(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "|", b: list(other) }) as MultisetExpression;
  }
  xor(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "^", b: list(other) }) as MultisetExpression;
  }

  // Counts.
  mul(n: Int): MultisetExpression { return mxCounts(this, "mul", null, n); }
  neg(): MultisetExpression { return mxCounts(this, "neg", null, null); }
  pos(): MultisetExpression { return mxCounts(this, "pos", null, null); }
  floordiv(n: Int): MultisetExpression { return mxCounts(this, "floordiv", null, n); }
  mod(n: Int): MultisetExpression { return mxCounts(this, "mod", null, n); }
  unique(n: Int = 1): MultisetExpression { return mxCounts(this, "unique", null, n); }
  keepCounts(comparison: Comparison, n: Int): MultisetExpression { return mxCounts(this, "keep_counts", comparison, n); }
  multiplyCounts(n: Int): MultisetExpression { return mxCounts(this, "multiply_counts", null, n); }
  divideCounts(n: Int): MultisetExpression { return mxCounts(this, "divide_counts", null, n); }
  moduloCounts(n: Int): MultisetExpression { return mxCounts(this, "modulo_counts", null, n); }

  // Outcomes.
  keepOutcomes(target: readonly Outcome[] | ((outcome: never) => unknown) | MultisetExpression): MultisetExpression {
    return call("mx_filter", { expr: this, invert: false, target: list(target) }) as MultisetExpression;
  }
  dropOutcomes(target: readonly Outcome[] | ((outcome: never) => unknown) | MultisetExpression): MultisetExpression {
    return call("mx_filter", { expr: this, invert: true, target: list(target) }) as MultisetExpression;
  }
  forceOrder(order: OrderValue): MultisetExpression {
    return call("mx_force_order", { expr: this, order }) as MultisetExpression;
  }
  /** `MultisetGenerator.weightless()`: each possible multiset weighted 1. Only generators (pools, deals, compound
   * keeps, weightless ones) have it in Python; on another expression the core raises AttributeError. */
  weightless(): MultisetExpression {
    return call("weightless", { generator: this }) as MultisetExpression;
  }

  // Pairing.
  maxPairKeep(comparison: Comparison, other: MultisetArg, priority: "low" | "high" | null = null): MultisetExpression {
    return call("mx_max_pair", { expr: this, keep: true, comparison, other: list(other), priority }) as MultisetExpression;
  }
  maxPairDrop(comparison: Comparison, other: MultisetArg, priority: "low" | "high" | null = null): MultisetExpression {
    return call("mx_max_pair", { expr: this, keep: false, comparison, other: list(other), priority }) as MultisetExpression;
  }
  sortPair(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair", comparison, other, order, extra);
  }
  sortPairKeepWhile(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair_keep_while", comparison, other, order, extra);
  }
  sortPairDropWhile(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair_drop_while", comparison, other, order, extra);
  }
  versusAll(comparison: Comparison, other: MultisetArg): MultisetExpression {
    return call("mx_versus", { expr: this, all: true, comparison, other: list(other) }) as MultisetExpression;
  }
  versusAny(comparison: Comparison, other: MultisetArg): MultisetExpression {
    return call("mx_versus", { expr: this, all: false, comparison, other: list(other) }) as MultisetExpression;
  }

  // Evaluation.
  sum(): Die { return mxEvaluate(this, "sum"); }
  size(): Die { return mxEvaluate(this, "size"); }
  empty(): Die { return mxEvaluate(this, "empty"); }
  any(): Die { return mxEvaluate(this, "any"); }
  productOfCounts(): Die { return mxEvaluate(this, "product_of_counts"); }
  highestOutcomeAndCount(): Die { return mxEvaluate(this, "highest_outcome_and_count"); }
  largestCount(): Die { return mxEvaluate(this, "largest_count"); }
  largestCountAndOutcome(): Die { return mxEvaluate(this, "largest_count_and_outcome"); }
  largestStraight(): Die { return mxEvaluate(this, "largest_straight"); }
  allStraights(): Die { return mxEvaluate(this, "all_straights"); }
  largestStraightAndOutcome(priority: "high" | "low" = "high"): Die {
    return call("pool_largest_straight_and_outcome", { pool: this, priority }) as Die;
  }
  /** All elements as a sorted tuple (a die of arrays). */
  expand(order: OrderValue = Order.Ascending): Die {
    return call("pool_expand", { pool: this, order }) as Die;
  }
  allCounts(filter: Int | "all" = 1): Die {
    return call("pool_all_counts", { pool: this, filter }) as Die;
  }
  countSubset(divisor: MultisetArg, { emptyDivisor = null }: { emptyDivisor?: Int | null } = {}): Die {
    return call("pool_count_subset", { pool: this, divisor: list(divisor), empty_divisor: emptyDivisor }) as Die;
  }
  isdisjoint(other: MultisetArg): Die { return call("pool_isdisjoint", { pool: this, other: list(other) }) as Die; }
  issubset(other: MultisetArg): Die { return call("pool_issubset", { pool: this, other: list(other) }) as Die; }
  issuperset(other: MultisetArg): Die { return call("pool_issuperset", { pool: this, other: list(other) }) as Die; }
  /** `self < other` (a proper subset), as a die of bools. */
  lt(other: MultisetArg): Die { return poolCompare(this, other, "proper_subset"); }
  /** `self > other` (a proper superset), as a die of bools. */
  gt(other: MultisetArg): Die { return poolCompare(this, other, "proper_superset"); }
  /** `self == other` as multisets (a die of bools). */
  equalSet(other: MultisetArg): Die { return poolCompare(this, other, "equal_set"); }
  notEqualSet(other: MultisetArg): Die { return poolCompare(this, other, "not_equal_set"); }

  /** `MultisetExpression.map_counts(...args, { function })`. */
  static mapCounts(...rest: (MultisetArg | { function: (outcome: never, ...counts: never[]) => Int })[]): Expression {
    const [args, { function: f }] = opts<{ function: (outcome: never, ...counts: never[]) => Int }>(rest);
    return call("mx_map_counts", { args: List.from(args.map(list)), function: f }) as Expression;
  }
}

function mxMethod(name: string, args: unknown[]): Expression {
  return call("mx_method", { name, args: List.from(args.map(list)), keep_negative_counts: false }) as Expression;
}
function mxCounts(expr: MultisetExpression, o: string, comparison: Comparison | null, n: Int | null): MultisetExpression {
  return call("mx_counts", { expr, o, comparison, n }) as MultisetExpression;
}
function mxEvaluate(expr: MultisetExpression, method: string): Die {
  return call("mx_evaluate", { expr, method }) as Die;
}
function poolCompare(pool: MultisetExpression, other: MultisetArg, cmp: string): Die {
  return call("pool_compare", { pool, other: list(other), cmp }) as Die;
}
function sortPair(expr: MultisetExpression, method: string, comparison: Comparison, other: MultisetArg, order: OrderValue, extra: SortPairExtra): MultisetExpression {
  return call("mx_sort_pair", { expr, method, comparison, other: list(other), order, extra }) as MultisetExpression;
}

/** A `KeepGenerator`: a pool or a deal, whose sorted rolls can be kept or dropped. */
export class KeepGenerator extends MultisetExpression {
  highest(keep: Int | null = null, drop: Int | null = null): this {
    return call("pool_highest", { pool: this, keep, drop }) as this;
  }
  lowest(keep: Int | null = null, drop: Int | null = null): this {
    return call("pool_lowest", { pool: this, keep, drop }) as this;
  }
  middle(keep: Int = 1, { tie = "error" }: { tie?: "error" | "high" | "low" } = {}): this {
    return call("pool_middle", { pool: this, keep, tie }) as this;
  }
  /** `pool.keep(index)`: an array of counts (a keep tuple, `Ellipsis` allowed), or an int: that sorted roll as a
   * die. An array is a tuple, as everywhere: a `List` index is unhashable (a mixture of such keeps raises). */
  keep(index: readonly (Int | typeof Ellipsis)[]): this;
  keep(index: Int): Die;
  keep(index: Int | readonly (Int | typeof Ellipsis)[]): this | Die {
    return call("pool_keep", { pool: this, index }) as this | Die;
  }
  /** `pool[index]`: the roll at that sorted position, as a die. */
  at(index: Int): Die {
    return call("pool_index", { pool: this, index }) as Die;
  }
  /** `pool[start:stop:step]`. */
  slice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): this {
    return call("pool_slice", { pool: this, start, stop, step }) as this;
  }
}

/** What `new Pool(...)` takes: dice or outcomes (with times), or a Map of them to counts. */
export type PoolDice = readonly Operand[] | Map<Operand, Int>;

/** `icepool.Pool`: `new Pool([d6, d6, d8])`, `new Pool([d6, d8], [2, 1])`, `new Pool(new Map([[d6, 3]]))`. */
export class Pool extends KeepGenerator {
  constructor(dice: PoolDice, times: Int | readonly Int[] = 1) {
    super(HANDLE, -1);
    const made =
      dice instanceof Map
        ? call("pool_dict", { items: List.from(Array.from(dice, ([k, v]) => [k, v])), times: list(times) })
        : call("pool", { dice: list(dice), times: list(times) });
    this.adopt(made, Pool);
  }
  /** `(keep_tuple(), keep_size(), raw_size(), denominator(), outcomes(), _dice)` */
  info(): [Int[], Int, Int, Int, Outcome[], [Die, Int][]] {
    return call("pool_info", { pool: this }) as [Int[], Int, Int, Int, Outcome[], [Die, Int][]];
  }
  keepTuple(): Int[] { return this.info()[0]; }
  keepSize(): Int { return this.info()[1]; }
  rawSize(): Int { return this.info()[2]; }
  denominator(): bigint { return BigInt(this.info()[3]); }
  outcomes(): Outcome[] { return this.info()[4]; }
}

/** `icepool.Pool(dice, times)` without `new`. */
export function pool(dice: PoolDice, times: Int | readonly Int[] = 1): Pool {
  return new Pool(dice, times);
}
/** One d(size) per size; a Map of size -> count. */
export function dPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return standardPool("d_pool", dieSizes);
}
/** One z(size) per size; a Map of size -> count. */
export function zPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return standardPool("z_pool", dieSizes);
}
function standardPool(op: string, dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return dieSizes instanceof Map
    ? (call(op, { die_sizes: List.from(Array.from(dieSizes, ([k, v]) => [k, v])), mapping: true }) as Pool)
    : (call(op, { die_sizes: list(dieSizes), mapping: false }) as Pool);
}

/** `icepool.MultisetMixture(inners)`. */
export function multisetMixture(inners: readonly MultisetExpression[]): Expression {
  return call("mx_mixture", { inners: list(inners) }) as Expression;
}

/** Any other multiset expression (an operator over pools, a CompoundKeepGenerator...). */
export class Expression extends MultisetExpression {}

registerKind("pool", (h) => wrapHandle(Pool.prototype, h));
registerKind("expr", (h) => new Expression(HANDLE, h));
