// `icepool.Again` (population/again.py): an expression object standing for "roll again", used as an outcome of
// `new Die(..., { againDepth })`. INTERIM (task C6): held through the bridge (rpc.ts).

import type { DieWithTruth, Operand } from "./die.js";
import { HANDLE, Handle, call, registerKind } from "./rpc.js";
import type { Int } from "./values.js";

type BinaryName =
  | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
  | "lt" | "le" | "gt" | "ge";
const BINARY: BinaryName[] = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor", "lt", "le", "gt", "ge"];
const UNARY = ["neg", "pos", "abs", "invert"] as const;

// The operator methods, installed on the prototype below. Each example is a die `[1, 2, <expression>]` with
// `againDepth: 1`: the third face rolls the die again, and at the last depth `Again` is 0 (the default `againEnd`).
export interface AgainExpression {
  /** `Again + other`: roll again and add `other`. Additive: allowed with `againCount`.
   *
   * @example
   * ```ts
   * // A d6 that explodes once on a 6.
   * new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 1 }).outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
   * new Die([1, 2, Again.add(3)], { againDepth: 1 }).items(); //=> [[1, 3n], [2, 3n], [4, 1n], [5, 1n], [6, 1n]]
   * ```
   */
  add(other: Operand): AgainExpression;
  /** `Again - other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.sub(3)], { againDepth: 1 }).outcomes(); //=> [-6, -2, -1, 1, 2]
   * ```
   */
  sub(other: Operand): AgainExpression;
  /** `Again * other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.mul(3)], { againDepth: 1 }).outcomes(); //=> [0, 1, 2, 3, 6]
   * ```
   */
  mul(other: Operand): AgainExpression;
  /** `Again / other` (the same as {@link AgainExpression.div}).
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.truediv(2)], { againDepth: 1 }).probability(0.5).toString(); //=> "1/9"
   * ```
   */
  truediv(other: Operand): AgainExpression;
  /** `Again // other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.floordiv(3)], { againDepth: 1 }).items(); //=> [[0, 3n], [1, 3n], [2, 3n]]
   * ```
   */
  floordiv(other: Operand): AgainExpression;
  /** `Again % other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.mod(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  mod(other: Operand): AgainExpression;
  /** `Again ** other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.pow(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 3n], [8, 1n]]
   * ```
   */
  pow(other: Operand): AgainExpression;
  /** `Again << other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.lshift(3)], { againDepth: 1 }).outcomes(); //=> [0, 1, 2, 8, 16]
   * ```
   */
  lshift(other: Operand): AgainExpression;
  /** `Again >> other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rshift(1)], { againDepth: 1 }).items(); //=> [[0, 2n], [1, 4n], [2, 3n]]
   * ```
   */
  rshift(other: Operand): AgainExpression;
  /** `Again & other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.and(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  and(other: Operand): AgainExpression;
  /** `Again | other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.or(3)], { againDepth: 1 }).items(); //=> [[1, 3n], [2, 3n], [3, 3n]]
   * ```
   */
  or(other: Operand): AgainExpression;
  /** `Again ^ other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.xor(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  xor(other: Operand): AgainExpression;
  /** `other + Again`. Additive, like {@link AgainExpression.add}.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.radd(3)], { againDepth: 1 }).items(); //=> [[1, 3n], [2, 3n], [4, 1n], [5, 1n], [6, 1n]]
   * ```
   */
  radd(other: Operand): AgainExpression;
  /** `other - Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rsub(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  rsub(other: Operand): AgainExpression;
  /** `other * Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rmul(3)], { againDepth: 1 }).outcomes(); //=> [0, 1, 2, 3, 6]
   * ```
   */
  rmul(other: Operand): AgainExpression;
  /** `other / Again`. With the default `againEnd` of 0 this divides by zero at the last depth; here it is 1.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rtruediv(6)], { againDepth: 1, againEnd: 1 }).probability(3).toString(); //=> "1/9"
   * new Die([1, 2, Again.rtruediv(6)], { againDepth: 1 }); //=> throws ZeroDivisionError
   * ```
   */
  rtruediv(other: Operand): AgainExpression;
  /** `other // Again`.
   *
   * @example
   * ```ts
   * const floored = new Die([1, 2, Again.rfloordiv(6)], { againDepth: 1, againEnd: 1 });
   * floored.items(); //=> [[1, 4n], [2, 3n], [3, 1n], [6, 1n]]
   * ```
   */
  rfloordiv(other: Operand): AgainExpression;
  /** `other % Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rmod(7)], { againDepth: 1, againEnd: 3 }).items(); //=> [[0, 2n], [1, 4n], [2, 3n]]
   * ```
   */
  rmod(other: Operand): AgainExpression;
  /** `other ** Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rpow(3)], { againDepth: 1 }).items(); //=> [[1, 3n], [2, 3n], [3, 2n], [9, 1n]]
   * ```
   */
  rpow(other: Operand): AgainExpression;
  /** `other << Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rlshift(3)], { againDepth: 1 }).outcomes(); //=> [1, 2, 6, 12, 24]
   * ```
   */
  rlshift(other: Operand): AgainExpression;
  /** `other >> Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rrshift(3)], { againDepth: 1 }).items(); //=> [[0, 2n], [1, 4n], [2, 3n]]
   * ```
   */
  rrshift(other: Operand): AgainExpression;
  /** `other & Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rand(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  rand(other: Operand): AgainExpression;
  /** `other | Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.ror(3)], { againDepth: 1 }).items(); //=> [[1, 3n], [2, 3n], [3, 3n]]
   * ```
   */
  ror(other: Operand): AgainExpression;
  /** `other ^ Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.rxor(3)], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  rxor(other: Operand): AgainExpression;
  /** `Again < other`: a boolean outcome (in a die, `true` and `false` equal 1 and 0, as in Python).
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.lt(3)], { againDepth: 1 }).items(); //=> [[1, 6n], [2, 3n]]
   * ```
   */
  lt(other: Operand): AgainExpression;
  /** `Again <= other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.le(3)], { againDepth: 1 }).items(); //=> [[1, 6n], [2, 3n]]
   * ```
   */
  le(other: Operand): AgainExpression;
  /** `Again > other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.gt(1)], { againDepth: 1 }).items(); //=> [[false, 2n], [1, 4n], [2, 3n]]
   * ```
   */
  gt(other: Operand): AgainExpression;
  /** `Again >= other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.ge(3)], { againDepth: 1 }).items(); //=> [[false, 3n], [1, 3n], [2, 3n]]
   * ```
   */
  ge(other: Operand): AgainExpression;
  /** `-Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.neg()], { againDepth: 1 }).outcomes(); //=> [-2, -1, 0, 1, 2]
   * ```
   */
  neg(): AgainExpression;
  /** `+Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.pos()], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  pos(): AgainExpression;
  /** `abs(Again)`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.neg().abs()], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 4n], [2, 4n]]
   * ```
   */
  abs(): AgainExpression;
  /** `~Again`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.invert()], { againDepth: 1 }).outcomes(); //=> [-3, -2, 0, 1, 2]
   * ```
   */
  invert(): AgainExpression;
}

/** An `AgainExpression`: `Again`, or operators applied to it. An expression indicating that the die should be
 * rolled again, usually with some operation applied; see {@link Again} for the full documentation.
 *
 * Any of the operands may themselves be Again expressions: these are at the same level for `againDepth`.
 *
 * @example
 * ```ts
 * const twice: AgainExpression = Again.add(Again); // roll again twice and sum
 * new Die([1, 2, twice], { againDepth: 1 }).items(); //=> [[0, 1n], [1, 11n], [2, 12n], [3, 2n], [4, 1n]]
 * ```
 */
export class AgainExpression extends Handle {
  /** `==`: an expression, with a truth value (`truth(expr)`) as Python's: whether both sides are the same Again
   * expression.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.eq(1)], { againDepth: 1 }).items(); //=> [[false, 2n], [1, 4n], [2, 3n]]
   * truth(Again.eq(Again)); //=> true
   * truth(Again.eq(3)); //=> false
   * ```
   */
  eq(other: Operand): AgainExpression | DieWithTruth {
    return call("eq", { a: this, b: other }) as AgainExpression;
  }
  /** `!=`: an expression, with a truth value (`truth(expr)`) as Python's: whether the two sides are different
   * expressions.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.ne(2)], { againDepth: 1 }).items(); //=> [[false, 1n], [1, 5n], [2, 3n]]
   * truth(Again.ne(Again)); //=> false
   * ```
   */
  ne(other: Operand): AgainExpression | DieWithTruth {
    return call("ne", { a: this, b: other }) as AgainExpression;
  }
  /** `truediv`: `Again / other`.
   *
   * @example
   * ```ts
   * new Die([1, 2, Again.div(2)], { againDepth: 1 }).probability(0.5).toString(); //=> "1/9"
   * ```
   */
  div(other: Operand): AgainExpression {
    return this.truediv(other);
  }
  /** `Again @ other`: roll again, then roll `other` that many times and sum. Python's `@` needs a die on the
   * right, so the last depth's `Again` must not make it `int @ int`.
   *
   * @example
   * ```ts
   * // The third face rolls again and sums that many d2s.
   * const summed = new Die([1, 2, Again.matmul(d(2))], { againDepth: 1, againEnd: new Die([1]) });
   * summed.items(); //=> [[1, 30n], [2, 33n], [3, 6n], [4, 3n]]
   * new Die([1, 2, Again.matmul(2)], { againDepth: 1 }); //=> throws TypeError
   * ```
   */
  matmul(other: Int | Operand): AgainExpression {
    return call("matmul", { n: this, die: other }) as AgainExpression;
  }
  /** `n @ Again`: roll again `n` times and sum. Additive (allowed with `againCount`) for a non-negative `n`. As in
   * Python, `Again` must stand for a die here: with the default `againEnd` of 0 the last depth is `int @ int`, a
   * `TypeError`.
   *
   * @example
   * ```ts
   * const twice = new Die([1, 2, Again.rmatmul(2)], { againDepth: 1, againEnd: new Die([0]) });
   * twice.equals(new Die([1, 2, Again.add(Again)], { againDepth: 1 })); //=> true
   * ```
   */
  rmatmul(n: Int | Operand): AgainExpression {
    return call("matmul", { n, die: this }) as AgainExpression;
  }
}

{
  const proto = AgainExpression.prototype as unknown as Record<string, unknown>;
  for (const op of BINARY) {
    proto[op] = function (this: AgainExpression, other: unknown) {
      return call(op, { a: this, b: other });
    };
  }
  // The reflected arithmetic operators: `3 - Again` is `Again.rsub(3)`.
  for (const op of BINARY.slice(0, 12)) {
    proto[`r${op}`] = function (this: AgainExpression, other: unknown) {
      return call(op, { a: other, b: this });
    };
  }
  for (const op of UNARY) {
    proto[op] = function (this: AgainExpression) {
      return call(op, { a: this });
    };
  }
}

registerKind("again", (h) => new AgainExpression(HANDLE, h));

/** The one shared `Again`, made on first use; `free()` leaves it. */
class SharedAgain extends AgainExpression {
  protected override get h(): number {
    if (this.handle < 0) this.adopt(call("Again"), AgainExpression);
    return this.handle;
  }
  override free(): void {}
}

/** `icepool.Again`, for `new Die([1, 2, Again.add(3)], { againDepth: 2 })`: a symbol indicating that the die should
 * be rolled again, usually with some operation applied.
 *
 * This is designed to be used with the `Die` constructor. Again expressions should not be fed to functions or
 * methods other than `new Die(...)` (or indirectly via `map()`), but they can be used with the operator methods:
 * `Again.add(6)` rolls again and adds 6, `Again.add(Again)` rolls again twice and sums.
 *
 * The `againCount`, `againDepth` and `againEnd` options of `new Die(...)` affect how these are processed. At most
 * one of `againCount` or `againDepth` may be given; if neither is, the behaviour is as `againDepth: 1`. For finer
 * control over rolling processes, use e.g. `Die.map()` instead.
 *
 * **Count mode.** With `againCount`, we start with one roll queued and execute one roll at a time. For every `Again`
 * we roll, we queue another roll. If we run out of rolls, we sum the rolls to find the result. We evaluate up to
 * `againCount` extra rolls. If, at this point, there are still dice remaining:
 * - `Restart`: if there would be dice over the limit, we restart the entire process from the beginning,
 *   effectively conditioning the process against this sequence of events;
 * - `Reroll`: any remaining dice can't produce more `Again`s;
 * - an outcome: any remaining dice are each treated as the given outcome;
 * - `null`: any remaining dice are treated as zero.
 *
 * This mode only allows "additive" expressions: binary `add` (`radd`), and `rmatmul(n)` (`n @ Again`) where `n` is
 * a non-negative int or population. `add` is assumed to be associative and commutative, so for example string or
 * array outcomes will not produce elements with a definite order. Anything else throws `ValueError`.
 *
 * **Depth mode.** With `againDepth: 0`, `againEnd` is substituted directly for each occurrence of `Again`. For other
 * values of `againDepth`, the result for `againDepth - 1` is substituted for each `Again`. With `againEnd: Reroll`,
 * any Again expressions in the final depth are rerolled. `Restart` cannot be used with `againDepth`.
 *
 * @example
 * ```ts
 * // A d6 that explodes once on a 6 (Python: `Die([1, 2, 3, 4, 5, Again + 6], again_depth=1)`).
 * const exploding = new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 1 });
 * exploding.outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
 * exploding.probability(12).toString(); //=> "1/36"
 * // Count mode: up to two extra rolls, the same die here as two levels of depth.
 * const counted = new Die([1, 2, 3, 4, 5, Again.add(6)], { againCount: 2 });
 * counted.equals(new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 2 })); //=> true
 * new Die([1, 2, Again.sub(2)], { againCount: 2 }); //=> throws ValueError
 * ```
 */
export const Again: AgainExpression = new SharedAgain(HANDLE, -1);
