// Pools and multiset expressions (generator/pool.py, generator/keep.py, expression/multiset_expression.py): the
// expression tree is the TypeScript's own (see "The native expression tree" below), over keep generators of the
// core (src/api/multiset.rs), turned into the core's tree when evaluated (src/api/evaluate.rs).

import type { ExpressionBuilder, WasmDie, WasmGenerator } from "../pkg-node/icepool_wasm.js";
import { AgainExpression } from "./again.js";
import { Die, d as dDie, dictItems, itemize, list, opts, pyBool, z as zDie, type Comparison, type DieOutcomes, type Operand } from "./die.js";
import { IcepoolError } from "./errors.js";
import { HANDLE, Handle, call, registerKind, wrapHandle } from "./rpc.js";
import { Complex, Ellipsis, Float, Fraction, List, Order, Symbols, Vector, isPlainValue, pyFloatRepr, pyTypeName, type Int, type OrderValue, type Outcome } from "./values.js";
import { HOST, callback, isTrap, wasm, withCallbacks } from "./wasm.js";

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
  /** The native form of this expression; `undefined` for one only the bridge holds. @internal */
  declare node?: Node;
  /** The bridge call this native expression stands for, made if the bridge needs it. */
  private declare replay?: () => unknown;

  /** A native expression of this class. @internal */
  static native<T extends MultisetExpression>(this: abstract new (...args: never[]) => T, node: Node): T {
    const x = Object.create(this.prototype as object) as T;
    x.handle = -1;
    x.node = node;
    return x;
  }

  /** Remembers the bridge call that makes this native expression, unless it already has one. @internal */
  replayWith(bridge: () => unknown): void {
    if (this.handle < 0 && this.replay === undefined && this.node !== undefined) this.replay = bridge;
  }

  /** The bridge's handle, made by replaying the bridge call the first time the bridge needs it. @internal */
  protected override get h(): number {
    if (this.handle < 0 && this.replay !== undefined) {
      const replay = this.replay;
      this.replay = undefined;
      this.adopt(replay(), Handle);
    }
    if (this.handle < 0) throw new TypeError("icepool: this object was freed");
    return this.handle;
  }

  /** Frees the expression's WASM memory now rather than when it is garbage-collected. @internal */
  override free(): void {
    this.node = undefined;
    this.replay = undefined;
    super.free();
  }

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
    return mxMethod(this, "additive_union", "MultisetAdditiveUnion", others);
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
    const name = pyBool(keepNegativeCounts) ? "MultisetDifferenceKeepNegative" : "MultisetDifferenceDropNegative";
    return mxMethod(this, "difference", name, others, keepNegativeCounts);
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
    return mxMethod(this, "intersection", "MultisetIntersection", others);
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
    return mxMethod(this, "union", "MultisetUnion", others);
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
    return mxBinary(this, "^", other) as Expression;
  }
  /** Python's `self + other`: {@link MultisetExpression.additiveUnion} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).add([3, 4]).expand().items(); //=> [[[1, 2, 2, 3, 3, 4], 1n]]
   * ```
   */
  add(other: MultisetArg): MultisetExpression {
    return mxBinary(this, "+", other);
  }
  /** Python's `self - other`: {@link MultisetExpression.difference} of two multisets, negative counts set to zero.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).sub([2, 3]).expand().items(); //=> [[[1, 2], 1n]]
   * ```
   */
  sub(other: MultisetArg): MultisetExpression {
    return mxBinary(this, "-", other);
  }
  /** Python's `self & other`: {@link MultisetExpression.intersection} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).and([2, 2, 3, 3]).expand().items(); //=> [[[2, 2, 3], 1n]]
   * ```
   */
  and(other: MultisetArg): MultisetExpression {
    return mxBinary(this, "&", other);
  }
  /** Python's `self | other`: {@link MultisetExpression.union} of two multisets.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).or([2, 2, 3, 3]).expand().items(); //=> [[[1, 2, 2, 3, 3], 1n]]
   * ```
   */
  or(other: MultisetArg): MultisetExpression {
    return mxBinary(this, "|", other);
  }
  /** Python's `self ^ other`: the same as {@link MultisetExpression.symmetricDifference}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).xor([2, 3, 3]).expand().items(); //=> [[[1, 2, 3], 1n]]
   * ```
   */
  xor(other: MultisetArg): MultisetExpression {
    return mxBinary(this, "^", other);
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
    return native(this, () => highestLowest(this, true, keep, drop), () => call("pool_highest", { pool: this, keep, drop }));
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
    return native(this, () => highestLowest(this, false, keep, drop), () => call("pool_lowest", { pool: this, keep, drop }));
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
    return native(this, () => keepIndex(this, index), () => call("pool_keep", { pool: this, index }));
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
    return native(this, () => keepIndex(this, index) as Die | undefined, () => call("pool_index", { pool: this, index }));
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
    return native(this, () => keepSliceIndex(this, start, stop, step), () => call("pool_slice", { pool: this, start, stop, step }));
  }

  // Counts.
  /** Python's `self * n`: the same as {@link MultisetExpression.multiplyCounts}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).mul(2).expand().items(); //=> [[[1, 1, 2, 2, 2, 2, 3, 3], 1n]]
   * ```
   */
  mul(n: Int): MultisetExpression {
    return mxCounts(this, "mul", null, n, () => (isIntArg(n) ? multiplyCounts(this, intOf(n)) : notInt(this, "mul", n)));
  }
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
  neg(): MultisetExpression {
    return mxCounts(this, "neg", null, null, () => multiplyCounts(this, -1));
  }
  /** Python's `+self`: sets all negative counts to zero (the same as `keepCounts(">=", 0)`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).difference([2, 2, 2], { keepNegativeCounts: true }).pos().expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  pos(): MultisetExpression {
    return mxCounts(this, "pos", null, null, () => positive(this));
  }
  /** Python's `self // n` with an int: the same as {@link MultisetExpression.divideCounts}. (`//` with a multiset is
   * {@link MultisetExpression.countSubset}.)
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).floordiv(2).expand().items(); //=> [[[2], 1n]]
   * ```
   */
  floordiv(n: Int): MultisetExpression {
    // With anything but an int, `count_subset`.
    const divide = () => (isIntArg(n) ? floordivCounts(this, intOf(n)) : evaluationOf("count_subset", [null], this, n));
    return mxCounts(this, "floordiv", null, n, divide);
  }
  /** Python's `self % n`: the same as {@link MultisetExpression.moduloCounts}.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).mod(2).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  mod(n: Int): MultisetExpression {
    return mxCounts(this, "mod", null, n, () => (isIntArg(n) ? moduloCounts(this, intOf(n)) : notInt(this, "mod", n)));
  }
  /** `MultisetExpression.unique(n=1)`: counts each outcome at most `n` times.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).unique().expand().items(); //=> [[[1, 2, 3], 1n]]
   * // How many different faces show on 3d6?
   * d(6).pool(3).unique().size().items(); //=> [[1, 6n], [2, 90n], [3, 120n]]
   * ```
   */
  unique(n: Int = 1): MultisetExpression {
    const c = intConstant(n);
    return mxCounts(this, "unique", null, n, () => (c === undefined ? undefined : unique(this, c)));
  }
  /** `MultisetExpression.keep_counts(comparison, n)`: keeps the counts that fit the comparison with `n`, treating
   * the rest as zero. `keepCounts(">=", 2)` keeps pairs, triplets, etc. and drops singles.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 3, 3]).keepCounts(">=", 2).expand().items(); //=> [[[2, 2, 3, 3, 3], 1n]]
   * ```
   */
  keepCounts(comparison: Comparison, n: Int): MultisetExpression {
    return mxCounts(this, "keep_counts", comparison, n, () => keepCounts(this, comparison, n));
  }
  /** `MultisetExpression.multiply_counts(n)`: multiplies all counts by `n` (Python's `self * n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).multiplyCounts(2).expand().items(); //=> [[[1, 1, 2, 2, 2, 2, 3, 3], 1n]]
   * ```
   */
  multiplyCounts(n: Int): MultisetExpression {
    const c = constant(n);
    return mxCounts(this, "multiply_counts", null, n, () => (c === undefined ? undefined : multiplyCounts(this, c)));
  }
  /** `MultisetExpression.divide_counts(n)`: divides all counts by `n`, rounding down (Python's `self // n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).divideCounts(2).expand().items(); //=> [[[2], 1n]]
   * ```
   */
  divideCounts(n: Int): MultisetExpression {
    const c = intConstant(n);
    return mxCounts(this, "divide_counts", null, n, () => (c === undefined ? undefined : floordivCounts(this, c)));
  }
  /** `MultisetExpression.modulo_counts(n)`: each count modulo `n` (Python's `self % n`).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3]).moduloCounts(2).expand().items(); //=> [[[1, 3], 1n]]
   * ```
   */
  moduloCounts(n: Int): MultisetExpression {
    return mxCounts(this, "modulo_counts", null, n, () => (isIntArg(n) ? moduloCounts(this, intOf(n)) : notInt(this, "mod", n)));
  }

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
    return native(this, () => filterOutcomes(this, false, target), () => call("mx_filter", { expr: this, invert: false, target: list(target) }));
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
    return native(this, () => filterOutcomes(this, true, target), () => call("mx_filter", { expr: this, invert: true, target: list(target) }));
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
    return native(this, () => forceOrder(this, order), () => call("mx_force_order", { expr: this, order }));
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
    return native(this, () => weightless(this), () => call("weightless", { generator: this }));
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
    const bridge = () => call("mx_max_pair", { expr: this, keep: true, comparison, other: list(other), priority });
    return native(this, () => maxPair(this, true, comparison, other, priority), bridge);
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
    const bridge = () => call("mx_max_pair", { expr: this, keep: false, comparison, other: list(other), priority });
    return native(this, () => maxPair(this, false, comparison, other, priority), bridge);
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
    const bridge = () => call("mx_versus", { expr: this, all: true, comparison, other: list(other) });
    return native(this, () => versus(this, true, comparison, other), bridge);
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
    const bridge = () => call("mx_versus", { expr: this, all: false, comparison, other: list(other) });
    return native(this, () => versus(this, false, comparison, other), bridge);
  }

  /** Python's `str(expression)`: a pool lists its dice, an operator shows its operands.
   *
   * @example
   * ```ts
   * pool([1]).toString(); //=> "Pool of 1 dice with keep_tuple=(1,)\n  Die({1: 1}) : 1,\n"
   * pool([1]).union([2]).toString().split("\n")[0]; //=> "(Pool of 1 dice with keep_tuple=(1,)"
   * ```
   */
  override toString(): string {
    const native = isNative(this) ? expressionStr(this) : undefined;
    return native ?? (call("mx_str", { expr: this }) as string);
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
  sum(): Die {
    return evaluation("sum", [], [this]) ?? mxEvaluate(this, "sum");
  }
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
  size(): Die {
    return evaluation("size", [], [this]) ?? mxEvaluate(this, "size");
  }
  /** `MultisetExpression.empty()`: evaluation: whether all counts are zero. The opposite of `any()`.
   *
   * @example
   * ```ts
   * // Does 2d6 roll no 1?
   * d(6).pool(2).keepOutcomes([1]).empty().probability(true).toString(); //=> "25/36"
   * ```
   */
  empty(): Die {
    return evaluation("empty", [false], [this]) ?? mxEvaluate(this, "empty");
  }
  /** `MultisetExpression.any()`: evaluation: whether any count is nonzero. The opposite of `empty()`.
   *
   * @example
   * ```ts
   * // Does 2d6 roll at least one 1?
   * d(6).pool(2).keepOutcomes([1]).any().probability(true).toString(); //=> "11/36"
   * ```
   */
  any(): Die {
    return evaluation("empty", [true], [this]) ?? mxEvaluate(this, "any");
  }
  /** `MultisetExpression.product_of_counts()`: evaluation: the product of the counts.
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 3, 3]).productOfCounts().items(); //=> [[4, 1n]]
   * ```
   */
  productOfCounts(): Die {
    return evaluation("product_of_counts", [], [this]) ?? mxEvaluate(this, "product_of_counts");
  }
  /** `MultisetExpression.highest_outcome_and_count()`: evaluation: the highest outcome with a positive count and
   * that count, as `[outcome, count]`. If no count is positive, the lowest outcome with count 0.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 3]).highestOutcomeAndCount().items(); //=> [[[3, 2], 1n]]
   * ```
   */
  highestOutcomeAndCount(): Die {
    return evaluation("highest_outcome_and_count", [], [this]) ?? mxEvaluate(this, "highest_outcome_and_count");
  }
  /** `MultisetExpression.largest_count()`: evaluation: the size of the largest matching set (x of a kind). Python's
   * `wild` options are not available here.
   *
   * @example
   * ```ts
   * d(6).pool(3).largestCount().items(); //=> [[1, 120n], [2, 90n], [3, 6n]]
   * ```
   */
  largestCount(): Die {
    return evaluation("largest_count", [], [this]) ?? mxEvaluate(this, "largest_count");
  }
  /** `MultisetExpression.largest_count_and_outcome()`: evaluation: the largest matching set and its outcome, as
   * `[count, outcome]` (the higher outcome on a tie).
   *
   * @example
   * ```ts
   * pool([1, 2, 2, 5, 5]).largestCountAndOutcome().items(); //=> [[[2, 5], 1n]]
   * ```
   */
  largestCountAndOutcome(): Die {
    return evaluation("largest_count_and_outcome", [], [this]) ?? mxEvaluate(this, "largest_count_and_outcome");
  }
  /** `MultisetExpression.largest_straight()`: evaluation: the size of the longest run of consecutive outcomes.
   * Outcomes must be ints.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 5, 6]).largestStraight().items(); //=> [[3, 1n]]
   * d(6).pool(5).largestStraight().probability(5).toString(); //=> "5/162"
   * ```
   */
  largestStraight(): Die {
    return evaluation("largest_straight", [], [this]) ?? mxEvaluate(this, "largest_straight");
  }
  /** `MultisetExpression.all_straights()`: evaluation: the sizes of all straights, in descending order. Each element
   * belongs to one straight only, though duplicates can make straights that overlap in outcomes; elements go to
   * the longer straight first. Outcomes must be ints.
   *
   * @example
   * ```ts
   * pool([1, 2, 3, 5, 6]).allStraights().items(); //=> [[[3, 2], 1n]]
   * ```
   */
  allStraights(): Die {
    return evaluation("all_straights", [], [this]) ?? mxEvaluate(this, "all_straights");
  }
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
    // The evaluator's constructor checks the priority.
    if (isNative(this) && priority !== "high" && priority !== "low") throw valueError("priority must be 'low' or 'high'.");
    const native = evaluation("largest_straight_and_outcome", [priority === "high"], [this]);
    return native ?? (call("pool_largest_straight_and_outcome", { pool: this, priority }) as Die);
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
    // `order != Order.Ascending`, for an `IntEnum`.
    const native = isIntArg(order) ? evaluation("expand", [Number(intOf(order)) === 1], [this]) : undefined;
    return native ?? (call("pool_expand", { pool: this, order }) as Die);
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
    const native = isPlainValue(filter) ? evaluation("all_counts", [filter], [this]) : undefined;
    return native ?? (call("pool_all_counts", { pool: this, filter }) as Die);
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
    const native = isPlainValue(emptyDivisor) && isNative(this) ? evaluationOf("count_subset", [emptyDivisor], this, divisor) : undefined;
    return native ?? (call("pool_count_subset", { pool: this, divisor: list(divisor), empty_divisor: emptyDivisor }) as Die);
  }
  /** Python's `other // self` (`__rfloordiv__`): `other.countSubset(self)`, an array of outcomes being a pool.
   *
   * @example
   * ```ts
   * pool([1, 1]).rfloordiv([1, 1, 1, 1, 2, 2]).items(); //=> [[2, 1n]]
   * ```
   */
  rfloordiv(other: MultisetArg): Die {
    const make = () => {
      // `Die.__floordiv__` converts the expression into a one-outcome die, which cannot sort it.
      if (other instanceof Die) throw typeError("Items do not appear to be sortable.");
      const left = implicit(other);
      return left === undefined ? undefined : evaluation("count_subset", [null], [left, this]);
    };
    return native(this, make, () => call("mx_rfloordiv", { other: list(other), pool: this }));
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
  isdisjoint(other: MultisetArg): Die {
    return compare(this, other, "is_disjoint_set") ?? (call("pool_isdisjoint", { pool: this, other: list(other) }) as Die);
  }
  /** `MultisetExpression.issubset(other)` (Python's `self <= other`): evaluation: whether each count of this
   * multiset is at most the other's. {@link MultisetExpression.lt} is the proper subset.
   *
   * @example
   * ```ts
   * pool([1, 2]).issubset([1, 2, 3]).items(); //=> [[true, 1n]]
   * pool([1, 1]).issubset([1, 2, 3]).items(); //=> [[false, 1n]]
   * ```
   */
  issubset(other: MultisetArg): Die {
    return compare(this, other, "is_subset") ?? (call("pool_issubset", { pool: this, other: list(other) }) as Die);
  }
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
  issuperset(other: MultisetArg): Die {
    return compare(this, other, "is_superset") ?? (call("pool_issuperset", { pool: this, other: list(other) }) as Die);
  }
  /** `self < other` (a proper subset), as a die of bools: as `issubset`, but false if the two are equal.
   *
   * @example
   * ```ts
   * pool([1, 2]).lt([1, 2, 3]).items(); //=> [[true, 1n]]
   * pool([1, 2]).lt([1, 2]).items(); //=> [[false, 1n]]
   * ```
   */
  lt(other: MultisetArg): Die {
    return compare(this, other, "is_proper_subset") ?? poolCompare(this, other, "proper_subset");
  }
  /** `self > other` (a proper superset), as a die of bools: as `issuperset`, but false if the two are equal.
   *
   * @example
   * ```ts
   * pool([1, 2, 3]).gt([1, 2]).items(); //=> [[true, 1n]]
   * ```
   */
  gt(other: MultisetArg): Die {
    return compare(this, other, "is_proper_superset") ?? poolCompare(this, other, "proper_superset");
  }
  /** `self == other` as multisets (a die of bools): whether every count is the same.
   *
   * @example
   * ```ts
   * pool([1, 2]).equalSet([2, 1]).items(); //=> [[true, 1n]]
   * // Two 2d6 rolls showing the same faces.
   * d(6).pool(2).equalSet(d(6).pool(2)).probability(true).toString(); //=> "11/216"
   * ```
   */
  equalSet(other: MultisetArg): Die {
    return compare(this, other, "is_equal_set") ?? poolCompare(this, other, "equal_set");
  }
  /** `self != other` as multisets (a die of bools): whether any count differs.
   *
   * @example
   * ```ts
   * pool([1, 2]).notEqualSet([1, 3]).items(); //=> [[true, 1n]]
   * ```
   */
  notEqualSet(other: MultisetArg): Die {
    return compare(this, other, "is_not_equal_set") ?? poolCompare(this, other, "not_equal_set");
  }

  // Sorting and lexicographic comparisons.
  /** `MultisetExpression.argsort(*args, order=Order.Descending, limit=None, limit_pad=None, tie)` (experimental):
   * evaluation: which of this multiset and `args` each sorted element comes from, as a tuple of indexes (0 for
   * this one), from the first element in `order` on. `tie` (required) says what equal elements of different
   * multisets do: `"drop"` (none of them count), `"left"` or `"right"` (the leftmost or rightmost index first).
   * `limit` keeps at most that many indexes; `limitPad` pads short results to `limit` with it.
   *
   * @param rest - The other multisets, then the options `{ tie, order, limit, limitPad }`.
   *
   * @example
   * ```ts
   * pool([1, 3]).argsort([2], { tie: "drop" }).items(); //=> [[[0, 1, 0], 1n]]
   * pool([1, 3]).argsort([2], [3], { tie: "left" }).items(); //=> [[[0, 2, 1, 0], 1n]]
   * ```
   */
  argsort(...rest: (MultisetArg | { order?: OrderValue; limit?: Int | null; limitPad?: Int | null; tie: "drop" | "left" | "right" })[]): Die {
    type Options = { order: OrderValue; limit: Int | null; limitPad: Int | null; tie: string };
    const [others, { order = Order.Descending, limit = null, limitPad = null, tie }] = opts<Options>(rest);
    const make = () => {
      const params = [tie, order, limit, limitPad];
      return params.every(isPlainValue) ? evaluationOf("argsort", params, this, ...others) : undefined;
    };
    const args = List.of(this, ...others.map(list));
    return make() ?? (call("argsort", { args, order, limit, limit_pad: limitPad, tie }) as Die);
  }
  /** `MultisetExpression.argsort_grouped(*args, order=Order.Descending, limit=None)` (experimental): evaluation: as
   * {@link MultisetExpression.argsort}, but equal elements of different multisets form one group, a tuple of their
   * indexes.
   *
   * @param rest - The other multisets, then the options `{ order, limit }`.
   *
   * @example
   * ```ts
   * pool([1, 3]).argsortGrouped([3], [2]).items(); //=> [[[[0, 1], [2], [0]], 1n]]
   * ```
   */
  argsortGrouped(...rest: (MultisetArg | { order?: OrderValue; limit?: Int | null })[]): Die {
    const [others, { order = Order.Descending, limit = null }] = opts<{ order: OrderValue; limit: Int | null }>(rest);
    const make = () => ([order, limit].every(isPlainValue) ? evaluationOf("argsort_grouped", [order, limit], this, ...others) : undefined);
    return make() ?? (call("argsort_grouped", { args: List.of(this, ...others.map(list)), order, limit }) as Die);
  }
  /** `MultisetExpression.leximin(comparison, other, extra='high')`: evaluation: compares the two multisets
   * lexicographically after sorting each in ascending order: 1 if the comparison holds, else 0 (with `"cmp"`: -1,
   * 0 or 1). `extra` says what an element without a counterpart counts as: `"low"`, `"high"`, or
   * `"drop"` (left out).
   *
   * @example
   * ```ts
   * pool([1, 3]).leximin("<", [1, 4]).items(); //=> [[1, 1n]]
   * ```
   */
  leximin(comparison: Comparison | "cmp", other: MultisetArg, extra: "low" | "high" | "drop" = "high"): Die {
    return lexi(this, false, comparison, other, extra);
  }
  /** `MultisetExpression.leximax(comparison, other, extra='high')`: as {@link MultisetExpression.leximin}, sorting
   * each in descending order.
   *
   * @example
   * ```ts
   * pool([1, 3]).leximax("cmp", [1, 4]).items(); //=> [[-1, 1n]]
   * ```
   */
  leximax(comparison: Comparison | "cmp", other: MultisetArg, extra: "low" | "high" | "drop" = "high"): Die {
    return lexi(this, true, comparison, other, extra);
  }

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
    return native(null, () => mapCounts(args, f), () => call("mx_map_counts", { args: List.from(args.map(list)), function: f }));
  }
}

/** `MultisetExpression.<name>(self, *others)` called on the class (`keep_negative_counts` for `difference`). */
function mxMethod(self: MultisetExpression, name: string, operatorName: string, others: unknown[], keepNegativeCounts = false): Expression {
  const make = () => {
    const args = implicitAll([self, ...others]);
    return args === undefined ? undefined : binaryOperator(operatorName, args);
  };
  const bridge = () => call("mx_method", { name, args: List.of(self, ...others.map(list)), keep_negative_counts: keepNegativeCounts });
  return native(self, make, bridge);
}
/** `self op other` for `op` in `+ - & | ^`. */
function mxBinary(self: MultisetExpression, o: string, other: unknown): MultisetExpression {
  return native(self, () => binary(self, o, other), () => call("mx_binary", { a: self, o, b: list(other) })) as MultisetExpression;
}
/** A count-adjusting operation: `make` natively, else the bridge's node `mx_counts`. */
function mxCounts(expr: MultisetExpression, o: string, comparison: Comparison | null, n: unknown, make: () => unknown): MultisetExpression {
  return native(expr, make, () => {
    // `__mul__` and `__mod__` return `NotImplemented` for anything but an int, so an Again expression's reflected
    // operator makes an expression of the two (the bridge cannot take one).
    if (n instanceof AgainExpression && (o === "mul" || o === "mod")) return notInt(expr, o, n);
    return call("mx_counts", { expr, o, comparison, n });
  }) as MultisetExpression;
}
function mxEvaluate(expr: MultisetExpression, method: string): Die {
  return call("mx_evaluate", { expr, method }) as Die;
}
function poolCompare(pool: MultisetExpression, other: MultisetArg, cmp: string): Die {
  return call("pool_compare", { pool, other: list(other), cmp }) as Die;
}
/** `self.leximin(comparison, other, extra)` / `leximax` (`max`). */
function lexi(self: MultisetExpression, max: boolean, comparison: unknown, other: unknown, extra: unknown): Die {
  const native = typeof comparison === "string" && typeof extra === "string" && isNative(self)
    ? evaluationOf("lexi", [max, comparison, extra], self, other)
    : undefined;
  return native ?? (call(max ? "pool_leximax" : "pool_leximin", { pool: self, comparison, other: list(other), extra }) as Die);
}
/** A comparison evaluator (evaluator/comparison.py) of `self` and `other`, natively. */
function compare(self: MultisetExpression, other: unknown, name: string): Die | undefined {
  return isNative(self) ? evaluationOf(name, [], self, other) : undefined;
}
function sortPair(expr: MultisetExpression, method: string, comparison: Comparison, other: MultisetArg, order: OrderValue, extra: SortPairExtra): MultisetExpression {
  const bridge = () => call("mx_sort_pair", { expr, method, comparison, other: list(other), order, extra });
  return native(expr, () => sortPairOperator(expr, method, comparison, other, order, extra), bridge);
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
    return native(this, () => middle(this, keep, tie), () => call("pool_middle", { pool: this, keep, tie })) as this;
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
    return super.keep(index as Int) as this | Die;
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
    const bridge = () =>
      dice instanceof Map
        ? call("pool_dict", { items: List.from(Array.from(dice, ([k, v]) => [k, v])), times: list(times) })
        : call("pool", { dice: list(dice), times: list(times) });
    const g = poolGenerator(dice, times);
    if (g === undefined) {
      this.adopt(bridge(), Pool);
    } else {
      this.node = { kind: "generator", g };
      this.replayWith(bridge);
    }
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
    const g = generatorOf(this);
    if (g === undefined) return call("pool_info", { pool: this }) as [Int[], Int, Int, Int, Outcome[], [Die, Int][]];
    return wasm(() => {
      const raw = g.dice();
      const dice: [Die, Int][] = [];
      for (let i = 0; i < raw.length; i += 2) dice.push([Die.wrap(raw[i] as WasmDie), raw[i + 1] as Int]);
      const denominator = pyInt(g.denominator() as bigint);
      return [g.keepTuple() as Int[], g.keepSize() as Int, g.rawSize(), denominator, g.outcomes() as Outcome[], dice];
    });
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
  return native(null, () => standardPool(false, dieSizes), () => bridgeStandardPool("d_pool", dieSizes));
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
  return native(null, () => standardPool(true, dieSizes), () => bridgeStandardPool("z_pool", dieSizes));
}
function bridgeStandardPool(op: string, dieSizes: readonly Int[] | Map<Int, Int>): Pool {
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
  return native(null, () => mixture(inners), () => call("mx_mixture", { inners: list(inners) }));
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

// --- The native expression tree -----------------------------------------------------------------------------------
//
// As the Python facade keeps upstream's expression objects and turns them into the core's tree when one is
// evaluated, an expression here is a TypeScript object whose `node` is a keep generator of the core (`WasmGenerator`:
// a pool or a compound keep), a weightless generator, an operator over child expressions, or a mixture. Python's
// argument juggling happens as the expression is built, in the functions below (ported from the program runner's
// nodes, crates/icepool-harness/src/expression.rs, pool.rs and evaluate.rs, which copy Python's); an evaluation walks
// the tree into the core's `ExpressionBuilder` (src/api/evaluate.rs).
//
// INTERIM (tasks C6e, C6f): whatever the tree does not model (deals, multiset tuple expressions, multiset function
// parameters, JS evaluators, arguments of other types) still goes through the bridge. Each function below returns
// `undefined` for what it does not model, and the method then calls the bridge's node instead; a native expression
// the bridge needs as an argument is made there once, by replaying the bridge call its method stands for (`replay`).

/** How an operator node pushes itself into the core's builder. */
type Push = (b: ExpressionBuilder) => void;
type AnyFunction = (...args: never[]) => unknown;

/** A native expression's tree node. @internal */
export type Node =
  | { readonly kind: "generator"; readonly g: WasmGenerator }
  | { readonly kind: "weightless"; readonly inner: MultisetExpression }
  | {
      readonly kind: "operator";
      readonly children: readonly MultisetExpression[];
      readonly push: Push;
      /** The operator's `_expression_key`: its class and parameters (ints as BigInts, functions by identity). */
      readonly key: readonly unknown[];
      /** False for a `MultisetKeep` given a `List` index, whose `hash_key` cannot be hashed. */
      readonly hashable: boolean;
      /** `str()` given its children's; absent for the operators without a `__str__` (theirs is
       * `object.__repr__`, an address). */
      readonly str?: Str;
    }
  | { readonly kind: "mixture"; readonly inners: readonly (readonly [MultisetExpression, bigint])[] }
  /** `MultisetParameter`: argument `argIndex` of the multiset function being traced (evaluator.ts), named `name`
   * (its `str()`). */
  | { readonly kind: "parameter"; readonly argIndex: number; readonly name: string };

const isNative = (x: MultisetExpression): boolean => x.node !== undefined;

// --- Multiset function parameters (evaluator.ts traces the bodies) ---

/** Thrown where the bridge would need a parameter of a multiset function traced natively: the bridge cannot take
 * one, so the function's call is made through the bridge instead (evaluator.ts `MultisetFunction.call`). INTERIM
 * (task C6f): only deals and multiset tuple expressions still need the bridge. @internal */
export class BridgeNeeded extends Error {}

/** `MultisetParameter`: an argument of a multiset function, while its body is traced. */
class Parameter extends Expression {
  protected override get h(): number {
    throw new BridgeNeeded("icepool: a multiset function parameter cannot go through the bridge");
  }
}

/** The parameter for argument `argIndex` of the function being traced, named `name` (`_make_param`). @internal */
export function parameter(argIndex: number, name: string): MultisetExpression {
  return Parameter.native({ kind: "parameter", argIndex, name });
}

/** `expression._has_parameter`: whether a parameter is anywhere inside (so it is inside a multiset function's
 * body). Without recursion: an expression may nest deeper than JS's stack holds. @internal */
export function hasParameter(x: MultisetExpression): boolean {
  const todo = [x];
  for (let next = todo.pop(); next !== undefined; next = todo.pop()) {
    if (next.node?.kind === "parameter") return true;
    if (next.node !== undefined) todo.push(...childrenOf(next.node));
  }
  return false;
}

/** What a raw result records: a built-in evaluator of the core (by name, with its parameters), a JS evaluator (as
 * evaluator.ts wraps it for the core), or a call of a multiset function with its keyword arguments. @internal */
export type RawEvaluator =
  | { readonly kind: "builtin"; readonly name: string; readonly params: unknown[] }
  | { readonly kind: "host"; readonly evaluator: object }
  | { readonly kind: "function"; readonly f: object; readonly kwargs: object };

/** `MultisetFunctionRawResult(evaluator, body_exps, inner_kwargs)`: what an evaluation returns instead of a die
 * inside a multiset function's body (an input holds a parameter). @internal */
export class RawResult {
  constructor(
    readonly evaluator: RawEvaluator,
    readonly inputs: readonly MultisetExpression[],
  ) {}

  /** Pushes it into a trace's builder: its inputs, then itself (src/api/multiset_function.rs). */
  push(b: ExpressionBuilder): void {
    for (const x of this.inputs) build(b, x);
    const ev = this.evaluator;
    switch (ev.kind) {
      case "builtin":
        return b.rawBuiltin(ev.name, ev.params, this.inputs.length);
      case "host":
        return b.rawHost(ev.evaluator, this.inputs.length);
      case "function":
        return b.rawFunction(ev.f, this.inputs.length, ev.kwargs);
    }
  }
}

/** Python's `isinstance(x, int)` (so also a bool). */
const isIntArg = (x: unknown): x is Int | boolean =>
  typeof x === "bigint" || typeof x === "boolean" || (typeof x === "number" && Number.isInteger(x));

/** An int argument as the core takes it (a bool as 0 or 1). */
const intOf = (x: Int | boolean): Int => (typeof x === "boolean" ? Number(x) : x);

/** An `int | None` argument (`None` for null or undefined); `undefined` for anything else. */
function optionalInt(x: unknown): Int | null | undefined {
  if (x === null || x === undefined) return null;
  return isIntArg(x) ? intOf(x) : undefined;
}

/** An int as Python returns it: a number when it is safe. */
const pyInt = (x: bigint): Int => (Number.isSafeInteger(Number(x)) ? Number(x) : x);

/** A key part: ints as BigInts, so that `1` and `1n` compare equal. */
const keyPart = (x: unknown): unknown => (isIntArg(x) ? BigInt(intOf(x)) : x);

const typeError = (message: string) => new IcepoolError("WrongType", "TypeError", message);
const valueError = (message: string) => new IcepoolError("InvalidArgument", "ValueError", message);

/** The result of a method: the native one, else the bridge's. A new native expression remembers the bridge call
 * it stands for, in case the bridge needs it later. */
function native<T>(self: MultisetExpression | null, make: () => T | undefined, bridge: () => unknown): T {
  const made = self === null || isNative(self) ? make() : undefined;
  if (made === undefined) return bridge() as T;
  if (made instanceof MultisetExpression) made.replayWith(bridge);
  return made;
}

/** A keep generator of the core as an expression: a `Pool`, or a `KeepGenerator` for a compound keep. */
function generator(g: WasmGenerator): KeepGenerator {
  return (g.isPool() ? Pool : KeepGenerator).native({ kind: "generator", g });
}

/** The core generator of a native keep generator, if `x` is one. */
function generatorOf(x: MultisetExpression): WasmGenerator | undefined {
  return x.node?.kind === "generator" ? x.node.g : undefined;
}

/** A keep generator without negative keeps (closed under keep-like operations). */
function keepable(x: MultisetExpression): WasmGenerator | undefined {
  const g = generatorOf(x);
  return g !== undefined && !wasm(() => g.hasNegativeKeeps()) ? g : undefined;
}

/** A `MultisetOperator` node over `children`. */
function operator(
  children: MultisetExpression[],
  key: unknown[],
  push: Push,
  { str, hashable = true }: { str?: Str; hashable?: boolean } = {},
): Expression {
  return Expression.native({ kind: "operator", children, push, key: key.map(keyPart), hashable, str });
}

/** `str()` of an operator, given its children's. */
type Str = (children: readonly string[]) => string;
/** `str()` of a count-adjusting operator: `(child op n)`. */
const countStr = (op: string, n: Int): Str => ([c]) => `(${c} ${op} ${n})`;

/** A binary operator (operator/binary.py) by its class's name. */
function binaryOperator(name: string, children: MultisetExpression[]): Expression {
  // `'(' + (' ' + self.symbol() + ' ').join(str(child) for child in self._children) + ')'`
  const symbol = BINARY_SYMBOLS[name];
  return operator(children, [name], (b) => b.binary(name, children.length), { str: (cs) => `(${cs.join(` ${symbol} `)})` });
}

const BINARY_SYMBOLS: Record<string, string> = {
  MultisetIntersection: "&",
  MultisetDifferenceDropNegative: "-",
  MultisetDifferenceKeepNegative: "-",
  MultisetUnion: "|",
  MultisetAdditiveUnion: "+",
  MultisetSymmetricDifference: "^",
};

/** `+expr`: `MultisetKeepCounts(expr, comparison='>=', constant=0)`. */
const positive = (x: MultisetExpression) => keepCountsOperator(x, ">=", 0);

/** `MultisetKeepCounts(expr, comparison=comparison, constant=n)`. */
function keepCountsOperator(x: MultisetExpression, comparison: string, n: Int): Expression {
  const str: Str = ([c]) => `${c}.keep_counts('${comparison}', ${n})`;
  return operator([x], ["MultisetKeepCounts", comparison, n], (b) => b.keepCounts(comparison, n), { str });
}

/** `implicit_convert_to_expression(x)`: an expression stays itself; a sequence (an array, a str) becomes
 * `Pool(x)`; anything else raises `ImplicitConversionError` (a `TypeError`). */
function implicit(x: unknown): MultisetExpression | undefined {
  if (x instanceof MultisetExpression) return isNative(x) ? x : undefined;
  if (Array.isArray(x) || typeof x === "string") return poolOf(x, 1);
  if (x instanceof Die) throw valueError(`A Pool cannot be constructed with a ${x.constructor.name} argument.`);
  // An `AgainExpression` is neither a mapping nor a sequence; nor is a number.
  if (x instanceof AgainExpression || x === null || x === undefined || ["number", "bigint", "boolean"].includes(typeof x) ||
    x instanceof Float || x instanceof Fraction || x instanceof Complex) {
    throw typeError(`Argument of type ${x instanceof AgainExpression ? "AgainExpression" : pyTypeName(x)} cannot be implicitly converted to a Pool.`);
  }
  return undefined;
}

/** `implicit` of each; `undefined` if any is not native. @internal */
export const implicitAll = (xs: readonly unknown[]): MultisetExpression[] | undefined => {
  const converted = xs.map(implicit);
  return converted.every((x) => x !== undefined) ? (converted as MultisetExpression[]) : undefined;
};

// --- Pools ---

/** `Pool(dice, times)` (generator/pool.py `Pool.__new__`). */
function poolOf(dice: unknown, times: unknown): Pool | undefined {
  const g = poolGenerator(dice, times);
  return g === undefined ? undefined : Pool.native({ kind: "generator", g });
}

function poolGenerator(dice: unknown, times: unknown): WasmGenerator | undefined {
  if (dice instanceof Die) throw valueError(`A Pool cannot be constructed with a ${dice.constructor.name} argument.`);
  if (!(Array.isArray(dice) || typeof dice === "string" || dice instanceof Map)) return undefined;
  // Pools, decks and expressions among the dice: the bridge's.
  const keys = dice instanceof Map ? [...dice.keys()] : [...dice];
  if (keys.some((k) => k instanceof Handle)) return undefined;
  const items = itemize(dice as DieOutcomes, times);
  // `implicit_convert_to_die` of each.
  const dieOf = (x: unknown): Die => {
    if (x instanceof Die) return x;
    if (x instanceof AgainExpression) throw typeError("Again expression cannot be implicitly converted to a Die.");
    return new Die([x as Operand]);
  };
  const converted = items.outcomes.map(dieOf);
  return buildPool(converted.map((die, i) => [die, items.times[i]]));
}

/** A pool of these dice, each put in that many times (already checked to be non-negative numbers). */
function buildPool(dice: (readonly [Die, unknown])[]): WasmGenerator {
  return wasm((w) => {
    const builder = new w.PoolBuilder();
    try {
      for (const [die, count] of dice) builder.add(die.w, count);
      return builder.build();
    } finally {
      builder.free();
    }
  });
}

/** A sequence index of ints and `...` (`null`), for a keep generator; `undefined` for anything else. */
function sequenceIndex(index: readonly unknown[]): (Int | null)[] | undefined {
  const counts = index.map((x) => (x === Ellipsis ? null : isIntArg(x) ? intOf(x) : undefined));
  return counts.every((c) => c !== undefined) ? (counts as (Int | null)[]) : undefined;
}

/** `Die.pool(rolls)`: `Pool({die: rolls})` for an int, else `Pool({die: len(rolls)})[rolls]`. @internal */
export function diePool(die: Die, rolls: Int | readonly Int[]): Pool {
  return native(
    null,
    () => {
      if (isIntArg(rolls)) {
        // `itemize` refuses a negative count.
        if (rolls < 0) throw valueError("times cannot have negative values.");
        return Pool.native({ kind: "generator", g: buildPool([[die, intOf(rolls)]]) });
      }
      const counts = Array.isArray(rolls) ? sequenceIndex(rolls) : undefined;
      if (counts === undefined) return undefined;
      const g = buildPool([[die, counts.length]]);
      return generator(wasm(() => g.keepSequence(counts))) as Pool;
    },
    () => call("die_pool", { die, rolls: Array.isArray(rolls) ? list(rolls) : rolls }),
  );
}

/** The longest list `[k] * v` d_pool builds here; past it the bridge (Python's `MemoryError`, the pool size limit). */
const MAX_STANDARD_POOL = 1 << 16;

/** `icepool.d_pool(die_sizes)` (`z` false) or `icepool.z_pool(die_sizes)`. */
function standardPool(z: boolean, dieSizes: unknown): Pool | undefined {
  let sizes: unknown[] = [];
  if (dieSizes instanceof Map) {
    const [keys, values] = dictItems(dieSizes);
    // If empty, the pool will be considered to consist of zero zeros.
    if (keys.length === 0) return emptyStandardPool();
    // `itertools.chain.from_iterable([k] * v for k, v in die_sizes.items())`.
    for (const [i, k] of keys.entries()) {
      const v = values[i];
      if (!isIntArg(v)) throw typeError("can't multiply sequence by non-int");
      if (sizes.length + Number(v) > MAX_STANDARD_POOL) return undefined;
      for (let j = 0; j < Number(v); j++) sizes.push(k);
    }
  } else if (Array.isArray(dieSizes)) {
    if (dieSizes.length === 0) return emptyStandardPool();
    sizes = [...dieSizes];
  } else {
    return undefined;
  }
  // `icepool.z(x)` checks its argument as `d(x)` does.
  const dice = sizes.map((x) => [(z ? zDie : dDie)(x as Int), 1] as const);
  return Pool.native({ kind: "generator", g: buildPool(dice) });
}

/** `Pool({icepool.Die([0]): 0})`. */
const emptyStandardPool = (): Pool => Pool.native({ kind: "generator", g: buildPool([[new Die([0]), 0]]) });

// --- Keeping ---

/** `MultisetMixture`'s inners if it is `_static_keepable` (all inners keep generators, or such mixtures): then
 * `keep` and `multiply_counts` apply to each inner instead. */
function keepableMixture(x: MultisetExpression): readonly (readonly [MultisetExpression, bigint])[] | undefined {
  const staticKeepable = (e: MultisetExpression): boolean =>
    e.node?.kind === "generator" || (e.node?.kind === "mixture" && e.node.inners.every(([inner]) => staticKeepable(inner)));
  return x.node?.kind === "mixture" && staticKeepable(x) ? x.node.inners : undefined;
}

/** `MultisetMixture._unary_operation(op)`: `op` applied to each inner, counted with its weight in a new mixture. */
function mapMixture(
  inners: readonly (readonly [MultisetExpression, bigint])[],
  op: (inner: MultisetExpression) => unknown,
): Expression | undefined {
  const data: [MultisetExpression, bigint][] = [];
  for (const [inner, weight] of inners) {
    const result = op(inner);
    if (!(result instanceof MultisetExpression) || !isNative(result)) return undefined;
    addInner(data, result, weight);
  }
  return Expression.native({ kind: "mixture", inners: data });
}

/** `MultisetKeep(expr, index=slice(start, stop, step))` (`step` as given, which only `str()` shows). */
function keepSliceOperator(x: MultisetExpression, start: Int | null, stop: Int | null, step: unknown = null, given: unknown[] = [start, stop]): Expression {
  const hasStep = step !== null && step !== undefined;
  const bounds = [...given, step].map(indexRepr);
  const str: Str | undefined = bounds.every((b) => b !== undefined) ? ([c]) => `${c}[slice(${bounds.join(", ")})]` : undefined;
  return operator([x], ["MultisetKeep", "slice", start, stop, hasStep], (b) => b.keepSlice(start, stop, hasStep), { str });
}

/** Python's `repr()` of an element of a keep index (an int, `None`, `...`, a float); `undefined` for others. */
function indexRepr(x: unknown): string | undefined {
  if (x === null || x === undefined) return "None";
  if (x === Ellipsis) return "Ellipsis";
  if (typeof x === "boolean") return x ? "True" : "False";
  if (isIntArg(x)) return String(x);
  if (typeof x === "number") return pyFloatRepr(x);
  return undefined;
}

/** A keep-like operation (`lowest`, `highest`, `middle`) keeping `[start:stop]`: a keep generator without negative
 * keeps stays one; anything else falls back to the less-capable `MultisetKeep` expression. */
function keepSlice(x: MultisetExpression, start: Int | null, stop: Int | null): MultisetExpression | undefined {
  const inners = keepableMixture(x);
  if (inners !== undefined) return mapMixture(inners, (inner) => keepSlice(inner, start, stop));
  const g = keepable(x);
  if (g !== undefined) return generator(wasm(() => g.keepSlice(start, stop, false)));
  return keepSliceOperator(x, start, stop);
}

/** `expr.highest(keep, drop)` / `expr.lowest(keep, drop)`. */
function highestLowest(x: MultisetExpression, highest: boolean, keep: unknown, drop: unknown): MultisetExpression | undefined {
  const [k, d] = [optionalInt(keep), optionalInt(drop)];
  if (k === undefined || d === undefined) return undefined;
  const [start, stop] = wasm((w) => w.keepDropSlice(highest, k, d)) as (Int | null)[];
  return keepSlice(x, start, stop);
}

/** `expr.keep(index)` / `expr[index]` for an int or a sequence of counts. On anything but a keep generator without
 * negative keeps: `MultisetExpression.keep`, i.e. the keep evaluator with an int index, else a `MultisetKeep`. */
function keepIndex(x: MultisetExpression, index: unknown): MultisetExpression | Die | undefined {
  const inners = keepableMixture(x);
  if (inners !== undefined) {
    if (!isIntArg(index)) return mapMixture(inners, (inner) => keepIndex(inner, index));
    // Each inner's `keep(int)` is a die; `keep_evaluator.evaluate(MultisetMixture({die: weight}), index=None)` then
    // asks the first die for `_has_parameter` (an empty mixture evaluates to an empty die).
    for (const [inner] of inners) if (keepIndex(inner, index) === undefined) return undefined;
    if (inners.length === 0) return new Die([]);
    throw new IcepoolError("MissingAttribute", "AttributeError", "'Die' object has no attribute '_has_parameter'");
  }
  const g = keepable(x);
  if (isIntArg(index)) {
    // The keep tuple (or its IndexError) first; then Python evaluates it into a Die.
    if (g !== undefined) return evaluation("keep", [null], [generator(wasm(() => g.keepIndex(intOf(index))))]);
    return evaluation("keep", [intOf(index)], [x]);
  }
  if (g !== undefined) {
    const counts = Array.isArray(index) ? sequenceIndex(index) : undefined;
    return counts === undefined ? undefined : generator(wasm(() => g.keepSequence(counts)));
  }
  // `MultisetKeep(expr, index=index)`: its elements are only looked at when evaluated.
  if (Array.isArray(index)) {
    const counts = index.map((c) => (c === Ellipsis ? null : isIntArg(c) ? intOf(c) : undefined));
    if (counts.some((c) => c === undefined)) return undefined;
    const seq = counts as (Int | null)[];
    // A list index makes the expression unhashable.
    const list = index instanceof List;
    // `str()` of the index: a list's or tuple's repr.
    const items = index.map(indexRepr).join(", ");
    const shown = list ? `[${items}]` : index.length === 1 ? `(${items},)` : `(${items})`;
    const str: Str = ([c]) => `${c}[${shown}]`;
    return operator([x], ["MultisetKeep", "seq", ...seq], (b) => b.keepSequence(seq), { str, hashable: !list });
  }
  // Neither a slice nor a sequence: refused when evaluated, by its type.
  const typeName = index instanceof Die ? index.constructor.name : isPlainValue(index) && typeof index !== "string" &&
    !(index instanceof Vector) && !(index instanceof Symbols) ? pyTypeName(index) : undefined;
  if (typeName === undefined) return undefined;
  const shown = indexRepr(index);
  const str: Str | undefined = shown === undefined ? undefined : ([c]) => `${c}[${shown}]`;
  return operator([x], ["MultisetKeep", "invalid", typeName], (b) => b.keepInvalid(typeName), { str });
}

/** `expr[start:stop:step]`. */
function keepSliceIndex(x: MultisetExpression, start: unknown, stop: unknown, step: unknown): MultisetExpression | undefined {
  const inners = keepableMixture(x);
  if (inners !== undefined) return mapMixture(inners, (inner) => keepSliceIndex(inner, start, stop, step));
  const hasStep = step !== null && step !== undefined;
  let [s, t] = [optionalInt(start), optionalInt(stop)];
  const g = keepable(x);
  if (g === undefined) {
    // `MultisetKeep(index=slice)`: the bounds are only looked at when evaluated.
    return s === undefined || t === undefined ? undefined : keepSliceOperator(x, s, t, step, [start, stop]);
  }
  // A bound that is neither an int nor None is only looked at when the list is sliced, after the checks for a step.
  if (s === undefined || t === undefined) {
    if (!hasStep) throw typeError("slice indices must be integers or None or have an __index__ method");
    [s, t] = [null, null];
  }
  const [a, b] = [s, t];
  return generator(wasm(() => g.keepSlice(a, b, hasStep)));
}

// --- Counts ---

/** `expr * n` and `-expr`: on a keep generator they change its keep tuple, on anything else they build
 * `MultisetMultiplyCounts`. */
function multiplyCounts(x: MultisetExpression, n: Int): MultisetExpression | undefined {
  const inners = keepableMixture(x);
  if (inners !== undefined) return mapMixture(inners, (inner) => multiplyCounts(inner, n));
  const g = generatorOf(x);
  if (g !== undefined) return generator(wasm(() => g.multiplyCounts(n)));
  return operator([x], ["MultisetMultiplyCounts", n], (b) => b.multiplyCounts(n), { str: countStr("*", n) });
}

/** `expr * n` / `expr % n` with a non-int `n`: `__mul__` and `__mod__` return NotImplemented, so Python asks `n`. */
function notInt(x: MultisetExpression, name: "mul" | "mod", n: unknown): AgainExpression {
  // An Again expression's reflected operator makes an expression of the two.
  if (n instanceof AgainExpression) return name === "mul" ? n.rmul(x as unknown as Operand) : n.rmod(x as unknown as Operand);
  // `Die.__rmul__` converts the expression first: `Die([expr])` cannot sort its outcome.
  if (n instanceof Die) throw typeError("Items do not appear to be sortable.");
  throw typeError(`unsupported operand type(s) for ${name === "mul" ? "*" : "%"}`);
}

/** A count constant (`multiply_counts`, `keep_counts`): an int. */
const constant = (n: unknown): Int | undefined => (isIntArg(n) ? intOf(n) : undefined);
/** `unique(True)` is `min(count, True)`, which keeps a bool count: left to the bridge. */
const intConstant = (n: unknown): Int | undefined => (typeof n === "boolean" ? undefined : constant(n));

const KEEP_COUNTS_COMPARISONS = ["==", "!=", "<=", "<", ">=", ">"];

// --- Combining ---

/** `self.additive_union(*args)` as the method resolves on `args[0]`: `Pool.additive_union` merges pools whose
 * sorted positions all count the same, other keep generators make a compound keep. */
function methodAdditiveUnion(args: MultisetExpression[]): MultisetExpression {
  const first = generatorOf(args[0]);
  if (first === undefined) return binaryOperator("MultisetAdditiveUnion", args);
  const generators = args.map(generatorOf);
  if (wasm(() => first.isPool()) && generators.every((g) => g !== undefined && g.isPool())) {
    const merged = wasm((w) => w.WasmGenerator.mergePools(generators.map((g) => (g as WasmGenerator).copy())));
    if (merged !== undefined) return generator(merged);
  }
  return keepGeneratorAdditiveUnion(args);
}

/** `KeepGenerator.additive_union(*args)`: a `CompoundKeepGenerator` if all are keep generators without negative
 * keeps (counting each element once), else `MultisetExpression.additive_union`. */
function keepGeneratorAdditiveUnion(args: MultisetExpression[]): MultisetExpression {
  const generators = args.map(keepable);
  if (generators.every((g) => g !== undefined)) {
    return generator(wasm((w) => w.WasmGenerator.compound(generators.map((g) => (g as WasmGenerator).copy()))));
  }
  return binaryOperator("MultisetAdditiveUnion", args);
}

const BINARY: Record<string, [string, "radd" | "rsub" | "rand" | "ror" | "rxor"]> = {
  "+": ["MultisetAdditiveUnion", "radd"],
  "-": ["MultisetDifferenceDropNegative", "rsub"],
  "&": ["MultisetIntersection", "rand"],
  "|": ["MultisetUnion", "ror"],
  "^": ["MultisetSymmetricDifference", "rxor"],
};

/** `expr op other` for `op` in `+ - & | ^`. */
function binary(x: MultisetExpression, op: string, other: unknown): MultisetExpression | AgainExpression | undefined {
  const [name, reflected] = BINARY[op];
  // An Again expression cannot be converted to a pool: Python then calls its reflected operator.
  if (other instanceof AgainExpression) return other[reflected](x as unknown as Operand);
  const y = implicit(other);
  if (y === undefined) return undefined;
  // `KeepGenerator.__add__` is `self.additive_union(other)`.
  return op === "+" ? methodAdditiveUnion([x, y]) : binaryOperator(name, [x, y]);
}

/** `str(expression)`; `undefined` where Python's is `object.__repr__`, a memory address. */
function expressionStr(x: MultisetExpression): string | undefined {
  const node = x.node as Node;
  const strs = (xs: readonly MultisetExpression[]) => {
    const all = xs.map(expressionStr);
    return all.every((s) => s !== undefined) ? (all as string[]) : undefined;
  };
  switch (node.kind) {
    case "generator":
      return wasm(() => node.g.toString());
    case "weightless":
      return undefined;
    // `MultisetParameterBase.__str__`: `self._name`.
    case "parameter":
      return node.name;
    case "operator": {
      const children = strs(node.children);
      return children === undefined || node.str === undefined ? undefined : node.str(children);
    }
    case "mixture": {
      // `'MultisetMixture({\n' + ',\n'.join(f'{k}: {v}' for k, v in self._inner_expressions.items()) + '})'`
      const inners = strs(node.inners.map(([inner]) => inner));
      if (inners === undefined) return undefined;
      return `MultisetMixture({\n${inners.map((s, i) => `${s}: ${node.inners[i][1]}`).join(",\n")}})`;
    }
  }
}

/** Whether `hash(expression)` works: its `hash_key` holds nothing unhashable. */
function hashable(x: MultisetExpression): boolean {
  const node = x.node;
  return node?.kind !== "operator" || (node.hashable && node.children.every(hashable));
}

/** Whether a dict holding expression `a` finds it equal to `b`: their `hash_key`s. */
function sameExpression(a: MultisetExpression, b: MultisetExpression): boolean {
  const [x, y] = [a.node, b.node];
  if (a === b) return true;
  if (x === undefined || y === undefined) return false;
  if (x.kind === "generator" && y.kind === "generator") return wasm(() => x.g.sameKey(y.g));
  // `(WeightlessGenerator, wrapped.hash_key)`.
  if (x.kind === "weightless" && y.kind === "weightless") return sameExpression(x.inner, y.inner);
  if (x.kind === "parameter" && y.kind === "parameter") return x.argIndex === y.argIndex;
  // `(MultisetMixture, tuple(inners.items()))`.
  if (x.kind === "mixture" && y.kind === "mixture") {
    return x.inners.length === y.inners.length &&
      x.inners.every(([inner, w], i) => sameExpression(inner, y.inners[i][0]) && w === y.inners[i][1]);
  }
  // `(_expression_key, tuple(child.hash_key ...))`.
  if (x.kind === "operator" && y.kind === "operator") {
    return x.key.length === y.key.length && x.key.every((k, i) => k === y.key[i]) &&
      x.children.length === y.children.length && x.children.every((c, i) => sameExpression(c, y.children[i]));
  }
  return false;
}

/** `data[inner] += weight` for a mixture's `defaultdict(int)` of inners. */
function addInner(data: [MultisetExpression, bigint][], inner: MultisetExpression, weight: bigint): void {
  if (!hashable(inner)) throw new IcepoolError("Unhashable", "TypeError", "unhashable type: 'list'");
  const found = data.find(([key]) => sameExpression(key, inner));
  if (found !== undefined) found[1] += weight;
  else data.push([inner, weight]);
}

// --- Evaluating ---

Object.assign(HOST, {
  /** `map_counts`' function: `function(outcome, *counts)`. */
  countFunction: (f: AnyFunction, outcome: unknown, counts: unknown[]) =>
    callback(() => (f as (...args: unknown[]) => unknown)(outcome, ...counts)),
  /** `keep_outcomes`' callable: `bool(func(outcome))`. */
  outcomePredicate: (f: AnyFunction, outcome: unknown) => callback(() => pyBool((f as (x: unknown) => unknown)(outcome))),
});

/** The child expressions of a node. */
function childrenOf(node: Node): readonly MultisetExpression[] {
  switch (node.kind) {
    case "generator":
    case "parameter":
      return [];
    case "weightless":
      return [node.inner];
    case "operator":
      return node.children;
    case "mixture":
      return node.inners.map(([inner]) => inner);
  }
}

/** Pushes a native expression into the builder, children first. Without recursion: an expression may nest deeper
 * than JS's stack holds (the core raises Python's `RecursionError` for it, when it prepares the tree). */
export function build(b: ExpressionBuilder, root: MultisetExpression): void {
  const todo: [Node, boolean][] = [[root.node as Node, false]];
  for (let next = todo.pop(); next !== undefined; next = todo.pop()) {
    const [node, childrenDone] = next;
    if (!childrenDone) {
      todo.push([node, true]);
      for (const child of [...childrenOf(node)].reverse()) todo.push([child.node as Node, false]);
      continue;
    }
    switch (node.kind) {
      case "generator":
        b.generator(node.g);
        break;
      case "weightless":
        b.weightless();
        break;
      case "operator":
        node.push(b);
        break;
      case "mixture":
        b.mixture(node.inners.map(([, weight]) => weight));
        break;
      case "parameter":
        b.parameter(node.argIndex, undefined);
        break;
    }
  }
}

/** `f(b)` with a fresh builder holding the inputs (freed afterwards), called into the core: a JS callback's
 * exception comes out as itself. */
export function withInputs<T>(inputs: readonly MultisetExpression[], f: (b: ExpressionBuilder) => T): T {
  return withCallbacks(() =>
    wasm((w) => {
      const b = new w.ExpressionBuilder();
      let result: T;
      try {
        for (const x of inputs) build(b, x);
        result = f(b);
      } catch (e) {
        // After a trap the abandoned call still borrows the builder: it is left behind (a leak, see wasm.ts).
        if (!isTrap(e)) b.free();
        throw e;
      }
      b.free();
      return result;
    }),
  );
}

/** `evaluator.evaluate(*inputs)` of the core's built-in evaluator `name` (with `params`: evaluate.rs `with_builtin`);
 * `undefined` if an input is not native. Inside a multiset function's body, the raw result recording it. */
function evaluation(name: string, params: unknown[], inputs: MultisetExpression[]): Die | undefined {
  if (!inputs.every(isNative)) return undefined;
  if (inputs.some(hasParameter)) return new RawResult({ kind: "builtin", name, params }, inputs) as unknown as Die;
  return Die.wrap(withInputs(inputs, (b) => b.evaluate(name, params, inputs.length)));
}

/** An evaluation of `self` and the others (implicitly converted). */
function evaluationOf(name: string, params: unknown[], self: MultisetExpression, ...others: unknown[]): Die | undefined {
  const converted = implicitAll(others);
  return converted === undefined ? undefined : evaluation(name, params, [self, ...converted]);
}

// --- The other operators ---

const floordivCounts = (x: MultisetExpression, n: Int) =>
  operator([x], ["MultisetFloordivCounts", n], (b) => b.floordivCounts(n), { str: countStr("//", n) });
const moduloCounts = (x: MultisetExpression, n: Int) =>
  operator([x], ["MultisetModuloCounts", n], (b) => b.moduloCounts(n), { str: countStr("%", n) });

/** `MultisetUnique(expr, constant=n)`. */
function unique(x: MultisetExpression, n: Int): Expression {
  const str: Str = ([c]) => (Number(n) === 1 ? `${c}.unique()` : `${c}.unique(${n})`);
  return operator([x], ["MultisetUnique", n], (b) => b.unique(n), { str });
}

/** `expr.keep_counts(comparison, n)`: the constructor checks the comparison. */
function keepCounts(x: MultisetExpression, comparison: unknown, n: unknown): Expression | undefined {
  if (typeof comparison !== "string") throw valueError("Invalid comparison");
  if (!KEEP_COUNTS_COMPARISONS.includes(comparison)) throw valueError(`Invalid comparison ${comparison}`);
  const c = constant(n);
  if (c === undefined) return undefined;
  return keepCountsOperator(x, comparison, c);
}

/** `expr.keep_outcomes(target)` / `drop_outcomes` (`invert`): an expression target counts while positive
 * (`MultisetFilterOutcomesBinary`); a callable is called per outcome; a collection becomes a `frozenset`. */
function filterOutcomes(x: MultisetExpression, invert: boolean, target: unknown): Expression | undefined {
  if (target instanceof MultisetExpression) {
    if (!isNative(target)) return undefined;
    const str: Str = ([c, t]) => `${c}.${invert ? "drop" : "keep"}_outcomes(${t})`;
    return operator([x, target], ["MultisetFilterOutcomesBinary", invert], (b) => b.filterBinary(invert), { str });
  }
  if (typeof target === "function") {
    const f = target as AnyFunction;
    const push: Push = (b) => b.filterFunction(invert, f);
    return operator([x], ["MultisetFilterOutcomes", f, invert], push, { str: filterStr(invert) });
  }
  if (!Array.isArray(target) && typeof target !== "string") return undefined;
  // `frozenset(outcomes)`: every element hashable.
  const items: unknown[] = [...target];
  if (items.some((item) => item instanceof List)) throw new IcepoolError("Unhashable", "TypeError", "unhashable type: 'list'");
  if (!items.every(isPlainValue)) return undefined;
  // A new function per operator: such operators never compare equal.
  return operator([x], ["MultisetFilterOutcomes", {}, invert], (b) => b.filterSet(invert, items), { str: filterStr(invert) });
}

/** The unary filter shows neither its function nor its set of outcomes. */
const filterStr = (invert: boolean): Str => ([c]) => `${c}.${invert ? "drop" : "keep"}_outcomes(...)`;

/** An `Order` argument: -1, 0 or 1. */
const orderOf = (x: unknown): number | undefined => {
  const o = isIntArg(x) ? Number(intOf(x)) : undefined;
  return o === -1 || o === 0 || o === 1 ? o : undefined;
};

/** `expr.force_order(order)`: `Order.Any` returns the expression itself. */
function forceOrder(x: MultisetExpression, order: unknown): MultisetExpression | undefined {
  const o = orderOf(order);
  if (o === undefined) return undefined;
  return o === 0 ? x : operator([x], ["MultisetForceOrder", o], (b) => b.forceOrder(o));
}

/** `g.weightless()`: only multiset generators have the method, and a `WeightlessGenerator` returns itself. */
function weightless(x: MultisetExpression): MultisetExpression {
  if (x.node?.kind === "weightless") return x;
  if (x.node?.kind !== "generator") {
    throw new IcepoolError("MissingAttribute", "AttributeError", "object has no attribute 'weightless'");
  }
  return Expression.native({ kind: "weightless", inner: x });
}

/** `expr.sort_pair(comparison, other, order, extra)`, `sort_pair_keep_while` and `sort_pair_drop_while`. Nothing
 * is checked until evaluated. */
function sortPairOperator(x: MultisetExpression, method: string, comparison: unknown, other: unknown, order: unknown, extra: unknown): Expression | undefined {
  const y = implicit(other);
  const o = orderOf(order);
  if (y === undefined || o === undefined || typeof comparison !== "string" || typeof extra !== "string") return undefined;
  // `Order.Any` sorts descending (as the constructor does).
  const sortOrder = o === 0 ? -1 : o;
  if (method === "sort_pair") {
    return operator([x, y], ["MultisetSortPair", comparison, sortOrder, extra], (b) => b.sortPair(comparison, sortOrder, extra));
  }
  const keep = method === "sort_pair_keep_while";
  const push: Push = (b) => b.sortPairWhile(keep, comparison, sortOrder, extra);
  return operator([x, y], ["MultisetSortPairWhile", keep, comparison, sortOrder, extra], push);
}

/** lexi.py `compute_lexi_tuple(comparison, order)`. */
const lexiTuple = (comparison: string, order: number): number[] => wasm((w) => w.lexiTuple(comparison, order) as unknown as number[]);

/** `expr.max_pair_keep(comparison, other, priority)` / `max_pair_drop` (`keep`). */
function maxPair(x: MultisetExpression, keep: boolean, comparison: unknown, other: unknown, priority: unknown): MultisetExpression | undefined {
  const y = implicit(other);
  if (y === undefined || typeof comparison !== "string") return undefined;
  if (comparison === "==") {
    // `+self & +other`, or `self - other`.
    if (keep) return binaryOperator("MultisetIntersection", [positive(x), positive(y)]);
    return binaryOperator("MultisetDifferenceDropNegative", [x, y]);
  }
  let order: number;
  let early: boolean;
  if (priority === null || priority === undefined) {
    order = lexiTuple(comparison, 1)[0] !== 0 ? -1 : 1;
    early = false;
  } else if (priority === "low" || priority === "high") {
    order = priority === "low" ? 1 : -1;
    early = lexiTuple(comparison, order)[0] !== 0;
  } else {
    throw valueError("priority must be 'low' or 'high'.");
  }
  const pairEqual = lexiTuple(comparison, order)[1] !== 0;
  const name = early ? "MultisetMaxPairEarly" : "MultisetMaxPairLate";
  return operator([x, y], [name, order, pairEqual, keep], (b) => b.maxPair(early, order, pairEqual, keep));
}

/** `expr.versus_all(comparison, other)` / `versus_any` (`all` false). */
function versus(x: MultisetExpression, all: boolean, comparison: unknown, other: unknown): Expression | undefined {
  const y = implicit(other);
  if (y === undefined || typeof comparison !== "string") return undefined;
  const [a, tie, c, o] = wasm((w) => w.lexiTupleZeroRightFirst(comparison) as unknown as number[]);
  const [lexi, order] = all ? [[a, tie, c], o] : [[c, tie, a], -o];
  return operator([x, y], ["MultisetVersus", ...lexi, order], (b) => b.versus(new Int32Array(lexi), order));
}

/** `MultisetExpression.map_counts(*args, function=function)`. */
function mapCounts(args: unknown[], f: unknown): Expression | undefined {
  const xs = implicitAll(args);
  if (xs === undefined || typeof f !== "function") return undefined;
  const g = f as AnyFunction;
  return operator(xs, ["MultisetMapCounts", g], (b) => b.mapCounts(g, xs.length));
}

/** `icepool.MultisetMixture(inners)` of a sequence of expressions: each counted once, equal ones together. */
function mixture(inners: readonly unknown[]): Expression | undefined {
  if (!inners.every((x) => x instanceof MultisetExpression && isNative(x))) return undefined;
  const data: [MultisetExpression, bigint][] = [];
  for (const inner of inners as MultisetExpression[]) addInner(data, inner, 1n);
  return Expression.native({ kind: "mixture", inners: data });
}

/** `generator.middle(keep, tie=tie)`. */
function middle(x: MultisetExpression, keep: unknown, tie: unknown): MultisetExpression | undefined {
  const g = generatorOf(x);
  if (g === undefined || !isIntArg(keep)) return undefined;
  const [start, stop] = wasm((w) => w.middleSlice(g.keepSize(), intOf(keep), tie)) as (Int | null)[];
  return keepSlice(x, start, stop);
}
