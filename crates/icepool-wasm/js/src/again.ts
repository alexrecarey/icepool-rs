// `icepool.Again` (population/again.py): an expression object standing for "roll again", used as an outcome of
// `new Die(..., { againDepth })`. As in the Python facade, the expression is an object of the API's own language
// (here a plain TypeScript object), and `Die(...)` hands it to the core (crates/icepool-wasm/src/api/again.rs),
// which walks it as Python's `_evaluate` does and calls back here to apply its functions (with the API's operator
// dispatch) and to make dice (with `new Die`). The argument juggling of again.py is below: the checks, the default
// `again_end`, `compute_not_again_die`, and count mode's step die, whose Markov chain is the core's.

import { Die, pyBool, type DieWithTruth, type MapResult, type Operand } from "./die.js";
import { IcepoolError } from "./errors.js";
import { binary, matmul, unary, type BinaryName } from "./operators.js";
import { Reroll, Restart, pyTypeName as typeName, type Int } from "./values.js";
import { HOST, isTrap, wasm } from "./wasm.js";

const BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor", "lt", "le", "gt", "ge"] as const;
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

/** The function an expression applies: the name of Python's `operator` function (`operator.add` is `"add"`). Two
 * expressions' functions are equal (in `hash_key`) when their names are. @internal */
export type AgainFunction = BinaryName | "neg" | "pos" | "abs" | "invert" | "matmul";

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
export class AgainExpression implements Disposable {
  /** The function to apply; `null` for `Again` itself, the again roll directly. @internal */
  readonly func: AgainFunction | null;
  /** The arguments given to `func`, except that those that are Again expressions resolve to the die from rolling
   * again. @internal */
  readonly args: readonly unknown[];
  /** The truth value of an expression from `eq` / `ne` (`null` for the rest, which have none). @internal */
  readonly truthValue: boolean | null;
  /** Whether the expression is compatible with `againCount`: `Again` itself, `add` of additive operands, and
   * `n @ Again` (`rmatmul`) for a non-negative `n`.
   *
   * @example
   * ```ts
   * Again.add(6).isAdditive; //=> true
   * Again.sub(6).isAdditive; //=> false
   * ```
   */
  readonly isAdditive: boolean;

  /** `AgainExpression(function, *args, truth_value=None, is_additive=False)`. Use {@link Again} and its operator
   * methods rather than this. @internal */
  constructor(func: AgainFunction | null, args: readonly unknown[], { truthValue = null, isAdditive = false }: { truthValue?: boolean | null; isAdditive?: boolean } = {}) {
    this.func = func;
    this.args = args;
    this.truthValue = truthValue;
    this.isAdditive = isAdditive;
  }

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
    const truthValue = other instanceof AgainExpression && hashKeyEquals(this, other);
    return new AgainExpression("eq", [this, other], { truthValue });
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
    const truthValue = !(other instanceof AgainExpression && hashKeyEquals(this, other));
    return new AgainExpression("ne", [this, other], { truthValue });
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
    return new AgainExpression("matmul", [this, other]);
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
    let isAdditive = this.isAdditive;
    // A die (a population) counts only if it cannot roll a negative count; another expression never does.
    if (n instanceof Die) isAdditive &&= pyBool(binary("ge", n.minOutcome(), 0));
    else if (n instanceof AgainExpression) isAdditive = false;
    return new AgainExpression("matmul", [n, this], { isAdditive });
  }

  /** `MaybeHashKeyed.equals(other)`: whether `other` is this expression, or an Again expression with the same
   * `hash_key` (the same function applied to equal arguments). Unlike {@link AgainExpression.eq}, a plain boolean.
   *
   * @example
   * ```ts
   * Again.add(1).equals(Again.add(1)); //=> true
   * Again.add(1).equals(Again.add(2)); //=> false
   * Again.equals(d(6)); //=> false
   * ```
   */
  equals(other: unknown): boolean {
    return this === other || (other instanceof AgainExpression && hashKeyEquals(this, other));
  }

  /** Nothing to free: an expression is a plain JS object (kept so that code freeing it keeps working).
   *
   * @example
   * ```ts
   * const plusSix = Again.add(6);
   * plusSix.free();
   * new Die([1, 2, plusSix], { againDepth: 1 }).maxOutcome(); //=> 12
   * ```
   */
  free(): void {}

  /** `free()`, for `using`. */
  [Symbol.dispose](): void {}
}

{
  const proto = AgainExpression.prototype as unknown as Record<string, unknown>;
  for (const op of BINARY) {
    proto[op] = function (this: AgainExpression, other: unknown) {
      // `__add__` is additive if both sides are (a plain operand is).
      const isAdditive = op === "add" && this.isAdditive && (!(other instanceof AgainExpression) || other.isAdditive);
      return new AgainExpression(op, [this, other], { isAdditive });
    };
  }
  // The reflected arithmetic operators: `3 - Again` is `Again.rsub(3)`, `AgainExpression(operator.sub, 3, Again)`.
  for (const op of BINARY.slice(0, 12)) {
    proto[`r${op}`] = function (this: AgainExpression, other: unknown) {
      const isAdditive = op === "add" && this.isAdditive && (!(other instanceof AgainExpression) || other.isAdditive);
      return new AgainExpression(op, [other, this], { isAdditive });
    };
  }
  for (const op of UNARY) {
    proto[op] = function (this: AgainExpression) {
      return new AgainExpression(op, [this]);
    };
  }
}

/** Calls the function of an expression on its evaluated arguments, with Python's operator dispatch (what the
 * core calls back for, as `againApply`). */
function applyFunction(func: AgainFunction, args: unknown[]): unknown {
  switch (func) {
    case "neg":
    case "pos":
    case "abs":
    case "invert":
      return unary(func, args[0]);
    case "matmul":
      return matmul(args[0], args[1]);
    default:
      return binary(func, args[0], args[1]);
  }
}

/** `x.hash_key == y.hash_key`, the tuples `(AgainExpression, self._func, self._args)`: Python compares the
 * functions, then the argument tuples (lengths first, then element by element up to the first unequal one, each
 * by identity, else with `==` and `bool`). Two expressions in there compare by their own `__eq__`, so by
 * `hash_key` again; an expression and anything else are unequal (the expression's truth value is `False`). */
function hashKeyEquals(x: AgainExpression, y: AgainExpression): boolean {
  if (x.func !== y.func || x.args.length !== y.args.length) return false;
  return x.args.every((a, i) => {
    const b = y.args[i];
    if (a instanceof AgainExpression || b instanceof AgainExpression) {
      return a instanceof AgainExpression && b instanceof AgainExpression && hashKeyEquals(a, b);
    }
    return a === b || pyBool(binary("eq", a, b));
  });
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
export const Again: AgainExpression = new AgainExpression(null, [], { isAdditive: true });

// --- Die(...) with Again outcomes (population/again.py) -----------------------------------------------------------

/** What `Die` passes on: its itemized outcomes and times (creation_args.py `itemize`). */
type Items = { outcomes: readonly unknown[]; times: readonly unknown[] };

/** Python's `x < 0` (which raises `TypeError` for anything but a number). */
const negative = (x: unknown) => pyBool(binary("lt", x, 0));

/** An `again_depth` or `again_count`: an int (a bool is one); other numbers are not ported (Python would get as
 * far as `range()`, a `TypeError`). */
function againInt(x: unknown, name: string): number {
  const n = typeof x === "boolean" ? Number(x) : typeof x === "bigint" || Number.isInteger(x) ? Number(x) : NaN;
  if (Number.isNaN(n)) throw new IcepoolError("Unported", "NotImplementedError", `a non-int ${name} (${typeName(x)})`);
  if (n >= 2 ** 32) throw new IcepoolError("Unported", "NotImplementedError", `a huge ${name}`);
  return n;
}

/** `compute_not_again_die(outcomes, times)`: a die of the outcomes that are not `Again` expressions (nor `Reroll`
 * or `Restart`, which an empty die equals). */
function notAgainDie({ outcomes, times }: Items): Die {
  const kept = outcomes.flatMap((outcome, i) => {
    const reroll = outcome === Reroll || outcome === Restart || (outcome instanceof Die && outcome.isEmpty());
    return reroll || outcome instanceof AgainExpression ? [] : [i];
  });
  if (kept.length === 0) {
    // `icepool.Die(*zip(*filtered))` with nothing left calls `Die()`.
    throw new IcepoolError("WrongType", "TypeError", "Die.__new__() missing 1 required positional argument: 'outcomes'");
  }
  return new Die(kept.map((i) => outcomes[i]) as MapResult[], kept.map((i) => times[i]) as Int[]);
}

/** `evaluate_agains_using_depth(outcomes, times, again_depth, again_end)`: the core evaluates the expressions,
 * each level replacing `Again` with the die of the level below. */
export function evaluateAgainsUsingDepth(items: Items, againDepth: unknown, againEnd: unknown): Die {
  if (negative(againDepth)) throw new IcepoolError("InvalidArgument", "ValueError", "again_depth cannot be negative.");
  if (againEnd === null || againEnd === undefined) againEnd = notAgainDie(items).zeroOutcome();
  // Every outcome that is an expression raises this, so the first one does.
  if (againEnd === Restart) throw new IcepoolError("InvalidArgument", "ValueError", "Restart is not compatible with again_depth.");
  const depth = againInt(againDepth, "again_depth");
  const { outcomes, times } = items;
  return wasm((w) => w.evaluateAgainsUsingDepth([...outcomes], [...times], depth, againEnd, againEnd === Reroll, Reroll)) as Die;
}

/** `evaluate_agains_using_count(outcomes, times, again_count, again_end)`: the step die is built here as Python
 * builds it (its outcomes by the core, which evaluates the expressions); the Markov chain over it is the core's. */
export function evaluateAgainsUsingCount(items: Items, againCount: unknown, againEnd: unknown): Die {
  if (negative(againCount)) throw new IcepoolError("InvalidArgument", "ValueError", "again_count cannot be negative.");
  const count = againInt(againCount, "again_count");
  const notAgain = notAgainDie(items);
  const zero = notAgain.zeroOutcome();
  if (againEnd === null || againEnd === undefined) againEnd = zero;
  else if (againEnd === Reroll) againEnd = notAgain;
  const restart = againEnd === Restart;
  const againEndDie = restart ? notAgain : new Die([againEnd as MapResult]);
  // `make_step_outcome` for each outcome: add_flat, add_not_again, add_again (tupleized by `Die`).
  const stepOutcomes = wasm((w) => w.againStepOutcomes([...items.outcomes], zero)) as MapResult[];
  const stepDie = new Die(stepOutcomes, items.times as Int[]);
  return Die.wrap(wasm((w) => w.WasmDie.evaluateAgainsUsingCount(stepDie.w, notAgain.w, zero, count, againEndDie.w, restart)));
}

// --- What the core calls back for (src/api/host.rs) ---------------------------------------------------------------

/** A callback's result for the core: `[true, result]`, or `[false, exception]` for what it threw (a trap below it
 * must abandon the WASM frames above it too, so it is rethrown). */
function envelope(f: () => unknown): [boolean, unknown] {
  try {
    return [true, f()];
  } catch (e) {
    if (isTrap(e)) throw e;
    return [false, e];
  }
}

Object.assign(HOST, {
  againParts: (x: unknown) => (x instanceof AgainExpression ? [x.func, x.args, x.isAdditive] : []),
  againApply: (func: AgainFunction, args: unknown[]) => envelope(() => applyFunction(func, args)),
  againDie: (objects: MapResult[], times: Int[]) => envelope(() => new Die(objects, times)),
});
