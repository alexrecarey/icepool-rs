// Python's value types that JS has no counterpart for, and the sentinels (`Reroll`, `Order`, ...).
//
//   int       a JS number when it is a safe integer, else a BigInt (both go in as ints)
//   float     a JS number when it is not integral; an integral float (2.0, -0.0) comes out as a `Float`, so that it
//             goes back in as a float. Any non-integral number, Infinity and NaN go in as floats.
//   tuple     a JS array (a Python list, where an argument must be one, is a `List`)
//   Fraction, complex, Vector, Symbols: the classes below. None is null.

import type { Die } from "./die.js";
import { binary, round, unary } from "./operators.js";
import { call } from "./rpc.js";
import { wasm } from "./wasm.js";

/** Anything that can be an outcome: an int (a JS integer or BigInt), a float (a non-integral number or a
 * {@link Float}), a string, a bool, `null` (Python's `None`), a {@link Fraction}, {@link Complex}, {@link Vector} or
 * {@link Symbols}, or an array of outcomes (a Python tuple). The outcomes of one die must be mutually comparable.
 *
 * @example
 * ```ts
 * import { Die, type Outcome } from "icepool-wasm";
 * const faces: Outcome[] = [[1, "a"], [2, "b"]];
 * new Die(faces).outcomes(); //=> [[1, "a"], [2, "b"]]
 * ```
 */
export type Outcome =
  | number
  | bigint
  | string
  | boolean
  | null
  | Float
  | Fraction
  | Complex
  | Vector
  | Symbols
  | readonly Outcome[];

/** An int argument: a JS integer or a BigInt. Ints come back as JS numbers while they are safe integers, as BigInts
 * beyond.
 *
 * @example
 * ```ts
 * import { d, type Int } from "icepool-wasm";
 * const sides: Int = 6n;
 * d(sides).outcomes().length; //=> 6
 * d(6).denominator(); //=> 6n
 * ```
 */
export type Int = number | bigint;

/** A Python float that happens to be integral (`2.0`), so that it stays a float. Behaves as its number.
 *
 * Non-integral floats are plain JS numbers; only an integral one needs this wrapper, since JS cannot tell `2.0`
 * from `2` (and Python's `Die({2.0: 1})` is not `Die({2: 1})`).
 *
 * @example
 * ```ts
 * d(4).div(2).outcomes().map(String); //=> ["0.5", "1.0", "1.5", "2.0"]
 * new Die([new Float(2)]).repr(); //=> "Die({2.0: 1})"
 * new Float(2).valueOf() + 1; //=> 3
 * ```
 */
export class Float {
  /** The number. */
  readonly value: number;
  /** A float with this value (`new Float(2)` is Python's `2.0`). */
  constructor(value: number) {
    this.value = Number(value);
  }
  /** The number, so that arithmetic and comparisons in JS use it.
   *
   * @example
   * ```ts
   * new Float(2).valueOf(); //=> 2
   * new Float(2) < new Float(3); //=> true
   * ```
   */
  valueOf(): number {
    return this.value;
  }
  /** Python's `repr(float)`: `"2.0"`, `"-0.0"`, `"1e+16"` (see {@link pyFloatRepr}).
   *
   * @example
   * ```ts
   * String(new Float(2)); //=> "2.0"
   * String(new Float(-0)); //=> "-0.0"
   * ```
   */
  toString(): string {
    return pyFloatRepr(this.value);
  }
}

/** An exact fraction (Python's `Fraction`), normalised by the core when it goes in: the constructor itself keeps
 * the numerator and denominator as given. Means, probabilities and other exact results come back as fractions.
 *
 * @example
 * ```ts
 * d(6).mean().toString(); //=> "7/2"
 * d(6).probability(6); //=> new Fraction(1, 6)
 * new Die([new Fraction(1, 2), new Fraction(2, 4)]).items(); //=> [[new Fraction(1, 2), 2n]]
 * ```
 */
export class Fraction {
  /** The numerator (as given to the constructor; lowest terms when it comes from the core). */
  readonly numerator: bigint;
  /** The denominator (as given to the constructor; positive and in lowest terms when it comes from the core). */
  readonly denominator: bigint;
  /** `Fraction(numerator, denominator=1)`, not reduced until it goes into the core. */
  constructor(numerator: Int, denominator: Int = 1n) {
    this.numerator = BigInt(numerator);
    this.denominator = BigInt(denominator);
  }
  /** The nearest JS number (inexact: `numerator / denominator` as floats).
   *
   * @example
   * ```ts
   * d(6).mean().valueOf(); //=> 3.5
   * +new Fraction(1, 4); //=> 0.25
   * ```
   */
  valueOf(): number {
    return Number(this.numerator) / Number(this.denominator);
  }
  /** Python's `str(Fraction)`: `"7/2"`, or `"3"` when the denominator is 1.
   *
   * @example
   * ```ts
   * d(6).mean().toString(); //=> "7/2"
   * new Fraction(3).toString(); //=> "3"
   * ```
   */
  toString(): string {
    return this.denominator === 1n ? `${this.numerator}` : `${this.numerator}/${this.denominator}`;
  }
}

/** A Python list, where an argument must be a list rather than a tuple (a JS array is a tuple).
 *
 * @example
 * ```ts
 * const xs = List.from([1, 2]);
 * xs instanceof Array; //=> true
 * [...xs]; //=> [1, 2]
 * ```
 */
export class List<T = unknown> extends Array<T> {}

/** `icepool.Break(outcome)`: EXPERIMENTAL. Returned by the function of `map(f, ..., { repeat })` to stop the
 * repetition early with `outcome`; `Break()` (outcome `null`) stops it with the current state, `f`'s first argument.
 *
 * @example
 * ```ts
 * // Add successive d6 until the total reaches 10.
 * const step = (total: number, roll: number) => (total >= 10 ? new Break() : total + roll);
 * map(step, 0, d(6), { repeat: Infinity }).outcomes(); //=> [10, 11, 12, 13, 14, 15]
 * ```
 */
export class Break {
  /** The wrapped outcome; `null` stands for the first argument of the current iteration of `map`. */
  readonly outcome: Outcome | Die;
  /** `Break(outcome=None)`. */
  constructor(outcome: Outcome | Die = null) {
    this.outcome = outcome;
  }
}

/** Python's `complex`: `String(z)` is Python's repr, e.g. `(1+2j)`, `-0j`. Complex numbers are not ordered, so they
 * cannot be the outcomes of a die (that is a `TypeError`), but they can be elements of a {@link Vector}.
 *
 * @example
 * ```ts
 * String(new Complex(1, 2)); //=> "(1+2j)"
 * String(new Complex(0, -1.5)); //=> "-1.5j"
 * new Vector([new Complex(1, 2)]).add(1).repr(); //=> "Vector(((2+2j),))"
 * new Die([new Complex(1, 2)]); //=> throws TypeError
 * ```
 */
export class Complex {
  /** The real part. */
  readonly re: number;
  /** The imaginary part. */
  readonly im: number;
  /** `complex(re, im=0)`. */
  constructor(re: number, im = 0) {
    this.re = re;
    this.im = im;
  }
  /** Python's `repr(complex)` (see {@link pyComplexRepr}).
   *
   * @example
   * ```ts
   * new Complex(1, -2).toString(); //=> "(1-2j)"
   * new Complex(0, -0).toString(); //=> "-0j"
   * ```
   */
  toString(): string {
    return pyComplexRepr(this.re, this.im);
  }
}

/** A Python object JS has no type for: it can be read, not passed back.
 *
 * @example
 * ```ts
 * const x = new Opaque("<object at 0x1>");
 * String(x); //=> "<object at 0x1>"
 * ```
 */
export class Opaque {
  /** Python's `repr()` of the object. */
  readonly repr: string;
  /** An opaque value with this repr. The API makes these when the core returns such an object. */
  constructor(repr: string) {
    this.repr = repr;
  }
  /** The object's repr.
   *
   * @example
   * ```ts
   * new Opaque("<object at 0x1>").toString(); //=> "<object at 0x1>"
   * ```
   */
  toString(): string {
    return this.repr;
  }
}

/** `icepool.Reroll`, `icepool.Restart` and `...` (Ellipsis, for keep indices). */
export class Special {
  readonly name: "Reroll" | "Restart" | "Ellipsis";
  constructor(name: "Reroll" | "Restart" | "Ellipsis") {
    this.name = name;
  }
  toString(): string {
    return this.name;
  }
}
/** `icepool.Reroll`: indicates that an outcome should be rerolled (with unlimited depth). As an outcome, or returned
 * by a `map` function, it drops out (the die is rerolled).
 *
 * This effectively removes the outcome from the probability space, along with its contribution to the denominator,
 * so it can be used for conditional probability by removing all outcomes not consistent with the given
 * observations. In specific cases:
 *
 * - Given to the `Die` constructor, it and its quantity are dropped.
 * - With `Again` or `map(..., { repeat })`, only that stage is rerolled, not the entire rolling process.
 * - To reroll with limited depth, use `Die.reroll()`, or `Again` with no modification.
 * - In a `MultisetEvaluator`, it currently means the same as {@link Restart}; prefer `Restart` there.
 *
 * @example
 * ```ts
 * new Die([1, 2, Reroll]).outcomes(); //=> [1, 2]
 * // 2d6, given that the total is at least 10.
 * d(6).add(d(6)).map((x: number) => (x < 10 ? Reroll : x)).items(); //=> [[10, 3n], [11, 2n], [12, 1n]]
 * ```
 */
export const Reroll = new Special("Reroll");
/** `icepool.Restart`: indicates that a rolling process should be restarted (with unlimited depth).
 *
 * It effectively removes the sequence of events from the probability space, along with its contribution to the
 * denominator, so it can be used for conditional probability by removing all sequences of events not consistent
 * with the given observations. It can be used with `againCount`, `map(..., { repeat })` or a `MultisetEvaluator`.
 * Given to the `Die` constructor it has the same effect as {@link Reroll} (prefer `Reroll` there). It can NOT be
 * used with `againDepth`, but `Reroll` can.
 *
 * @example
 * ```ts
 * // The total of two d6, starting over whenever a 1 is rolled.
 * const total = map((total: number, roll: number) => (roll === 1 ? Restart : total + roll), 0, d(6), { repeat: 2 });
 * total.outcomes(); //=> [4, 5, 6, 7, 8, 9, 10, 11, 12]
 * ```
 */
export const Restart = new Special("Restart");
/** `...` (Python's `Ellipsis`), for keep tuples: up to one may be used, and it stands for as many zero counts as
 * the pool's size needs (`[1, Ellipsis, 1]` on five dice is `[1, 0, 0, 0, 1]`), so `pool.keep([Ellipsis, 1])` keeps
 * the highest die whatever the size. If the tuple is longer than the pool, elements are dropped on the side of the
 * `...` (`[Ellipsis, 1, 2, 3]` on two dice is `[2, 3]`).
 *
 * @example
 * ```ts
 * d(6).pool(4).keep([Ellipsis, 1]).sum().equals(d(6).pool(4).highest(1).sum()); //=> true
 * String(Ellipsis); //=> "Ellipsis"
 * ```
 */
export const Ellipsis = new Special("Ellipsis");

/** `icepool.Order`: the order in which a `MultisetEvaluator` sees outcomes. `Ascending` (1), `Descending` (-1) or
 * `Any` (0), the numbers Python's `IntEnum` has.
 *
 * @example
 * ```ts
 * Order.Descending; //=> -1
 * // An evaluator that remembers the last outcome it saw: in ascending order, the highest.
 * const last = evaluator({ nextState: (state, order, outcome) => outcome });
 * last.evaluate(d(6).pool(1).forceOrder(Order.Ascending)).probability(6).toString(); //=> "1"
 * ```
 */
export const Order = Object.freeze({
  /** Lowest outcome first. */
  Ascending: 1,
  /** Highest outcome first. */
  Descending: -1,
  /** Either order: the evaluator works in both. */
  Any: 0,
} as const);
/** `Order.Ascending` (1), `Order.Descending` (-1) or `Order.Any` (0).
 *
 * @example
 * ```ts
 * import { Order, type OrderValue } from "icepool-wasm";
 * const order: OrderValue = Order.Ascending;
 * order; //=> 1
 * ```
 */
export type OrderValue = 1 | -1 | 0;

/** Python's float repr: shortest round-trip digits, exponent form below 1e-4 or from 1e16 (CPython's 'r').
 *
 * @example
 * ```ts
 * pyFloatRepr(2); //=> "2.0"
 * pyFloatRepr(0.1); //=> "0.1"
 * pyFloatRepr(1e16); //=> "1e+16"
 * pyFloatRepr(1e-5); //=> "1e-05"
 * pyFloatRepr(-Infinity); //=> "-inf"
 * ```
 */
export function pyFloatRepr(x: number): string {
  if (Number.isNaN(x)) return "nan";
  if (!Number.isFinite(x)) return x > 0 ? "inf" : "-inf";
  if (x === 0) return Object.is(x, -0) ? "-0.0" : "0.0";
  const [mant, e] = x.toExponential().split("e");
  const exp = Number(e);
  const sign = mant.startsWith("-") ? "-" : "";
  const digits = mant.replace("-", "").replace(".", "");
  if (exp < -4 || exp >= 16) {
    const m = digits.length > 1 ? `${digits[0]}.${digits.slice(1)}` : digits;
    return `${sign}${m}e${exp < 0 ? "-" : "+"}${String(Math.abs(exp)).padStart(2, "0")}`;
  }
  if (exp < 0) return `${sign}0.${"0".repeat(-exp - 1)}${digits}`;
  const int = digits.slice(0, exp + 1).padEnd(exp + 1, "0");
  const frac = digits.slice(exp + 1);
  return `${sign}${int}.${frac || "0"}`;
}

/** Python's `repr(complex)`: `(re+imj)`, or just `imj` when the real part is +0.0; no trailing `.0`.
 *
 * @example
 * ```ts
 * pyComplexRepr(1, 2); //=> "(1+2j)"
 * pyComplexRepr(1.5, -0.5); //=> "(1.5-0.5j)"
 * pyComplexRepr(0, 1); //=> "1j"
 * pyComplexRepr(-0, 1); //=> "(-0+1j)"
 * ```
 */
export function pyComplexRepr(re: number, im: number): string {
  const short = (x: number) => pyFloatRepr(x).replace(/\.0$/, "");
  if (re === 0 && !Object.is(re, -0)) return `${short(im)}j`;
  const i = short(im);
  return `(${short(re)}${i.startsWith("-") ? "" : "+"}${i}j)`;
}

// --- Vector and Symbols -------------------------------------------------------------------------------------------

/** Python's operators on a Vector or Symbols. Comparisons give what Python gives: a bool, or (between vectors) a
 * vector with a truth value. */
interface ValueOperators<Self> {
  /** `+`: elementwise for a Vector; the sum of the counts of each symbol for Symbols.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).add(new Vector([3, 4])).repr(); //=> "Vector((4, 6))"
   * new Symbols("aab").add("abc").repr(); //=> "Symbols('aaabbc')"
   * ```
   */
  add(other: unknown): Self;
  /** `-`: elementwise for a Vector; the difference of the counts of each symbol for Symbols.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).sub(new Vector([3, 4])).repr(); //=> "Vector((-2, -2))"
   * new Symbols("aab").sub("abc").repr(); //=> "Symbols('a - c')"
   * ```
   */
  sub(other: unknown): Self;
  /** `*`: elementwise for a Vector; for Symbols, multiplies every count by an int.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).mul(new Vector([3, 4])).repr(); //=> "Vector((3, 8))"
   * new Symbols("aab").mul(2).repr(); //=> "Symbols('aaaabb')"
   * ```
   */
  mul(other: unknown): Self;
  /** `/`, elementwise for a Vector (Symbols have no `/`: `TypeError`).
   *
   * @example
   * ```ts
   * new Vector([1, 2]).truediv(2).repr(); //=> "Vector((0.5, 1.0))"
   * ```
   */
  truediv(other: unknown): Self;
  /** `//`: elementwise for a Vector. For Symbols, by an int it divides the counts (rounding down), by anything else
   * it is `countSubset` (an int).
   *
   * @example
   * ```ts
   * new Vector([7, -7]).floordiv(2).repr(); //=> "Vector((3, -4))"
   * new Symbols("aaab").floordiv(2).repr(); //=> "Symbols('a')"
   * new Symbols("aaaabb").floordiv("ab"); //=> 2
   * ```
   */
  floordiv(other: unknown): Self;
  /** `%`: elementwise for a Vector; for Symbols, every count modulo an int.
   *
   * @example
   * ```ts
   * new Vector([7, -7]).mod(3).repr(); //=> "Vector((1, 2))"
   * new Symbols("aaab").mod(2).repr(); //=> "Symbols('ab')"
   * ```
   */
  mod(other: unknown): Self;
  /** `**`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([2, 3]).pow(2).repr(); //=> "Vector((4, 9))"
   * ```
   */
  pow(other: unknown): Self;
  /** `<<`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).lshift(3).repr(); //=> "Vector((8, 16))"
   * ```
   */
  lshift(other: unknown): Self;
  /** `>>`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([8, 16]).rshift(3).repr(); //=> "Vector((1, 2))"
   * ```
   */
  rshift(other: unknown): Self;
  /** `&`: elementwise for a Vector; the min count of each symbol for Symbols (`intersection`).
   *
   * @example
   * ```ts
   * new Vector([1, 2]).and(new Vector([3, 4])).repr(); //=> "Vector((1, 0))"
   * new Symbols("aab").and("abc").repr(); //=> "Symbols('ab')"
   * ```
   */
  and(other: unknown): Self;
  /** `|`: elementwise for a Vector; the max count of each symbol for Symbols (`union`).
   *
   * @example
   * ```ts
   * new Vector([1, 2]).or(new Vector([3, 4])).repr(); //=> "Vector((3, 6))"
   * new Symbols("aab").or("abc").repr(); //=> "Symbols('aabc')"
   * ```
   */
  or(other: unknown): Self;
  /** `^`: elementwise for a Vector; the absolute difference of the counts for Symbols (`symmetricDifference`).
   *
   * @example
   * ```ts
   * new Vector([1, 2]).xor(new Vector([3, 4])).repr(); //=> "Vector((2, 6))"
   * new Symbols("aab").xor("abc").repr(); //=> "Symbols('ac')"
   * ```
   */
  xor(other: unknown): Self;
  /** `<`: between Vectors, an elementwise Vector whose truth value is the lexicographic comparison; for Symbols
   * (only against Symbols), a bool comparing the count of each character in alphabetical order (not a proper subset
   * relation).
   *
   * @example
   * ```ts
   * String(new Vector([1, 5]).lt(new Vector([2, 0]))); //=> "Vector((True, False))"
   * truth(new Vector([1, 5]).lt(new Vector([2, 0]))); //=> true
   * new Symbols("a").lt(new Symbols("ab")); //=> true
   * ```
   */
  lt(other: unknown): Outcome;
  /** `<=`: between Vectors, elementwise with a lexicographic truth value; for Symbols, `issubset` (a bool).
   *
   * @example
   * ```ts
   * String(new Vector([1, 2]).le(new Vector([1, 3]))); //=> "Vector((True, True))"
   * new Symbols("ab").le("aab"); //=> true
   * ```
   */
  le(other: unknown): Outcome;
  /** `>`: between Vectors, elementwise with a lexicographic truth value; for Symbols (only against Symbols), a
   * lexicographic bool.
   *
   * @example
   * ```ts
   * String(new Vector([1, 2]).gt(new Vector([0, 3]))); //=> "Vector((True, False))"
   * new Symbols("a").gt(new Symbols("bb")); //=> true
   * ```
   */
  gt(other: unknown): Outcome;
  /** `>=`: between Vectors, elementwise with a lexicographic truth value; for Symbols, `issuperset` (a bool).
   *
   * @example
   * ```ts
   * String(new Vector([1, 2]).ge(new Vector([1, 3]))); //=> "Vector((True, False))"
   * new Symbols("aab").ge("ab"); //=> true
   * ```
   */
  ge(other: unknown): Outcome;
  /** `==`: between Vectors, elementwise with the truth value of the whole comparison (against a non-vector it is
   * elementwise with truth value `false`); for Symbols, a bool (all counts equal).
   *
   * @example
   * ```ts
   * truth(new Vector([1, 2]).eq(new Vector([1, 2]))); //=> true
   * String(new Vector([1, 2]).eq(1)); //=> "Vector((True, False))"
   * new Symbols("ab").eq("ba"); //=> true
   * ```
   */
  eq(other: unknown): Outcome;
  /** `!=`: the opposite of `eq` (against a non-vector, elementwise with truth value `true`).
   *
   * @example
   * ```ts
   * truth(new Vector([1, 2]).ne(new Vector([1, 3]))); //=> true
   * new Symbols("ab").ne("abc"); //=> true
   * ```
   */
  ne(other: unknown): Outcome;
  /** Unary `-`: elementwise for a Vector; reverses the sign of every count for Symbols.
   *
   * @example
   * ```ts
   * new Vector([1, -2]).neg().repr(); //=> "Vector((-1, 2))"
   * new Symbols("a-b").neg().repr(); //=> "Symbols('b - a')"
   * ```
   */
  neg(): Self;
  /** Unary `+`: elementwise for a Vector; drops every negative count for Symbols.
   *
   * @example
   * ```ts
   * new Vector([1, -2]).pos().repr(); //=> "Vector((1, -2))"
   * new Symbols("a-b").pos().repr(); //=> "Symbols('a')"
   * ```
   */
  pos(): Self;
  /** `~`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([1, -2]).invert().repr(); //=> "Vector((-2, 1))"
   * ```
   */
  invert(): Self;
  /** `abs()`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([1, -2]).abs().repr(); //=> "Vector((1, 2))"
   * ```
   */
  abs(): Self;
  /** `math.trunc()`, elementwise for a Vector: rounds towards zero.
   *
   * @example
   * ```ts
   * new Vector([1.5, -2.5]).trunc().repr(); //=> "Vector((1, -2))"
   * ```
   */
  trunc(): Self;
  /** `math.floor()`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([1.5, -2.5]).floor().repr(); //=> "Vector((1, -3))"
   * ```
   */
  floor(): Self;
  /** `math.ceil()`, elementwise for a Vector.
   *
   * @example
   * ```ts
   * new Vector([1.5, -2.5]).ceil().repr(); //=> "Vector((2, -2))"
   * ```
   */
  ceil(): Self;
}

const BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor"] as const;
const COMPARE = ["lt", "le", "gt", "ge", "eq", "ne"] as const;
const UNARY = ["neg", "pos", "invert", "abs", "trunc", "floor", "ceil"] as const;

export interface Vector extends ValueOperators<Vector> {}
/** `icepool.Vector`: an immutable tuple-like outcome that applies most operators elementwise; comparisons give
 * vectors with a truth value (`truth(v)`).
 *
 * A binary operator with another Vector applies to each pair of elements (both must have the same length, else
 * `IndexError`); any other operand is broadcast to each element. The comparisons (`lt`, `le`, `gt`, `ge`, `eq`,
 * `ne`) are elementwise too, and between Vectors the result also has a truth value: the lexicographic comparison of
 * the two. Between Vectors of different lengths, a comparison gives only that truth value (a `VectorWithTruthOnly`).
 * Use `vectorize` to make dice with Vector outcomes, e.g. to track several totals at once.
 *
 * @example
 * ```ts
 * new Vector([1, 2]).add(new Vector([3, 4])).repr(); //=> "Vector((4, 6))"
 * new Vector([1, 2]).mul(10).repr(); //=> "Vector((10, 20))"
 * // 2d6 tracking (total, number of sixes).
 * const roll = d(6).map((x: number) => new Vector([x, x === 6 ? 1 : 0]));
 * roll.add(roll).probability(new Vector([12, 2])).toString(); //=> "1/36"
 * ```
 */
export class Vector implements Iterable<Outcome> {
  /** The elements; `null` for a `VectorWithTruthOnly` (a comparison between vectors of different lengths). */
  readonly items: Outcome[] | null;
  /** Whether it has a truth value: it is the result of a comparison with another Vector (or of `eq`/`ne`). */
  readonly hasTruth: boolean;
  /** The truth value when `hasTruth` (else `null`); `truth(v)` reads it, and throws `TypeError` without one. */
  readonly truthValue: Outcome;

  /** `Vector(items)`, without a truth value. */
  constructor(items: Iterable<Outcome>) {
    this.items = Array.from(items);
    this.hasTruth = false;
    this.truthValue = null;
  }
  /** A vector from the core: `items` null for a `VectorWithTruthOnly`.
   *
   * @example
   * ```ts
   * Vector.fromCore([1, 2], false, null).repr(); //=> "Vector((1, 2))"
   * Vector.fromCore(null, true, true).repr(); //=> "VectorWithTruthOnly(True)"
   * ```
   */
  static fromCore(items: Outcome[] | null, hasTruth: boolean, truthValue: Outcome): Vector {
    const v = new Vector([]);
    Object.assign(v, { items, hasTruth, truthValue });
    return v;
  }
  /** `len(v)`: the number of elements (0 for a `VectorWithTruthOnly`). */
  get length(): number {
    return this.items?.length ?? 0;
  }
  /** Iterates over the elements, so that `[...v]` is an array of them. */
  [Symbol.iterator](): Iterator<Outcome> {
    return (this.items ?? [])[Symbol.iterator]();
  }
  /** `truediv`: `/`, elementwise.
   *
   * @example
   * ```ts
   * new Vector([1, 3]).div(2).repr(); //=> "Vector((0.5, 1.5))"
   * ```
   */
  div(other: unknown): Vector {
    return this.truediv(other);
  }
  /** `round(v, ndigits)`, elementwise (Python's rounding: halves to even). `ndigits` `null` rounds to ints.
   *
   * @example
   * ```ts
   * new Vector([1.5, 2.5]).round().repr(); //=> "Vector((2, 2))"
   * new Vector([1.25, 2.5]).round(1).repr(); //=> "Vector((1.2, 2.5))"
   * ```
   */
  round(ndigits: Int | null = null): Vector {
    return round(this, ndigits) as Vector;
  }
  /** A new Vector with `x` added at the end.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).append(3).repr(); //=> "Vector((1, 2, 3))"
   * ```
   */
  append(x: Outcome): Vector {
    return call("vector_method", { v: this, name: "append", arg: x }) as Vector;
  }
  /** A new Vector with the elements of `other` added at the end.
   *
   * @example
   * ```ts
   * new Vector([1, 2]).concatenate([5, 6]).repr(); //=> "Vector((1, 2, 5, 6))"
   * ```
   */
  concatenate(other: Iterable<Outcome>): Vector {
    return call("vector_method", { v: this, name: "concatenate", arg: other }) as Vector;
  }
  /** Python's `str(v)`, e.g. `Vector((1, 2))`: the same as `repr()`, since a tuple's `str` shows its elements' repr.
   *
   * @example
   * ```ts
   * String(new Vector([1, "a"])); //=> "Vector((1, 'a'))"
   * ```
   */
  toString(): string {
    return call("vector_method", { v: this, name: "str", arg: null }) as string;
  }
  /** Python's `repr(v)`, e.g. `Vector((1, 'a'))`.
   *
   * @example
   * ```ts
   * new Vector([1, "a"]).repr(); //=> "Vector((1, 'a'))"
   * ```
   */
  repr(): string {
    return call("vector_method", { v: this, name: "repr", arg: null }) as string;
  }
}

export interface Symbols extends ValueOperators<Symbols> {}
/** `icepool.Symbols`: EXPERIMENTAL. An immutable multiset of single characters, from a string such as "aab-c".
 * Spaces, dashes and underscores cannot be symbols. Counts may be negative: in the string, the characters after a
 * minus sign (optionally surrounded by whitespace) count negatively, so `"a - b"` has one positive a and one
 * negative b.
 *
 * Operations (`l` and `r` the counts of a character on each side):
 *
 * | Operation                          | Count / notes                   |
 * |:-----------------------------------|:--------------------------------|
 * | `additiveUnion`, `add`             | `l + r`                         |
 * | `difference`, `sub`                | `l - r`                         |
 * | `intersection`, `and`              | `min(l, r)`                     |
 * | `union`, `or`                      | `max(l, r)`                     |
 * | `symmetricDifference`, `xor`       | `abs(l - r)`                    |
 * | `multiplyCounts`, `mul`            | `count * n`                     |
 * | `divideCounts`, `floordiv`         | `count // n`                    |
 * | `issubset`, `le`                   | all counts l <= r               |
 * | `issuperset`, `ge`                 | all counts l >= r               |
 * | `eq`                               | all counts l == r               |
 * | `ne`                               | any count l != r                |
 * | `pos`                              | drop all negative counts        |
 * | `neg`                              | reverses the sign of all counts |
 *
 * `lt` and `gt` are lexicographic orderings rather than subset relations: they compare the count of each character
 * in alphabetical order (`'a' > ''`, `'a' > 'bb'`, `'-a' < 'bb'`, `'a' < 'ab'`). Binary operators other than `mul`
 * and `floordiv` convert the other argument with the constructor, so a string will do.
 *
 * A character that is not allowed, or more than one dash, is a `ValueError` once the core reads the string.
 *
 * @example
 * ```ts
 * new Symbols("aab").add("abc").repr(); //=> "Symbols('aaabbc')"
 * new Symbols("aab").get("a"); //=> 2
 * // 3d6 where 5 and 6 are successes ("s"): the chance of exactly two.
 * const hit = d(6).map((x: number) => new Symbols(x >= 5 ? "s" : ""));
 * hit.add(hit).add(hit).probability(new Symbols("ss")).toString(); //=> "2/9"
 * new Symbols("a--b").repr(); //=> throws ValueError
 * ```
 */
export class Symbols {
  /** The string it was made from, or Python's `str()` when the core made it (`null` if that raises: non-int
   * counts). Python's `str()` lists the characters in ascending order, negative ones after ` - `. */
  readonly str: string | null;
  /** The dict's `[char, count]` items in its order, once the core made it (`null` for one made from a string). */
  readonly items: readonly (readonly [string, Int])[] | null;

  /** `Symbols(str)` from a string; the core parses it when it is first used (so a bad string throws then). */
  constructor(str: string) {
    this.str = String(str);
    this.items = null;
  }
  /** A Symbols from the core.
   *
   * @example
   * ```ts
   * Symbols.fromCore([["a", 2], ["b", 1]], "aab").repr(); //=> "Symbols('aab')"
   * ```
   */
  static fromCore(items: readonly (readonly [string, Int])[], str: string | null): Symbols {
    const s = new Symbols("");
    Object.assign(s, { items, str });
    return s;
  }
  /** `Symbols(x)` from anything Python takes: a string, an iterable of characters, or a Map of characters to counts.
   *
   * @example
   * ```ts
   * Symbols.from(["b", "a", "b"]).repr(); //=> "Symbols('abb')"
   * Symbols.from(new Map([["a", 2], ["b", -1]])).repr(); //=> "Symbols('aa - b')"
   * ```
   */
  static from(x: string | readonly string[] | Map<string, Int>): Symbols {
    return call("symbols", { x: x instanceof Map ? new SymbolsMapping(x) : x }) as Symbols;
  }
  /** Calls the method `name` of Python's Symbols (snake_case, or `getitem` for `symbols[arg]`), with one argument.
   *
   * @example
   * ```ts
   * (new Symbols("aabc").method("getitem", "ab") as Symbols).repr(); //=> "Symbols('aab')"
   * new Symbols("aab").method("size"); //=> 3
   * ```
   */
  method(name: string, arg: unknown = null): unknown {
    return call("symbols_method", { s: this, name, arg });
  }
  /** `issubset(other)`: whether `this` is a subset of the other (every count at most the other's). Same as `le`.
   * Note that `lt` and `gt` are lexicographic orderings, not proper subset relations.
   *
   * @example
   * ```ts
   * new Symbols("aab").issubset("aabc"); //=> true
   * new Symbols("aab").issubset("abc"); //=> false
   * ```
   */
  issubset(other: unknown): boolean {
    return this.method("issubset", other) as boolean;
  }
  /** `issuperset(other)`: whether `this` is a superset of the other (every count at least the other's). Same as
   * `ge`. Note that `lt` and `gt` are lexicographic orderings, not proper subset relations.
   *
   * @example
   * ```ts
   * new Symbols("aab").issuperset("ab"); //=> true
   * ```
   */
  issuperset(other: unknown): boolean {
    return this.method("issuperset", other) as boolean;
  }
  /** `isdisjoint(other)`: meant to be whether `this` has no positive elements in common with the other, but
   * Python's returns whether some symbol is positive in both (the opposite), and the port copies that
   * (DIVERGENCES.md).
   *
   * @example
   * ```ts
   * new Symbols("aab").isdisjoint("bc"); //=> true
   * new Symbols("aab").isdisjoint("cd"); //=> false
   * ```
   */
  isdisjoint(other: unknown): boolean {
    return this.method("isdisjoint", other) as boolean;
  }
  /** `count_subset(divisor, empty_divisor=None)`: the number of times the divisor is contained in this multiset.
   *
   * @param emptyDivisor - the result for an empty divisor; `null` (the default) throws `ZeroDivisionError` instead.
   *
   * @example
   * ```ts
   * new Symbols("aaaabb").countSubset("ab"); //=> 2
   * new Symbols("aab").countSubset(""); //=> throws ZeroDivisionError
   * new Symbols("aab").countSubset("", { emptyDivisor: 0 }); //=> 0
   * ```
   */
  countSubset(divisor: unknown, { emptyDivisor = null }: { emptyDivisor?: Int | null } = {}): Int {
    return this.method("count_subset", [divisor, emptyDivisor]) as Int;
  }
  /** Whether any counts are negative.
   *
   * @example
   * ```ts
   * new Symbols("a-b").hasNegativeCounts(); //=> true
   * ```
   */
  hasNegativeCounts(): boolean {
    return this.method("has_negative_counts") as boolean;
  }
  /** The total number of elements: the sum of the counts, negative ones included.
   *
   * @example
   * ```ts
   * new Symbols("aab").size(); //=> 3
   * new Symbols("aa-b").size(); //=> 1
   * ```
   */
  size(): Int {
    return this.method("size") as Int;
  }
  /** Multiplies all counts by an integer (`mul`).
   *
   * @example
   * ```ts
   * new Symbols("aab").multiplyCounts(3).repr(); //=> "Symbols('aaaaaabbb')"
   * ```
   */
  multiplyCounts(n: Int): Symbols {
    return this.method("multiply_counts", n) as Symbols;
  }
  /** Divides all counts by an integer, rounding down.
   *
   * @example
   * ```ts
   * new Symbols("aaab").divideCounts(2).repr(); //=> "Symbols('a')"
   * ```
   */
  divideCounts(n: unknown): Symbols {
    return this.method("divide_counts", n) as Symbols;
  }
  /** Every count modulo an integer (`mod`).
   *
   * @example
   * ```ts
   * new Symbols("aaab").moduloCounts(2).repr(); //=> "Symbols('ab')"
   * ```
   */
  moduloCounts(n: Int): Symbols {
    return this.method("modulo_counts", n) as Symbols;
  }
  /** `additive_union(*args)`: the sum of counts of each symbol. On the oracle's Python (3.13) every call raises
   * `TypeError` (it calls `functools.reduce` with a keyword argument, which only Python 3.14 takes), and the port
   * copies that (DIVERGENCES.md): use `add` instead.
   *
   * @example
   * ```ts
   * new Symbols("a").additiveUnion("b", "b"); //=> throws TypeError
   * new Symbols("a").add("b").add("b").repr(); //=> "Symbols('abb')"
   * ```
   */
  additiveUnion(...others: unknown[]): Symbols {
    return this.method("additive_union", others) as Symbols;
  }
  /** `difference(*args)`: the difference between the counts of each symbol. Like `additiveUnion`, every call raises
   * `TypeError` on the oracle's Python 3.13, and so here: use `sub` instead.
   *
   * @example
   * ```ts
   * new Symbols("abc").difference("a", "b"); //=> throws TypeError
   * new Symbols("abc").sub("a").sub("b").repr(); //=> "Symbols('c')"
   * ```
   */
  difference(...others: unknown[]): Symbols {
    return this.method("difference", others) as Symbols;
  }
  /** `intersection(*args)`: the min count of each symbol. Like `additiveUnion`, every call raises `TypeError` on the
   * oracle's Python 3.13, and so here: use `and` instead.
   *
   * @example
   * ```ts
   * new Symbols("aab").intersection("ab"); //=> throws TypeError
   * new Symbols("aab").and("ab").repr(); //=> "Symbols('ab')"
   * ```
   */
  intersection(...others: unknown[]): Symbols {
    return this.method("intersection", others) as Symbols;
  }
  /** `union(*args)`: the max count of each symbol. Like `additiveUnion`, every call raises `TypeError` on the
   * oracle's Python 3.13, and so here: use `or` instead.
   *
   * @example
   * ```ts
   * new Symbols("aab").union("abc"); //=> throws TypeError
   * new Symbols("aab").or("abc").repr(); //=> "Symbols('aabc')"
   * ```
   */
  union(...others: unknown[]): Symbols {
    return this.method("union", others) as Symbols;
  }
  /** The absolute difference in symbol counts between the two sets (`xor`).
   *
   * @example
   * ```ts
   * new Symbols("aab").symmetricDifference("abc").repr(); //=> "Symbols('ac')"
   * ```
   */
  symmetricDifference(other: unknown): Symbols {
    return this.method("symmetric_difference", other) as Symbols;
  }
  /** The count of a character (`symbols[c]`), 0 if absent. With several characters, Python returns a Symbols with
   * only those (`method("getitem", "ab")` is the typed way to ask for that).
   *
   * @example
   * ```ts
   * new Symbols("aab").get("a"); //=> 2
   * new Symbols("aab").get("z"); //=> 0
   * ```
   */
  get(c: string): Int {
    return this.method("getitem", c) as Int;
  }
  /** The characters, in ascending order.
   *
   * @example
   * ```ts
   * new Symbols("baa").keys(); //=> ["a", "b"]
   * ```
   */
  keys(): string[] {
    return this.method("keys") as string[];
  }
  /** The counts, in the order of `keys()`.
   *
   * @example
   * ```ts
   * new Symbols("baa").values(); //=> [2, 1]
   * ```
   */
  values(): Int[] {
    return this.method("values") as Int[];
  }
  /** The `str` field: Python's `str()` (all symbols in unary form, ascending, negative ones after ` - `) for a
   * Symbols the core made; for one made with `new Symbols(string)`, the string as given.
   *
   * @example
   * ```ts
   * String(new Symbols("ba").add("")); //=> "ab"
   * String(Symbols.from(new Map([["a", 1], ["b", -2]]))); //=> "a - bb"
   * ```
   */
  toString(): string {
    return this.str ?? "";
  }
  /** Python's `repr()`, e.g. `Symbols('aab')`.
   *
   * @example
   * ```ts
   * new Symbols("ba").repr(); //=> "Symbols('ab')"
   * ```
   */
  repr(): string {
    return this.method("repr") as string;
  }
}

for (const cls of [Vector, Symbols]) {
  const proto = cls.prototype as unknown as Record<string, unknown>;
  for (const op of [...BINARY, ...COMPARE]) {
    proto[op] = function (this: Vector | Symbols, other: unknown) {
      return binary(op, this, other);
    };
  }
  for (const op of UNARY) {
    proto[op] = function (this: Vector | Symbols) {
      return unary(op, this);
    };
  }
}

/** A Map passed to `Symbols.from`: Python's dict of counts. */
export class SymbolsMapping {
  readonly map: Map<string, Int>;
  constructor(map: Map<string, Int>) {
    this.map = map;
  }
}

/** Python's `bool(x)` of a Vector (from a comparison) or Symbols. */
export function valueTruth(x: Vector | Symbols): boolean {
  return wasm((w) => w.valueTruth(x));
}

/** A value with nothing in it the core cannot take as an outcome (no die, Again, sentinel, list, function...). */
export function isPlainValue(x: unknown): boolean {
  switch (typeof x) {
    case "number":
    case "bigint":
    case "string":
    case "boolean":
      return true;
    case "object":
      if (x === null) return true;
      if (Array.isArray(x)) return !(x instanceof List) && x.every(isPlainValue);
      return x instanceof Float || x instanceof Fraction || x instanceof Complex || x instanceof Vector || x instanceof Symbols;
  }
  return false;
}
