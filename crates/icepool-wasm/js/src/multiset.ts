// Pools and multiset expressions (generator/pool.py, generator/keep.py, expression/multiset_expression.py).
// INTERIM (task C6): held through the bridge (rpc.ts) until they move onto typed exports.

import { AgainExpression } from "./again.js";
import { list, opts, type Comparison, type Die, type Operand } from "./die.js";
import { HANDLE, Handle, call, registerKind, wrapHandle } from "./rpc.js";
import { List, Order, type Ellipsis, type Int, type OrderValue, type Outcome } from "./values.js";

/** Where a multiset expression is expected, an array of outcomes is a pool of them (Python's implicit conversion
 * of a sequence to a `Pool`).
 *
 * @example
 * ```ts
 * import { d, pool, type MultisetArg } from "icepool-wasm";
 *
 * const kept: MultisetArg = [5, 6];
 * d(6).pool(3).keepOutcomes(kept).size().probability(0).toString(); //=> "8/27"
 * pool([1, 2, 2, 3]).sub([2, 3]).expand().items(); //=> [[[1, 2], 1n]]
 * ```
 */
export type MultisetArg = MultisetExpression | readonly Outcome[];

/** How `sortPair` (and `sortPairKeepWhile`, `sortPairDropWhile`) treats the extra elements when the left operand has
 * more elements than the right:
 *
 * - `"early"`, `"late"`: the extra elements count as occurring earlier or later in `order` than their missing
 *   counterparts;
 * - `"low"`, `"high"`, `"equal"`: they count as lower than, higher than or equal to their missing counterparts;
 * - `"keep"`, `"drop"`: they are always kept or dropped. For the `...While` methods these are Python's `'continue'`
 *   and `'break'` (icepool treats each pair of names alike): whether the "while", if it still holds when it reaches
 *   them, goes on over the extra elements.
 *
 * @example
 * ```ts
 * import { Order, pool, type SortPairExtra } from "icepool-wasm";
 *
 * const extra: SortPairExtra = "keep";
 * pool([6, 4, 3]).sortPair(">", [5, 5], Order.Descending, extra).expand().items(); //=> [[[3, 6], 1n]]
 * ```
 */
export type SortPairExtra = "early" | "late" | "low" | "high" | "equal" | "keep" | "drop";

/** `MultisetExpression`: any expression over multisets: a pool, a deal, or an operator over them. Evaluation methods
 * (`sum`, `size`, `expand`, ...) return dice.
 *
 * Operators act on the multiset of rolls, not of dice: `d(6).pool(3).sub(d(6).pool(3))` is not an empty pool but
 * "roll two pools of 3d6, the rolls of the second cancelling matching rolls of the first one for one". Wherever
 * another multiset is taken, an array of outcomes may stand for one ({@link MultisetArg}).
 *
 * | Operation (Python)                    | Count                              |
 * |:--------------------------------------|:-----------------------------------|
 * | `additiveUnion`, `add` (`+`)          | `l + r`                            |
 * | `difference`, `sub` (`-`)             | `l - r`                            |
 * | `intersection`, `and` (`&`)           | `min(l, r)`                        |
 * | `union`, `or` (`\|`)                  | `max(l, r)`                        |
 * | `symmetricDifference`, `xor` (`^`)    | `abs(l - r)`                       |
 * | `multiplyCounts`, `mul` (`*`)         | `count * n`                        |
 * | `divideCounts`, `floordiv` (`//`)     | `count // n`                       |
 * | `moduloCounts`, `mod` (`%`)           | `count % n`                        |
 * | `keepCounts`                          | `count if count >= n else 0` etc.  |
 * | `pos` (unary `+`)                     | same as `keepCounts(">=", 0)`      |
 * | `neg` (unary `-`)                     | reverses the sign of all counts    |
 * | `unique`                              | `min(count, n)`                    |
 * | `keepOutcomes`                        | `count if outcome in t else 0`     |
 * | `dropOutcomes`                        | `count if outcome not in t else 0` |
 * | `mapCounts`                           | `f(outcome, ...counts)`            |
 * | `keep`, `at`, `slice`, `highest`, ... | less capable than on a pool        |
 *
 * For joint evaluations (several results from one roll) see {@link multisetFunction}; for evaluations of your own,
 * {@link MultisetEvaluator}.
 *
 * @example
 * ```ts
 * // 4d6, keep the highest 3.
 * d(6).pool(4).highest(3).sum().mean().toString(); //=> "15869/1296"
 * // How many of 3d6 roll a 5 or a 6?
 * d(6).pool(3).keepOutcomes([5, 6]).size().probability(">=", 2).toString(); //=> "7/27"
 * ```
 */
export class MultisetExpression extends Handle {
  // Multiset operators: `+` `-` `&` `|` `^` (the other side may be an array of outcomes).

  /** `MultisetExpression.additive_union(*args)`: the combined elements of all the multisets: for each outcome, the
   * counts are summed. The same as `a.add(b).add(c)...` (Python's `a + b + c`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).additiveUnion([1, 2, 4]).expand().items(); //=> [[[1, 1, 2, 2, 2, 3, 4], 1n]]
   * ```
   */
  additiveUnion(...others: MultisetArg[]): Expression {
    return mxMethod("additive_union", [this, ...others]);
  }
  /** `MultisetExpression.difference(*args, keep_negative_counts=False)`: the elements of this multiset that are not
   * in any of the others: for each outcome, this count minus the counts of all the others. A negative result is
   * set to zero unless `keepNegativeCounts`. The same as `a.sub(b).sub(c)...` (Python's `a - b - c`).
   *
   * As a multiset operation this cancels elements only one for one: to drop all elements of some outcomes
   * whatever their count, use `dropOutcomes` (or a large count on the right).
   *
   * @param rest - The multisets to subtract, then optionally `{ keepNegativeCounts }` (default false).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).difference([1, 2, 4]).expand().items(); //=> [[[2, 3], 1n]]
   * // With negative counts kept, the 2 counts -1 times.
   * pool([1, 2]).difference([2, 2], { keepNegativeCounts: true }).sum().items(); //=> [[-1, 1n]]
   * ```
   */
  difference(...rest: (MultisetArg | { keepNegativeCounts?: boolean })[]): Expression {
    const [others, { keepNegativeCounts = false }] = opts<{ keepNegativeCounts: boolean }>(rest);
    return call("mx_method", {
      name: "difference",
      args: List.of(this, ...others.map(list)),
      keep_negative_counts: keepNegativeCounts,
    }) as Expression;
  }
  /** `MultisetExpression.intersection(*args)`: the elements all the multisets have in common: for each outcome, the
   * least count among them. The same as `a.and(b).and(c)...` (Python's `a & b & c`).
   *
   * This intersects elements only one for one: to keep all elements of some outcomes whatever their count, use
   * `keepOutcomes` (or a large count on the right).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).intersection([1, 2, 4]).expand().items(); //=> [[[1, 2], 1n]]
   * ```
   */
  intersection(...others: MultisetArg[]): Expression {
    return mxMethod("intersection", [this, ...others]);
  }
  /** `MultisetExpression.union(*args)`: the most of each outcome that appears in any of the multisets: for each
   * outcome, the greatest count among them. The same as `a.or(b).or(c)...` (Python's `a | b | c`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).union([1, 2, 4]).expand().items(); //=> [[[1, 2, 2, 3, 4], 1n]]
   * ```
   */
  union(...others: MultisetArg[]): Expression {
    return mxMethod("union", [this, ...others]);
  }
  /** `MultisetExpression.symmetric_difference(other)`: the elements in this or the other multiset but not both: for
   * each outcome, the absolute difference of the two counts. The same as `xor` (Python's `a ^ b`). Negative input
   * counts take part; to ignore them, use `a.pos().xor(b.pos())`.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).symmetricDifference([1, 2, 4]).expand().items(); //=> [[[2, 3, 4], 1n]]
   * ```
   */
  symmetricDifference(other: MultisetArg): Expression {
    return call("mx_binary", { a: this, o: "^", b: list(other) }) as Expression;
  }
  /** Python's `self + other`: {@link MultisetExpression.additiveUnion} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).add([3, 4]).expand().items(); //=> [[[1, 2, 2, 3, 3, 4], 1n]]
   * ```
   */
  add(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "+", b: list(other) }) as MultisetExpression;
  }
  /** Python's `self - other`: {@link MultisetExpression.difference} of two multisets, negative counts set to zero.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).sub([2, 3]).expand().items(); //=> [[[1, 2], 1n]]
   * ```
   */
  sub(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "-", b: list(other) }) as MultisetExpression;
  }
  /** Python's `self & other`: {@link MultisetExpression.intersection} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).and([2, 2, 3, 3]).expand().items(); //=> [[[2, 2, 3], 1n]]
   * ```
   */
  and(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "&", b: list(other) }) as MultisetExpression;
  }
  /** Python's `self | other`: {@link MultisetExpression.union} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).or([2, 2, 3, 3]).expand().items(); //=> [[[1, 2, 2, 3, 3], 1n]]
   * ```
   */
  or(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "|", b: list(other) }) as MultisetExpression;
  }
  /** Python's `self ^ other`: the same as {@link MultisetExpression.symmetricDifference}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).xor([2, 3, 3]).expand().items(); //=> [[[1, 2, 3], 1n]]
   * ```
   */
  xor(other: MultisetArg): MultisetExpression {
    return call("mx_binary", { a: this, o: "^", b: list(other) }) as MultisetExpression;
  }

  // Keeping some of the sorted elements.
  /** `MultisetExpression.highest(keep=None, drop=None)`: keeps some of the highest elements and drops the rest.
   * Unlike the die and free-function versions this does not sum: follow it with `.sum()`, or another evaluation.
   * On a general expression this needs the outcomes to be evaluated in descending order; on a pool or a deal see
   * {@link KeepGenerator.highest}.
   *
   * `keep` and `drop` work together: neither given keeps the single highest element; only `keep` keeps the `keep`
   * highest; only `drop` drops the `drop` highest and keeps the rest; both drop the `drop` highest and then keep
   * the next `keep` highest.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 4]).highest(2).expand().items(); //=> [[[3, 4], 1n]]
   * pool([1, 2, 3, 4]).highest(2, 1).expand().items(); //=> [[[2, 3], 1n]]
   * pool([1, 2]).add([3, 4]).highest(1).sum().items(); //=> [[4, 1n]]
   * ```
   */
  highest(keep: Int | null = null, drop: Int | null = null): MultisetExpression {
    return call("pool_highest", { pool: this, keep, drop }) as MultisetExpression;
  }
  /** `MultisetExpression.lowest(keep=None, drop=None)`: keeps some of the lowest elements and drops the rest, as
   * {@link MultisetExpression.highest} from the other end. On a general expression this needs the outcomes to be
   * evaluated in ascending order.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 4]).lowest(2).expand().items(); //=> [[[1, 2], 1n]]
   * pool([1, 2, 3, 4]).lowest(1, 1).expand().items(); //=> [[[2], 1n]]
   * ```
   */
  lowest(keep: Int | null = null, drop: Int | null = null): MultisetExpression {
    return call("pool_lowest", { pool: this, keep, drop }) as MultisetExpression;
  }
  /** `expr.keep(index)`: selects elements after sorting them in ascending order. `index` is either
   *
   * - an int: the element at that sorted position, as a die (as {@link MultisetExpression.at}); or
   * - an array of counts (Python's keep tuple): each sorted element is counted that many times. On a general
   *   expression `Ellipsis` must be at exactly one end, standing for enough zeros to fill the rest; a pool or a deal
   *   takes more ({@link KeepGenerator.keep}).
   *
   * An array is a tuple, as everywhere: a `List` index is unhashable (a mixture of such keeps raises). This version
   * does not know how many elements it selects from, so it must be anchored at the starting end; its advantage is
   * that it applies to any expression. Negative incoming counts count as zero.
   *
   * @example
   * ```ts
   * const roll = pool([1, 2, 3]).union([4]); // an expression, not a pool
   * roll.keep([1, Ellipsis]).expand().items(); //=> [[[1], 1n]]
   * roll.keep(0).items(); //=> [[1, 1n]]
   * ```
   */
  keep(index: readonly (Int | typeof Ellipsis)[]): MultisetExpression;
  keep(index: Int): Die;
  keep(index: Int | readonly (Int | typeof Ellipsis)[]): MultisetExpression | Die {
    return call("pool_keep", { pool: this, index }) as MultisetExpression | Die;
  }
  /** `expr[index]`: the element at that sorted (ascending) position, as a die. Negative indices count from the
   * highest.
   *
   * @example
   * ```ts
   * // The lowest and the highest of 3d6.
   * d(6).pool(3).at(0).mean().toString(); //=> "49/24"
   * d(6).pool(3).at(-1).mean().toString(); //=> "119/24"
   * ```
   */
  at(index: Int): Die {
    return call("pool_index", { pool: this, index }) as Die;
  }
  /** `expr[start:stop:step]`: the sorted (ascending) elements in that slice, each counted once. On a general
   * expression, `start` and `stop` (if both given) must both be non-negative or both negative; no `step` is
   * supported (neither is it on a pool).
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 4]).slice(1, 3).expand().items(); //=> [[[2, 3], 1n]]
   * ```
   */
  slice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): MultisetExpression {
    return call("pool_slice", { pool: this, start, stop, step }) as MultisetExpression;
  }

  // Counts.
  /** Python's `self * n`: the same as {@link MultisetExpression.multiplyCounts}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).mul(2).expand().items(); //=> [[[1, 1, 2, 2, 2, 2, 3, 3], 1n]]
   * ```
   */
  mul(n: Int): MultisetExpression { return mxCounts(this, "mul", null, n); }
  /** Python's `-self`: reverses the sign of all counts (as `mul(-1)`). Negative counts take part in later operators
   * and evaluations; `expand` leaves them out.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).neg().expand().items(); //=> [[[], 1n]]
   * // Each 6 cancels a 1 of the same roll.
   * d(6).pool(3).keepOutcomes([1]).add(d(6).pool(3).keepOutcomes([6]).neg()).size().mean().toString(); //=> "0"
   * ```
   */
  neg(): MultisetExpression { return mxCounts(this, "neg", null, null); }
  /** Python's `+self`: sets all negative counts to zero (the same as `keepCounts(">=", 0)`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).difference([2, 2, 2], { keepNegativeCounts: true }).pos().expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  pos(): MultisetExpression { return mxCounts(this, "pos", null, null); }
  /** Python's `self // n` with an int: the same as {@link MultisetExpression.divideCounts}. (`//` with a multiset is
   * {@link MultisetExpression.countSubset}.)
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).floordiv(2).expand().items(); //=> [[[2], 1n]]
   * ```
   */
  floordiv(n: Int): MultisetExpression { return mxCounts(this, "floordiv", null, n); }
  /** Python's `self % n`: the same as {@link MultisetExpression.moduloCounts}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).mod(2).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  mod(n: Int): MultisetExpression { return mxCounts(this, "mod", null, n); }
  /** `MultisetExpression.unique(n=1)`: counts each outcome at most `n` times.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).unique().expand().items(); //=> [[[1, 2, 3], 1n]]
   * // How many different faces show on 3d6?
   * d(6).pool(3).unique().size().items(); //=> [[1, 6n], [2, 90n], [3, 120n]]
   * ```
   */
  unique(n: Int = 1): MultisetExpression { return mxCounts(this, "unique", null, n); }
  /** `MultisetExpression.keep_counts(comparison, n)`: keeps the counts that fit the comparison with `n`, treating
   * the rest as zero. `keepCounts(">=", 2)` keeps pairs, triplets, etc. and drops singles.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 3, 3]).keepCounts(">=", 2).expand().items(); //=> [[[2, 2, 3, 3, 3], 1n]]
   * ```
   */
  keepCounts(comparison: Comparison, n: Int): MultisetExpression { return mxCounts(this, "keep_counts", comparison, n); }
  /** `MultisetExpression.multiply_counts(n)`: multiplies all counts by `n` (Python's `self * n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).multiplyCounts(2).expand().items(); //=> [[[1, 1, 2, 2, 2, 2, 3, 3], 1n]]
   * ```
   */
  multiplyCounts(n: Int): MultisetExpression { return mxCounts(this, "multiply_counts", null, n); }
  /** `MultisetExpression.divide_counts(n)`: divides all counts by `n`, rounding down (Python's `self // n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).divideCounts(2).expand().items(); //=> [[[2], 1n]]
   * ```
   */
  divideCounts(n: Int): MultisetExpression { return mxCounts(this, "divide_counts", null, n); }
  /** `MultisetExpression.modulo_counts(n)`: each count modulo `n` (Python's `self % n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).moduloCounts(2).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  moduloCounts(n: Int): MultisetExpression { return mxCounts(this, "modulo_counts", null, n); }

  // Outcomes.
  /** `MultisetExpression.keep_outcomes(outcomes)`: keeps the given outcomes and drops the rest, setting their counts
   * to zero. Like `intersection`, except that the right side counts as having unlimited multiplicity.
   *
   * @param target - A function returning true for the outcomes to keep, or an array or expression of them.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 4]).keepOutcomes([2, 4]).expand().items(); //=> [[[2, 2, 4], 1n]]
   * pool([1, 2, 3, 4]).keepOutcomes((x: number) => x % 2 === 1).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  keepOutcomes(target: readonly Outcome[] | ((outcome: never) => unknown) | MultisetExpression): MultisetExpression {
    return call("mx_filter", { expr: this, invert: false, target: list(target) }) as MultisetExpression;
  }
  /** `MultisetExpression.drop_outcomes(outcomes)`: drops the given outcomes, setting their counts to zero, and keeps
   * the rest. Like `difference`, except that the right side counts as having unlimited multiplicity.
   *
   * @param target - A function returning true for the outcomes to drop, or an array or expression of them.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 4]).dropOutcomes([2, 4]).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  dropOutcomes(target: readonly Outcome[] | ((outcome: never) => unknown) | MultisetExpression): MultisetExpression {
    return call("mx_filter", { expr: this, invert: true, target: list(target) }) as MultisetExpression;
  }
  /** `MultisetExpression.force_order(order)`: forces evaluators to see the outcomes in the given order; `Order.Any`
   * leaves the expression as it is. Useful for debugging and testing.
   *
   * @example
   * ```ts
   * d(6).pool(3).forceOrder(Order.Descending).sum().mean().toString(); //=> "21/2"
   * ```
   */
  forceOrder(order: OrderValue): MultisetExpression {
    return call("mx_force_order", { expr: this, order }) as MultisetExpression;
  }
  /** `MultisetGenerator.weightless()`: each possible multiset weighted 1. Only generators (pools, deals, compound
   * keeps, weightless ones) have it in Python; on another expression the core raises AttributeError.
   *
   * @example
   * ```ts
   * // 2d6 rolls 21 different multisets, 36 sequences.
   * d(6).pool(2).weightless().expand().denominator(); //=> 21n
   * d(6).pool(2).expand().denominator(); //=> 36n
   * ```
   */
  weightless(): MultisetExpression {
    return call("weightless", { generator: this }) as MultisetExpression;
  }

  // Pairing.
  /** `MultisetExpression.max_pair_keep(comparison, other, priority=None)` (experimental): forms as many pairs of an
   * element of this multiset and one of `other` as possible such that each pair fits the comparison, then keeps
   * the paired elements of this multiset and drops the rest. `priority` (`"low"` or `"high"`) prefers pairing the
   * lowest or highest elements; it does not change how many are paired. `"=="` is the same as
   * `this.pos().and(other.pos())`.
   *
   * This needs the outcomes evaluated in descending order with priority `"high"`, ascending with `"low"`. It is not
   * designed for negative counts. Contrast {@link MultisetExpression.sortPair}, which pairs in sorted order first.
   *
   * @example
   * ```ts
   * // Defender dice block attacker dice of equal or lower value, the highest they can: which are blocked?
   * pool([6, 4, 3, 1]).maxPairKeep("<=", [5, 5], "high").expand().items(); //=> [[[3, 4], 1n]]
   * ```
   */
  maxPairKeep(comparison: Comparison, other: MultisetArg, priority: "low" | "high" | null = null): MultisetExpression {
    return call("mx_max_pair", { expr: this, keep: true, comparison, other: list(other), priority }) as MultisetExpression;
  }
  /** `MultisetExpression.max_pair_drop(comparison, other, priority=None)` (experimental): as
   * {@link MultisetExpression.maxPairKeep}, but drops the paired elements of this multiset and keeps the rest.
   * `"=="` is the same as `this.sub(other)`.
   *
   * @example
   * ```ts
   * // Which attacker dice are not blocked?
   * pool([6, 4, 3, 1]).maxPairDrop("<=", [5, 5], "high").expand().items(); //=> [[[1, 6], 1n]]
   * ```
   */
  maxPairDrop(comparison: Comparison, other: MultisetArg, priority: "low" | "high" | null = null): MultisetExpression {
    return call("mx_max_pair", { expr: this, keep: false, comparison, other: list(other), priority }) as MultisetExpression;
  }
  /** `MultisetExpression.sort_pair(comparison, other, order=Order.Descending, extra='drop')` (experimental): sorts
   * both multisets and pairs their elements in that order, then keeps the elements of this multiset whose pairs fit
   * the comparison. To drop rather than keep, use the complementary comparison (`"=="`/`"!="`, `"<="`/`">"`,
   * `">="`/`"<"`). `extra` says what happens to elements of this multiset left without a pair
   * ({@link SortPairExtra}). Not designed for negative counts.
   *
   * Contrast {@link MultisetExpression.maxPairKeep}, which pairs to fit the comparison as often as possible.
   *
   * @example
   * ```ts
   * // RISK: the attacker's 6, 4, 3 against the defender's 5, 5. Which pairs did the attacker win?
   * pool([6, 4, 3]).sortPair(">", [5, 5]).expand().items(); //=> [[[6], 1n]]
   * pool([6, 4, 3]).sortPair(">", [5, 5], Order.Descending, "keep").expand().items(); //=> [[[3, 6], 1n]]
   * ```
   */
  sortPair(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair", comparison, other, order, extra);
  }
  /** `MultisetExpression.sort_pair_keep_while(comparison, other, order=Order.Descending, extra='break')`
   * (experimental): sorts both multisets and pairs their elements in that order, then keeps elements of this
   * multiset while their pairs fit the comparison, dropping the rest from the first pair that does not. `extra`
   * ({@link SortPairExtra}) says what happens to the unpaired elements; the default `"drop"` is Python's `'break'`.
   * Not designed for negative counts.
   *
   * @example
   * ```ts
   * pool([6, 4, 3]).sortPairKeepWhile(">", [5, 2]).expand().items(); //=> [[[4, 6], 1n]]
   * pool([6, 4, 3]).sortPairKeepWhile(">", [5, 2], Order.Descending, "keep").expand().items(); //=> [[[3, 4, 6], 1n]]
   * ```
   */
  sortPairKeepWhile(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair_keep_while", comparison, other, order, extra);
  }
  /** `MultisetExpression.sort_pair_drop_while(comparison, other, order=Order.Descending, extra='break')`
   * (experimental): sorts both multisets and pairs their elements in that order, then drops elements of this
   * multiset while their pairs fit the comparison, keeping the rest from the first pair that does not. `extra`
   * ({@link SortPairExtra}) says what happens to the unpaired elements; the default `"drop"` is Python's `'break'`.
   * Not designed for negative counts.
   *
   * @example
   * ```ts
   * pool([6, 4, 3]).sortPairDropWhile(">", [5, 2]).expand().items(); //=> [[[3], 1n]]
   * ```
   */
  sortPairDropWhile(comparison: Comparison, other: MultisetArg, order: OrderValue = Order.Descending, extra: SortPairExtra = "drop"): MultisetExpression {
    return sortPair(this, "sort_pair_drop_while", comparison, other, order, extra);
  }
  /** `MultisetExpression.versus_all(comparison, other)` (experimental): keeps the elements of this multiset that fit
   * the comparison (`"<="`, `"<"`, `">="` or `">"`) against every element of `other`. Negative counts in `other`
   * count as zero. Contrast {@link MultisetExpression.versusAny}.
   *
   * @example
   * ```ts
   * pool([1, 3, 5]).versusAll(">", [2, 4]).expand().items(); //=> [[[5], 1n]]
   * ```
   */
  versusAll(comparison: Comparison, other: MultisetArg): MultisetExpression {
    return call("mx_versus", { expr: this, all: true, comparison, other: list(other) }) as MultisetExpression;
  }
  /** `MultisetExpression.versus_any(comparison, other)` (experimental): keeps the elements of this multiset that fit
   * the comparison (`"<="`, `"<"`, `">="` or `">"`) against at least one element of `other`. Negative counts in
   * `other` count as zero. Contrast {@link MultisetExpression.versusAll}.
   *
   * @example
   * ```ts
   * pool([1, 3, 5]).versusAny(">", [2, 4]).expand().items(); //=> [[[3, 5], 1n]]
   * ```
   */
  versusAny(comparison: Comparison, other: MultisetArg): MultisetExpression {
    return call("mx_versus", { expr: this, all: false, comparison, other: list(other) }) as MultisetExpression;
  }

  // Evaluation.
  /** `MultisetExpression.sum()`: evaluation: the sum of all elements.
   *
   * @example
   * ```ts
   * d(6).pool(3).sum().equals(d(6).add(d(6)).add(d(6))); //=> true
   * d(6).pool(3).highest(2).sum().mean().toString(); //=> "203/24"
   * ```
   */
  sum(): Die { return mxEvaluate(this, "sum"); }
  /** `MultisetExpression.size()`: evaluation: the number of elements. Mostly useful after another operation:
   * `pool.unique().size()` counts the different outcomes, `pool.and([4, 5, 6]).size()` counts up to one each of 4,
   * 5 and 6.
   *
   * @example
   * ```ts
   * // How many sixes on 3d6?
   * d(6).pool(3).keepOutcomes([6]).size().items(); //=> [[0, 125n], [1, 75n], [2, 15n], [3, 1n]]
   * ```
   */
  size(): Die { return mxEvaluate(this, "size"); }
  /** `MultisetExpression.empty()`: evaluation: whether all counts are zero. The opposite of `any()`.
   *
   * @example
   * ```ts
   * // Does 2d6 roll no 1?
   * d(6).pool(2).keepOutcomes([1]).empty().probability(true).toString(); //=> "25/36"
   * ```
   */
  empty(): Die { return mxEvaluate(this, "empty"); }
  /** `MultisetExpression.any()`: evaluation: whether any count is nonzero. The opposite of `empty()`.
   *
   * @example
   * ```ts
   * // Does 2d6 roll at least one 1?
   * d(6).pool(2).keepOutcomes([1]).any().probability(true).toString(); //=> "11/36"
   * ```
   */
  any(): Die { return mxEvaluate(this, "any"); }
  /** `MultisetExpression.product_of_counts()`: evaluation: the product of the counts.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 3]).productOfCounts().items(); //=> [[4, 1n]]
   * ```
   */
  productOfCounts(): Die { return mxEvaluate(this, "product_of_counts"); }
  /** `MultisetExpression.highest_outcome_and_count()`: evaluation: the highest outcome with a positive count and
   * that count, as `[outcome, count]`. If no count is positive, the lowest outcome with count 0.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 3]).highestOutcomeAndCount().items(); //=> [[[3, 2], 1n]]
   * ```
   */
  highestOutcomeAndCount(): Die { return mxEvaluate(this, "highest_outcome_and_count"); }
  /** `MultisetExpression.largest_count()`: evaluation: the size of the largest matching set (x of a kind). Python's
   * `wild` options are not available here.
   *
   * @example
   * ```ts
   * d(6).pool(3).largestCount().items(); //=> [[1, 120n], [2, 90n], [3, 6n]]
   * ```
   */
  largestCount(): Die { return mxEvaluate(this, "largest_count"); }
  /** `MultisetExpression.largest_count_and_outcome()`: evaluation: the largest matching set and its outcome, as
   * `[count, outcome]` (the higher outcome on a tie).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 5, 5]).largestCountAndOutcome().items(); //=> [[[2, 5], 1n]]
   * ```
   */
  largestCountAndOutcome(): Die { return mxEvaluate(this, "largest_count_and_outcome"); }
  /** `MultisetExpression.largest_straight()`: evaluation: the size of the longest run of consecutive outcomes.
   * Outcomes must be ints.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 5, 6]).largestStraight().items(); //=> [[3, 1n]]
   * d(6).pool(5).largestStraight().probability(5).toString(); //=> "5/162"
   * ```
   */
  largestStraight(): Die { return mxEvaluate(this, "largest_straight"); }
  /** `MultisetExpression.all_straights()`: evaluation: the sizes of all straights, in descending order. Each element
   * belongs to one straight only, though duplicates can make straights that overlap in outcomes; elements go to
   * the longer straight first. Outcomes must be ints.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 5, 6]).allStraights().items(); //=> [[[3, 2], 1n]]
   * ```
   */
  allStraights(): Die { return mxEvaluate(this, "all_straights"); }
  /** `MultisetExpression.largest_straight_and_outcome(priority='high')`: evaluation: the size of the largest
   * straight and its highest (with `"low"`, lowest) outcome, as `[size, outcome]`. Size comes first; `priority`
   * also picks between straights of the same size. Outcomes must be ints.
   *
   * @throws IcepoolError `ValueError` if `priority` is neither `"low"` nor `"high"`.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 5, 6, 7]).largestStraightAndOutcome().items(); //=> [[[3, 7], 1n]]
   * pool([1, 2, 3, 5, 6, 7]).largestStraightAndOutcome("low").items(); //=> [[[3, 1], 1n]]
   * ```
   */
  largestStraightAndOutcome(priority: "high" | "low" = "high"): Die {
    return call("pool_largest_straight_and_outcome", { pool: this, priority }) as Die;
  }
  /** `MultisetExpression.expand(order=Order.Ascending)`: evaluation: all elements as a sorted tuple (a die of
   * arrays), ascending unless `order` is `Order.Descending`. Expensive unless there are few possibilities.
   *
   * @example
   * ```ts
   * pool([3, 1, 2]).expand().items(); //=> [[[1, 2, 3], 1n]]
   * pool([3, 1, 2]).expand(Order.Descending).items(); //=> [[[3, 2, 1], 1n]]
   * d(2).pool(2).expand().items(); //=> [[[1, 1], 1n], [[1, 2], 2n], [[2, 2], 1n]]
   * ```
   */
  expand(order: OrderValue = Order.Ascending): Die {
    return call("pool_expand", { pool: this, order }) as Die;
  }
  /** `MultisetExpression.all_counts(filter=1)`: evaluation: all counts (the sizes of all matching sets) in
   * descending order. Counts below `filter` are left out (`2` gives pairs and better); `"all"` keeps every count,
   * zeros too.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 3, 3]).allCounts().items(); //=> [[[3, 2, 1], 1n]]
   * pool([1, 2, 2, 3, 3, 3]).allCounts(2).items(); //=> [[[3, 2], 1n]]
   * ```
   */
  allCounts(filter: Int | "all" = 1): Die {
    return call("pool_all_counts", { pool: this, filter }) as Die;
  }
  /** `MultisetExpression.count_subset(divisor, *, empty_divisor=None)`: evaluation: how many times `divisor` is
   * contained in this multiset (Python's `self // divisor`).
   *
   * @param emptyDivisor - The outcome when the divisor is empty.
   * @throws IcepoolError `ZeroDivisionError` if the divisor may be empty and `emptyDivisor` is not set.
   *
   * @example
   * ```ts
   * pool([1, 1, 1, 1, 2, 2]).countSubset([1, 1]).items(); //=> [[2, 1n]]
   * pool([1, 2]).countSubset([], { emptyDivisor: 0 }).items(); //=> [[0, 1n]]
   * pool([1, 2]).countSubset([]); //=> throws ZeroDivisionError
   * ```
   */
  countSubset(divisor: MultisetArg, { emptyDivisor = null }: { emptyDivisor?: Int | null } = {}): Die {
    return call("pool_count_subset", { pool: this, divisor: list(divisor), empty_divisor: emptyDivisor }) as Die;
  }
  /** `MultisetExpression.isdisjoint(other)`: evaluation: whether no outcome has a positive count in both multisets.
   * Negative counts count as zero.
   *
   * @example
   * ```ts
   * // Do two d6 rolls share no face?
   * d(6).pool(1).isdisjoint(d(6).pool(1)).probability(true).toString(); //=> "5/6"
   * ```
   */
  isdisjoint(other: MultisetArg): Die { return call("pool_isdisjoint", { pool: this, other: list(other) }) as Die; }
  /** `MultisetExpression.issubset(other)` (Python's `self <= other`): evaluation: whether each count of this
   * multiset is at most the other's. {@link MultisetExpression.lt} is the proper subset.
   *
   * @example
   * ```ts
   * pool([1, 2]).issubset([1, 2, 3]).items(); //=> [[true, 1n]]
   * pool([1, 1]).issubset([1, 2, 3]).items(); //=> [[false, 1n]]
   * ```
   */
  issubset(other: MultisetArg): Die { return call("pool_issubset", { pool: this, other: list(other) }) as Die; }
  /** `MultisetExpression.issuperset(other)` (Python's `self >= other`): evaluation: whether each count of this
   * multiset is at least the other's: e.g. whether a hand holds a combo. {@link MultisetExpression.gt} is the
   * proper superset.
   *
   * @example
   * ```ts
   * // At least two sixes on 5d6.
   * d(6).pool(5).issuperset([6, 6]).probability(true).toString(); //=> "763/3888"
   * ```
   */
  issuperset(other: MultisetArg): Die { return call("pool_issuperset", { pool: this, other: list(other) }) as Die; }
  /** `self < other` (a proper subset), as a die of bools: as `issubset`, but false if the two are equal.
   *
   * @example
   * ```ts
   * pool([1, 2]).lt([1, 2, 3]).items(); //=> [[true, 1n]]
   * pool([1, 2]).lt([1, 2]).items(); //=> [[false, 1n]]
   * ```
   */
  lt(other: MultisetArg): Die { return poolCompare(this, other, "proper_subset"); }
  /** `self > other` (a proper superset), as a die of bools: as `issuperset`, but false if the two are equal.
   *
   * @example
   * ```ts
   * pool([1, 2, 3]).gt([1, 2]).items(); //=> [[true, 1n]]
   * ```
   */
  gt(other: MultisetArg): Die { return poolCompare(this, other, "proper_superset"); }
  /** `self == other` as multisets (a die of bools): whether every count is the same.
   *
   * @example
   * ```ts
   * pool([1, 2]).equalSet([2, 1]).items(); //=> [[true, 1n]]
   * // Two 2d6 rolls showing the same faces.
   * d(6).pool(2).equalSet(d(6).pool(2)).probability(true).toString(); //=> "11/216"
   * ```
   */
  equalSet(other: MultisetArg): Die { return poolCompare(this, other, "equal_set"); }
  /** `self != other` as multisets (a die of bools): whether any count differs.
   *
   * @example
   * ```ts
   * pool([1, 2]).notEqualSet([1, 3]).items(); //=> [[true, 1n]]
   * ```
   */
  notEqualSet(other: MultisetArg): Die { return poolCompare(this, other, "not_equal_set"); }

  /** `MultisetExpression.map_counts(...args, { function })`: maps the counts to new counts: `function` takes
   * `(outcome, ...counts)`, one count per argument, and returns the combined count.
   *
   * @example
   * ```ts
   * const product = (o: number, a: number, b: number) => a * b;
   * MultisetExpression.mapCounts([1, 2, 2], [2, 3], { function: product }).expand().items(); //=> [[[2, 2], 1n]]
   * // Double the sixes.
   * const doubleSixes = (o: number, n: number) => (o === 6 ? 2 * n : n);
   * MultisetExpression.mapCounts(pool([1, 6]), { function: doubleSixes }).sum().items(); //=> [[13, 1n]]
   * ```
   */
  static mapCounts(...rest: (MultisetArg | { function: (outcome: never, ...counts: never[]) => Int })[]): Expression {
    const [args, { function: f }] = opts<{ function: (outcome: never, ...counts: never[]) => Int }>(rest);
    return call("mx_map_counts", { args: List.from(args.map(list)), function: f }) as Expression;
  }
}

function mxMethod(name: string, args: unknown[]): Expression {
  return call("mx_method", { name, args: List.from(args.map(list)), keep_negative_counts: false }) as Expression;
}
function mxCounts(expr: MultisetExpression, o: string, comparison: Comparison | null, n: Int | null): MultisetExpression {
  // `__mul__` and `__mod__` return `NotImplemented` for anything but an int, so an Again expression's reflected
  // operator makes an expression of the two.
  if ((n as unknown) instanceof AgainExpression && (o === "mul" || o === "mod")) {
    const again = n as unknown as AgainExpression;
    const operand = expr as unknown as Operand;
    return (o === "mul" ? again.rmul(operand) : again.rmod(operand)) as unknown as MultisetExpression;
  }
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

/** A `KeepGenerator`: a pool or a deal, whose sorted rolls can be kept or dropped (keeping gives one of the same
 * kind), and which has `middle`. It always produces the same number of elements, and keeping works whatever order
 * the outcomes are evaluated in. Keeping applies only while the keep tuple has no negative counts; otherwise it
 * falls back to the {@link MultisetExpression} versions.
 *
 * @example
 * ```ts
 * const kept = d(6).pool(4).highest(3); // still a pool
 * kept.keepTuple(); //=> [0, 1, 1, 1]
 * kept.lowest(1).sum().equals(d(6).pool(4).at(1)); //=> true
 * ```
 */
export class KeepGenerator extends MultisetExpression {
  /** `KeepGenerator.highest(keep=None, drop=None)`: as {@link MultisetExpression.highest}, but the result is
   * again a pool or a deal (with a new keep tuple), whatever the evaluation order.
   *
   * @example
   * ```ts
   * d(6).pool(3).highest(2).keepTuple(); //=> [0, 1, 1]
   * d(6).pool(3).highest(2).sum().equals(d(6).highest(3, 2)); //=> true
   * ```
   */
  override highest(keep: Int | null = null, drop: Int | null = null): this {
    return super.highest(keep, drop) as this;
  }
  /** `KeepGenerator.lowest(keep=None, drop=None)`: as {@link MultisetExpression.lowest}, but the result is again a
   * pool or a deal.
   *
   * @example
   * ```ts
   * d(6).pool(4).lowest(2, 1).keepTuple(); //=> [0, 1, 1, 0]
   * ```
   */
  override lowest(keep: Int | null = null, drop: Int | null = null): this {
    return super.lowest(keep, drop) as this;
  }
  /** `KeepGenerator.middle(keep=1, *, tie='error')`: keeps the middle `keep` elements and drops the rest (without
   * summing them). If `keep` is greater than the current keep size, all are kept.
   *
   * @param tie - When `keep` and the current keep size differ in parity: `"error"` (default) raises, `"low"` or
   *   `"high"` takes the lower or higher of the two possible middles.
   * @throws IcepoolError `IndexError` if the middle is ambiguous and `tie` is `"error"`; `ValueError` if `keep` is
   *   negative.
   *
   * @example
   * ```ts
   * d(6).pool(5).middle(3).keepTuple(); //=> [0, 1, 1, 1, 0]
   * d(6).pool(4).middle(1, { tie: "high" }).keepTuple(); //=> [0, 0, 1, 0]
   * d(6).pool(4).middle(1); //=> throws IndexError
   * ```
   */
  middle(keep: Int = 1, { tie = "error" }: { tie?: "error" | "high" | "low" } = {}): this {
    return call("pool_middle", { pool: this, keep, tie }) as this;
  }
  /** `KeepGenerator.keep(index)` (Python's `g[index]`): counts the elements in sorted (ascending) order, whatever
   * order the outcomes are evaluated in. `index` is either
   *
   * - an int: only the element at that position counts, and the result is a die rather than a pool;
   * - an array with one count per element: each is counted that many times (possibly more than once, or negatively).
   *   Up to one `Ellipsis` stands for as many zeros as needed: `[1, Ellipsis, 1]` on five dice is
   *   `[1, 0, 0, 0, 1]`. If the array is longer than the keep size, `Ellipsis` at one end drops elements of the
   *   array from that end (`[Ellipsis, 1, 2, 3]` on two dice is `[2, 3]`), and in the middle it acts as the sum of
   *   the two one-sided versions.
   *
   * Applied again, the selection is relative to the previous keep tuple: `slice(null, 2).at(-1)` is the
   * second-lowest roll. For selecting two of five, `slice(3, 5)`, `slice(-2)`, `keep([Ellipsis, 1, 1])` and
   * `keep([0, 0, 0, 1, 1])` are all the same; `keep([-1, Ellipsis, 1])` counts the highest positively and the
   * lowest negatively.
   *
   * @throws IcepoolError `IndexError` if more than one `Ellipsis` is used, or a fixed-length `index` differs from
   *   the keep size.
   *
   * @example
   * ```ts
   * d(6).pool(5).keep([1, Ellipsis, 1]).keepTuple(); //=> [1, 0, 0, 0, 1]
   * // Highest minus lowest of 3d6.
   * d(6).pool(3).keep([-1, Ellipsis, 1]).sum().mean().toString(); //=> "35/12"
   * d(6).pool(3).keep(0).mean().toString(); //=> "49/24"
   * ```
   */
  override keep(index: readonly (Int | typeof Ellipsis)[]): this;
  override keep(index: Int): Die;
  override keep(index: Int | readonly (Int | typeof Ellipsis)[]): this | Die {
    return call("pool_keep", { pool: this, index }) as this | Die;
  }
  /** `g[start:stop]`: the sorted elements in that slice counted once each, as a pool or a deal. A `step` raises
   * `IndexError`.
   *
   * @example
   * ```ts
   * d(6).pool(5).slice(-2).keepTuple(); //=> [0, 0, 0, 1, 1]
   * d(6).pool(5).slice(1, 3).keepTuple(); //=> [0, 1, 1, 0, 0]
   * ```
   */
  override slice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): this {
    return super.slice(start, stop, step) as this;
  }
}

/** What `new Pool(...)` takes: an array of dice or outcomes (each an element; `times` repeats them), or a Map of
 * dice or outcomes to how many of each to put in the pool.
 *
 * @example
 * ```ts
 * import { d, Pool, type PoolDice } from "icepool-wasm";
 *
 * const dice: PoolDice = new Map([[d(6), 3]]);
 * new Pool(dice).sum().equals(d(6).pool(3).sum()); //=> true
 * ```
 */
export type PoolDice = readonly Operand[] | Map<Operand, Int>;

/** `icepool.Pool`: a multiset of outcomes from rolling several dice: `new Pool([d6, d6, d8])`,
 * `new Pool([d6, d8], [2, 1])`, `new Pool(new Map([[d6, 3]]))`. Evaluate it with its methods (`sum`, `size`, ...),
 * a {@link MultisetEvaluator} or a {@link multisetFunction}. `die.pool(n)` and {@link pool} make one too.
 *
 * Evaluation is most efficient when the dice are the same or same-side truncations of each other (d4, d6, d8,
 * d10, d12 are all truncations of d12). A pool may be empty, but not every evaluator handles that. A die may be in
 * the pool zero times: its outcomes are then considered without any count. All outcomes in a pool must be totally
 * orderable.
 *
 * @example
 * ```ts
 * const p = new Pool([d(6), d(6), d(8)]);
 * p.outcomes(); //=> [1, 2, 3, 4, 5, 6, 7, 8]
 * p.highest(2).sum().mean().toString(); //=> "2689/288"
 * new Pool([d(6), d(8)], [2, 1]).rawSize(); //=> 3
 * ```
 */
export class Pool extends KeepGenerator {
  /** `Pool(dice, times=1)`. `times` multiplies how many times each element of `dice` goes into the pool: one int for
   * all, or one per element. Throws `ValueError` for a bare `Die` or `Deck` (a pool of one die is `new Pool([die])`).
   */
  constructor(dice: PoolDice, times: Int | readonly Int[] = 1) {
    super(HANDLE, -1);
    const made =
      dice instanceof Map
        ? call("pool_dict", { items: List.from(Array.from(dice, ([k, v]) => [k, v])), times: list(times) })
        : call("pool", { dice: list(dice), times: list(times) });
    this.adopt(made, Pool);
  }
  /** `(keep_tuple(), keep_size(), raw_size(), denominator(), outcomes(), _dice)`: the pool's description in one call
   * (`_dice` being `[die, count]` pairs).
   *
   * @example
   * ```ts
   * const [keepTuple, keepSize, rawSize, denominator, outcomes] = d(6).pool(3).highest(2).info();
   * [keepTuple, keepSize, rawSize, denominator, outcomes]; //=> [[0, 1, 1], 2, 3, 216, [1, 2, 3, 4, 5, 6]]
   * ```
   */
  info(): [Int[], Int, Int, Int, Outcome[], [Die, Int][]] {
    return call("pool_info", { pool: this }) as [Int[], Int, Int, Int, Outcome[], [Die, Int][]];
  }
  /** `keep_tuple()`: how many times each sorted (ascending) roll is counted.
   *
   * @example
   * ```ts
   * d(6).pool(4).highest(3).keepTuple(); //=> [0, 1, 1, 1]
   * ```
   */
  keepTuple(): Int[] { return this.info()[0]; }
  /** `keep_size()`: the total count produced: the sum of the keep tuple.
   *
   * @example
   * ```ts
   * d(6).pool(4).highest(3).keepSize(); //=> 3
   * ```
   */
  keepSize(): Int { return this.info()[1]; }
  /** `raw_size()`: the number of dice in the pool before the keep tuple is applied.
   *
   * @example
   * ```ts
   * d(6).pool(4).highest(3).rawSize(); //=> 4
   * ```
   */
  rawSize(): Int { return this.info()[2]; }
  /** `denominator()`: the product of the dice's denominators, one per die.
   *
   * @example
   * ```ts
   * new Pool([d(6), d(8)]).denominator(); //=> 48n
   * ```
   */
  denominator(): bigint { return BigInt(this.info()[3]); }
  /** `outcomes()`: the union of the outcomes of all the dice in the pool, in ascending order.
   *
   * @example
   * ```ts
   * new Pool([d(4), z(3)]).outcomes(); //=> [0, 1, 2, 3, 4]
   * ```
   */
  outcomes(): Outcome[] { return this.info()[4]; }
}

/** `icepool.Pool(dice, times)` without `new`.
 *
 * @example
 * ```ts
 * pool([d(6), d(6)]).sum().equals(d(6).add(d(6))); //=> true
 * pool([1, 2, 2, 3]).unique().expand().items(); //=> [[[1, 2, 3], 1n]]
 * ```
 */
export function pool(dice: PoolDice, times: Int | readonly Int[] = 1): Pool {
  return new Pool(dice, times);
}
/** `icepool.d_pool(die_sizes)`: a pool of standard dice (d6, d8, ...): one d(size) per size, or a Map of size to
 * how many dice of that size. Empty, it is a pool of zero zeros.
 *
 * @example
 * ```ts
 * dPool([4, 6]).sum().equals(d(4).add(d(6))); //=> true
 * dPool(new Map([[6, 3]])).sum().mean().toString(); //=> "21/2"
 * ```
 */
export function dPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return standardPool("d_pool", dieSizes);
}
/** `icepool.z_pool(die_sizes)`: a pool of zero-indexed dice (z6 = 0 to 5, ...): one z(size) per size, or a Map of
 * size to how many dice of that size. Empty, it is a pool of zero zeros.
 *
 * @example
 * ```ts
 * zPool([6, 6]).sum().mean().toString(); //=> "5"
 * ```
 */
export function zPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return standardPool("z_pool", dieSizes);
}
function standardPool(op: string, dieSizes: readonly Int[] | Map<Int, Int>): Pool {
  return dieSizes instanceof Map
    ? (call(op, { die_sizes: List.from(Array.from(dieSizes, ([k, v]) => [k, v])), mapping: true }) as Pool)
    : (call(op, { die_sizes: list(dieSizes), mapping: false }) as Pool);
}

/** `icepool.MultisetMixture(inners)`: a mixture of multiset expressions: one of them, each equally likely (an
 * expression listed twice has twice the weight).
 *
 * @example
 * ```ts
 * // Roll 1d6 or 2d6, equally likely.
 * multisetMixture([d(6).pool(1), d(6).pool(2)]).sum().mean().toString(); //=> "21/4"
 * multisetMixture([pool([1]), pool([2, 2])]).sum().items(); //=> [[1, 1n], [4, 1n]]
 * ```
 */
export function multisetMixture(inners: readonly MultisetExpression[]): Expression {
  return call("mx_mixture", { inners: list(inners) }) as Expression;
}

/** Any other multiset expression (an operator over pools, a CompoundKeepGenerator...): what most
 * {@link MultisetExpression} methods return.
 *
 * @example
 * ```ts
 * const e = d(6).pool(3).union([6, 6]);
 * e instanceof Expression; //=> true
 * e.size().mean().toString(); //=> "973/216"
 * ```
 */
export class Expression extends MultisetExpression {}

registerKind("pool", (h) => wrapHandle(Pool.prototype, h));
registerKind("expr", (h) => new Expression(HANDLE, h));
