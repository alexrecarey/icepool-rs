// `icepool.Die` (population/die.py, population/base.py): a die of the core (`WasmDie`), with Python's argument
// juggling done here. Arithmetic is methods (`d(6).add(1)`, `die.gt(10)`), keyword arguments are a trailing
// options object (`die.reroll([1], { depth: 2 })`), quantities are BigInts.

import type { WasmDie } from "../pkg-node/icepool_wasm.js";
import type { AgainExpression } from "./again.js";
import type { MultisetExpression, Pool } from "./multiset.js";
import { IcepoolError } from "./errors.js";
import { ENCODE, Handle, call, registerKind } from "./rpc.js";
import { map, mapAndTime, meanTimeToAbsorb, ops } from "./functions.js";
import {
  Break,
  Complex,
  Float,
  Fraction,
  List,
  Reroll,
  Restart,
  Special,
  Symbols,
  Vector,
  isPlainValue,
  type Ellipsis,
  type Int,
  type Outcome,
} from "./values.js";
import { raw, wasm } from "./wasm.js";

// --- Argument types -----------------------------------------------------------------------------------------------

/** What dice operators and constructors accept on the other side: an outcome, a die, or an Again expression. A
 * plain outcome is converted to a die that always rolls it (Python's `implicit_convert_to_die`).
 *
 * @example
 * ```ts
 * d(6).add(1).outcomes(); //=> [2, 3, 4, 5, 6, 7]
 * d(6).add(d(6)).probability(7).toString(); //=> "1/6"
 * ```
 */
export type Operand = Outcome | Die | AgainExpression;

/** A JS function the core calls back. Its parameters are typed by the caller (`(x: number) => ...`); Python's
 * `star` is inferred from its `length`, as Python counts positional parameters.
 *
 * @example
 * ```ts
 * // One parameter: called with each outcome.
 * d(6).map((x: number) => x * 2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
 * // Two parameters (`length` 2): a tuple outcome is unpacked into them, as Python's `star`.
 * new Die([[1, 2], [3, 4]]).map((a: number, b: number) => a + b).outcomes(); //=> [3, 7]
 * ```
 */
export type Callback<R = unknown> = (...args: never[]) => R;

/** A selection of outcomes (`reroll`, `filter`, `split`, `explode`, ...): a predicate or the outcomes themselves
 * (a Map: its keys, as Python's dict).
 *
 * @example
 * ```ts
 * d(6).split([5, 6])[0].outcomes(); //=> [5, 6]
 * d(6).split((x: number) => x > 4)[0].outcomes(); //=> [5, 6]
 * ```
 */
export type Which = Callback | readonly Outcome[] | Map<unknown, unknown> | null;

/** A comparison, as Python spells it (`keepCounts`, `sortPair`, `versusAll`, ...).
 *
 * @example
 * ```ts
 * // The dice of 3d6 that are part of a pair or better: none, two or three.
 * d(6).pool(3).keepCounts(">=", 2).size().items(); //=> [[0, 120n], [2, 90n], [3, 6n]]
 * ```
 */
export type Comparison = "==" | "!=" | "<=" | "<" | ">=" | ">";

/** What a `map` function may return: an outcome, a die (its outcomes share the original outcome's weight),
 * `Reroll` (drops the outcome), `Restart` or `Break` (for `repeat`), an Again expression, or a tuple of these.
 *
 * @example
 * ```ts
 * d(6).map((x: number) => (x === 1 ? Reroll : x)).outcomes(); //=> [2, 3, 4, 5, 6]
 * d(6).map((x: number) => (x === 6 ? d(6).add(6) : x)).denominator(); //=> 36n
 * ```
 */
export type MapResult = Operand | typeof Reroll | typeof Restart | Break | readonly MapResult[];

/** What `map` takes: a function, or a Map (Python's dict: `mapping.get(outcome, outcome)`, keys compared with
 * Python's `==`, so `1`, `1.0` and `true` are one key).
 *
 * @example
 * ```ts
 * d(6).map((x: number) => x * 2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
 * // Outcomes not in the Map stay as they are.
 * d(6).map(new Map([[6, 0]])).outcomes(); //=> [0, 1, 2, 3, 4, 5]
 * ```
 */
export type Repl = Callback<MapResult> | Map<unknown, MapResult>;

/** The keyword arguments of `Die(...)` for `Again` outcomes (population/again.py): how many times to roll again.
 * At most one of `againCount` and `againDepth` may be given.
 *
 * @example
 * ```ts
 * // A d6 that explodes once on a 6.
 * const once = new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 1 });
 * once.outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
 * // As before, but rerolling a second 6 instead of counting it as 0.
 * new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 1, againEnd: Reroll }).maxOutcome(); //=> 11
 * ```
 */
export interface AgainOptions {
  /** `again_count`: count mode, at most this many extra rolls in total (additive Again expressions only). */
  againCount?: Int | null;
  /** `again_depth`: depth mode, Again nested at most this deep; the default (1) when neither option is given. */
  againDepth?: Int | null;
  /** `again_end`: what an `Again` beyond the limit becomes: an outcome, `Reroll`, `Restart` (count mode only), or
   * zero when null. */
  againEnd?: Operand | typeof Reroll | typeof Restart | null;
}

/** What `new Die(...)` takes as outcomes: a sequence of outcomes, dice, `Reroll` or Again expressions, a Map of
 * outcomes to quantities, or a die (whose quantities multiply the times).
 *
 * @example
 * ```ts
 * new Die([1, 2, 2, 3]).items(); //=> [[1, 1n], [2, 2n], [3, 1n]]
 * new Die(new Map([[1, 2n], [2, 1n]])).items(); //=> [[1, 2n], [2, 1n]]
 * ```
 */
export type DieOutcomes = readonly (MapResult | Special)[] | Map<unknown, Int> | Die;

// --- Helpers ------------------------------------------------------------------------------------------------------

/** A plain object: keyword arguments (no value the core takes is one). */
export const isOptions = (x: unknown): x is Record<string, unknown> =>
  x !== null && typeof x === "object" && Object.getPrototypeOf(x) === Object.prototype;

/** Splits a trailing options object off a rest-parameter list. */
export function opts<O extends object>(args: readonly unknown[]): [unknown[], Partial<O>] {
  const last = args[args.length - 1];
  return isOptions(last) ? [args.slice(0, -1), last as Partial<O>] : [[...args], {}];
}

/** An array as a Python list (the bridge's way of saying "a sequence argument"). */
export const list = <T>(xs: T): T => (xs instanceof List ? xs : Array.isArray(xs) ? (List.from(xs) as T) : xs);

const isInt = (x: unknown): x is Int => typeof x === "bigint" || Number.isInteger(x);

/** `Infinity` as Python's `'inf'` (a depth or a count without a limit). */
const inf = <T>(x: T): T | "inf" => (x === Infinity ? "inf" : x);

/** `implicit_convert_to_die(x)` where the core can do it directly: a die as it is, `Reroll` the empty die, a plain
 * value `Die([x])`. `undefined` for the rest (an Again expression, a tuple holding dice, ...), which the caller
 * hands to the bridge. */
function nativeDie(x: unknown): WasmDie | undefined {
  if (x instanceof Die) return x.w;
  // A Map is Python's dict, which `Die([x])` cannot hash.
  if (x instanceof Map) throw new IcepoolError("Unhashable", "TypeError", "unhashable type: 'dict'");
  if (x === Reroll || x === Restart) return wasm((w) => w.WasmDie.empty());
  if (isPlainValue(x)) return wasm((w) => w.WasmDie.single(x));
  return undefined;
}

/** Python's `bool(x)` of what a JS callback returned. */
export function pyBool(x: unknown): boolean {
  switch (typeof x) {
    case "undefined":
      return false;
    case "boolean":
      return x;
    case "number":
      return x !== 0; // NaN is true, as in Python
    case "bigint":
      return x !== 0n;
    case "string":
      return x.length > 0;
  }
  if (x === null) return false;
  if (Array.isArray(x)) return x.length > 0;
  if (x instanceof Float) return x.value !== 0;
  if (x instanceof Fraction) return x.numerator !== 0n;
  if (x instanceof Complex) return x.re !== 0 || x.im !== 0;
  if (x instanceof DieWithTruth) return x.truth();
  // A Vector or Symbols has its own truth value; a plain die raises, an Again expression has one too: the core
  // says.
  if (x instanceof Vector || x instanceof Symbols) return wasm((w) => w.valueTruth(x));
  if (x instanceof Die || x instanceof Handle) return call("truth", { x }) as boolean;
  return true; // any other object, as Python's default
}

/** `*x` for an outcome, where JS can unpack it as Python would (a tuple, a str); `undefined` otherwise. */
export function unpacked(x: unknown): unknown[] | undefined {
  if (Array.isArray(x)) return x;
  if (typeof x === "string") return [...x];
  return undefined;
}

/** A reroll depth as the core takes it: a non-negative int, or `null` for no limit (Infinity, Python's 'inf');
 * `undefined` for anything else, which the caller hands to the bridge (for Python's error). */
function depthArg(depth: unknown): number | null | undefined {
  if (depth === Infinity || depth === "inf") return null;
  if (isInt(depth) && depth >= 0 && depth < 2 ** 32) return Number(depth);
  return undefined;
}

// --- Die ----------------------------------------------------------------------------------------------------------

type BinaryName =
  | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
  | "lt" | "le" | "gt" | "ge";
type UnaryName = "neg" | "pos" | "invert" | "abs" | "trunc" | "floor" | "ceil";

/** The core's operator for each name. */
const BINARY: Record<BinaryName, keyof typeof import("../pkg-node/icepool_wasm.js").BinaryOp> = {
  add: "Add", sub: "Sub", mul: "Mul", truediv: "TrueDiv", floordiv: "FloorDiv", mod: "Mod", pow: "Pow",
  lshift: "LShift", rshift: "RShift", and: "And", or: "Or", xor: "Xor", lt: "Lt", le: "Le", gt: "Gt", ge: "Ge",
};
const UNARY: Record<UnaryName, keyof typeof import("../pkg-node/icepool_wasm.js").UnaryOp> = {
  neg: "Neg", pos: "Pos", invert: "Invert", abs: "Abs", trunc: "Trunc", floor: "Floor", ceil: "Ceil",
};

/** `icepool.Die`: sampling with replacement; quantities (BigInts) are weights.
 *
 * Dice are immutable: methods return a new die rather than changing this one. Operators are methods
 * (`die.add(1)`, `die.gt(3)`), keyword arguments a trailing options object. A die can also be empty (no outcomes),
 * though that has little use other than as a sentinel.
 *
 * `new Die(outcomes, times?, { againCount, againDepth, againEnd }?)` takes the faces as an array (duplicates add
 * quantity) or a Map of outcomes to quantities; a face may itself be a die, flattened into the result with the
 * quantity shared among its outcomes, `Reroll` (dropped), or an Again expression. Don't confuse it with `d()`:
 * `new Die([6])` always rolls 6, `d(6)` is a d6. Each die holds WASM memory: free it with `free()` when done with
 * it, or leave it to the garbage collector.
 *
 * @example
 * ```ts
 * // Some ways of making a d6.
 * d(6).outcomes(); //=> [1, 2, 3, 4, 5, 6]
 * new Die([1, 2, 3, 4, 5, 6]).equals(d(6)); //=> true
 * new Die([d(3), d(3).add(3)]).equals(d(6)); //=> true
 * new Die(new Map([[1, 1n], [2, 1n], [3, 1n], [4, 1n], [5, 1n], [6, 1n]])).equals(d(6)); //=> true
 * // `times` multiplies each face's quantity.
 * new Die([1, 2, 3], [3, 2, 1]).items(); //=> [[1, 3n], [2, 2n], [3, 1n]]
 * // A d6 that explodes on a 6, once.
 * new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 1 }).maxOutcome(); //=> 12
 * ```
 */
export class Die<T extends Outcome = Outcome> implements Iterable<T>, Disposable {
  /** The core's die; `null` once freed. @internal */
  protected core: WasmDie | null = null;

  /** `Die(outcomes, times=1, *, again_count=None, again_depth=None, again_end=None)`.
   *
   * All quantities must be non-negative; outcomes with zero quantity are omitted. For a flat set of outcomes the
   * denominator is the sum of the quantities; if some faces are dice, the overall denominator is the smallest
   * that keeps the relative weights of the faces (`new Die([d(4), d(6)])` has denominator 24).
   * `new Die([die])` is that very die.
   *
   * @param outcomes - The faces: an array of outcomes, dice, `Reroll` or Again expressions (tuples holding dice
   *   are tupleized into a die of tuples), a Map of outcomes to quantities, or a die.
   * @param times - Multiplies the quantity of each face: one int for all, or one per face. May be left out in
   *   favour of the options.
   * @param options - How `Again` faces are rolled again; see {@link AgainOptions}.
   * @throws IcepoolError `ValueError` if both `againCount` and `againDepth` are given; `TypeError` for outcomes
   *   that cannot be sorted together (such as `null`).
   */
  constructor(outcomes: DieOutcomes, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions) {
    if (isOptions(times)) [times, options] = [undefined, times as AgainOptions];
    const made = makeDie(outcomes, times as Int | readonly Int[] | undefined, options ?? {});
    // `Die([die])` is that very die (a `DieWithTruth` keeps its type and truth value), as in Python.
    if (made !== undefined && !(made instanceof WasmDieBox)) return made as unknown as Die<T>;
    this.core = made === undefined ? null : made.die;
  }

  /** The core's die (throws once freed). @internal */
  get w(): WasmDie {
    if (this.core === null) throw new TypeError("icepool: this die was freed");
    return this.core;
  }

  /** Wraps a core die. @internal */
  static wrap(w: WasmDie): Die {
    const die = Object.create(Die.prototype) as Die;
    die.core = w;
    return die;
  }

  /** @internal */
  [ENCODE](temps: number[] | null): string {
    const w = this.w;
    const h = wasm((m) => m.dieHandle(w, undefined));
    if (temps === null) return `{"$take":${h}}`;
    temps.push(h);
    return `{"$h":${h}}`;
  }

  /** Frees the die's WASM memory now rather than when it is garbage-collected. Using it afterwards throws a
   * `TypeError`; freeing it again does nothing. Dice made from it (`die.add(1)`) are not affected.
   *
   * @example
   * ```ts
   * const d6 = d(6);
   * const plusOne = d6.add(1);
   * d6.free();
   * d6.mean(); //=> throws TypeError
   * plusOne.outcomes(); //=> [2, 3, 4, 5, 6, 7]
   * ```
   */
  free(): void {
    this.core?.free();
    this.core = null;
  }

  /** `free()`, for `using die = d(6);` (where the runtime supports `using`). */
  [Symbol.dispose](): void {
    this.free();
  }

  // --- Operators ------------------------------------------------------------------------------------------------

  /** `self <op> other`, natively where the other operand converts to a die directly. */
  private binary(name: BinaryName, other: unknown, reflected = false): Die {
    const o = nativeDie(other);
    if (o === undefined) return call(name, reflected ? { a: other, b: this } : { a: this, b: other }) as Die;
    const op = raw().BinaryOp[BINARY[name]];
    const w = this.w;
    return Die.wrap(wasm(() => (reflected ? o.binary(op, w) : w.binary(op, o))));
  }

  private unary(name: UnaryName): Die {
    const op = raw().UnaryOp[UNARY[name]];
    const w = this.w;
    return Die.wrap(wasm(() => w.unary(op)));
  }

  // Binary operators: the operator applied to every pair of outcomes of the two dice, rolled independently (an
  // outcome operand is a die that always rolls it). With an Again expression on the other side, the result is an
  // Again expression, as in Python.

  /** `self + other`.
   * @example
   * ```ts
   * d(6).add(1).outcomes(); //=> [2, 3, 4, 5, 6, 7]
   * ```
   */
  add(other: Operand): Die { return this.binary("add", other); }
  /** `self - other`.
   * @example
   * ```ts
   * d(6).sub(1).outcomes(); //=> [0, 1, 2, 3, 4, 5]
   * ```
   */
  sub(other: Operand): Die { return this.binary("sub", other); }
  /** `self * other`.
   * @example
   * ```ts
   * d(6).mul(2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
   * ```
   */
  mul(other: Operand): Die { return this.binary("mul", other); }
  /** `self / other`: true division, so ints become floats.
   * @example
   * ```ts
   * d(4).truediv(2).repr(); //=> "Die({0.5: 1, 1.0: 1, 1.5: 1, 2.0: 1})"
   * ```
   */
  truediv(other: Operand): Die { return this.binary("truediv", other); }
  /** `self / other`: the same as `truediv`.
   * @example
   * ```ts
   * d(4).div(2).equals(d(4).truediv(2)); //=> true
   * ```
   */
  div(other: Operand): Die { return this.binary("truediv", other); }
  /** `self // other`: floor division.
   * @example
   * ```ts
   * d(6).floordiv(2).items(); //=> [[0, 1n], [1, 2n], [2, 2n], [3, 1n]]
   * ```
   */
  floordiv(other: Operand): Die { return this.binary("floordiv", other); }
  /** `self % other`, with Python's sign convention (the result has the sign of `other`).
   * @example
   * ```ts
   * d(6).mod(3).items(); //=> [[0, 2n], [1, 2n], [2, 2n]]
   * ```
   */
  mod(other: Operand): Die { return this.binary("mod", other); }
  /** `self ** other`.
   * @example
   * ```ts
   * d(3).pow(2).outcomes(); //=> [1, 4, 9]
   * ```
   */
  pow(other: Operand): Die { return this.binary("pow", other); }
  /** `self << other`.
   * @example
   * ```ts
   * d(3).lshift(1).outcomes(); //=> [2, 4, 6]
   * ```
   */
  lshift(other: Operand): Die { return this.binary("lshift", other); }
  /** `self >> other`.
   * @example
   * ```ts
   * d(6).rshift(1).items(); //=> [[0, 1n], [1, 2n], [2, 2n], [3, 1n]]
   * ```
   */
  rshift(other: Operand): Die { return this.binary("rshift", other); }
  /** `self & other`: bitwise and (logical for bools).
   * @example
   * ```ts
   * d(6).and(1).items(); //=> [[0, 3n], [1, 3n]]
   * ```
   */
  and(other: Operand): Die { return this.binary("and", other); }
  /** `self | other`: bitwise or (logical for bools).
   * @example
   * ```ts
   * d(4).or(1).outcomes(); //=> [1, 3, 5]
   * ```
   */
  or(other: Operand): Die { return this.binary("or", other); }
  /** `self ^ other`: bitwise exclusive or (logical for bools).
   * @example
   * ```ts
   * d(4).xor(1).outcomes(); //=> [0, 2, 3, 5]
   * ```
   */
  xor(other: Operand): Die { return this.binary("xor", other); }

  // The reflected operators: `3 - die` is `die.rsub(3)`, the other operand converted to a die on the left.

  /** `other + self`.
   * @example
   * ```ts
   * d(6).radd(1).outcomes(); //=> [2, 3, 4, 5, 6, 7]
   * ```
   */
  radd(other: Operand): Die { return this.binary("add", other, true); }
  /** `other - self`.
   * @example
   * ```ts
   * d(6).rsub(7).outcomes(); //=> [1, 2, 3, 4, 5, 6]
   * ```
   */
  rsub(other: Operand): Die { return this.binary("sub", other, true); }
  /** `other * self`.
   * @example
   * ```ts
   * d(6).rmul(2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
   * ```
   */
  rmul(other: Operand): Die { return this.binary("mul", other, true); }
  /** `other / self`: true division.
   * @example
   * ```ts
   * d(4).rtruediv(12).repr(); //=> "Die({3.0: 1, 4.0: 1, 6.0: 1, 12.0: 1})"
   * ```
   */
  rtruediv(other: Operand): Die { return this.binary("truediv", other, true); }
  /** `other // self`: floor division.
   * @example
   * ```ts
   * d(4).rfloordiv(12).outcomes(); //=> [3, 4, 6, 12]
   * ```
   */
  rfloordiv(other: Operand): Die { return this.binary("floordiv", other, true); }
  /** `other % self`.
   * @example
   * ```ts
   * d(4).rmod(5).items(); //=> [[0, 1n], [1, 2n], [2, 1n]]
   * ```
   */
  rmod(other: Operand): Die { return this.binary("mod", other, true); }
  /** `other ** self`.
   * @example
   * ```ts
   * d(3).rpow(2).outcomes(); //=> [2, 4, 8]
   * ```
   */
  rpow(other: Operand): Die { return this.binary("pow", other, true); }
  /** `other << self`.
   * @example
   * ```ts
   * d(3).rlshift(1).outcomes(); //=> [2, 4, 8]
   * ```
   */
  rlshift(other: Operand): Die { return this.binary("lshift", other, true); }
  /** `other >> self`.
   * @example
   * ```ts
   * d(3).rrshift(8).outcomes(); //=> [1, 2, 4]
   * ```
   */
  rrshift(other: Operand): Die { return this.binary("rshift", other, true); }
  /** `other & self`.
   * @example
   * ```ts
   * d(3).rand(1).items(); //=> [[0, 1n], [1, 2n]]
   * ```
   */
  rand(other: Operand): Die { return this.binary("and", other, true); }
  /** `other | self`.
   * @example
   * ```ts
   * d(3).ror(4).outcomes(); //=> [5, 6, 7]
   * ```
   */
  ror(other: Operand): Die { return this.binary("or", other, true); }
  /** `other ^ self`.
   * @example
   * ```ts
   * d(3).rxor(1).outcomes(); //=> [0, 2, 3]
   * ```
   */
  rxor(other: Operand): Die { return this.binary("xor", other, true); }

  // Comparisons: dice of bools.

  /** `self < other`: a die of whether this die rolls lower.
   * @example
   * ```ts
   * d(6).lt(3).items(); //=> [[false, 4n], [true, 2n]]
   * ```
   */
  lt(other: Operand): Die { return this.binary("lt", other); }
  /** `self <= other`.
   * @example
   * ```ts
   * d(6).le(3).items(); //=> [[false, 3n], [true, 3n]]
   * ```
   */
  le(other: Operand): Die { return this.binary("le", other); }
  /** `self > other`.
   * @example
   * ```ts
   * d(6).gt(4).items(); //=> [[false, 4n], [true, 2n]]
   * ```
   */
  gt(other: Operand): Die { return this.binary("gt", other); }
  /** `self >= other`.
   * @example
   * ```ts
   * d(20).ge(11).probability(true).toString(); //=> "1/2"
   * ```
   */
  ge(other: Operand): Die { return this.binary("ge", other); }
  /** `self == other`: a die of bools with a truth value, `truth()`, which is whether the dice are the same
   * (`equals`; a die is never the same as a plain outcome).
   * @example
   * ```ts
   * d(6).eq(3).items(); //=> [[false, 5n], [true, 1n]]
   * d(6).eq(d(6)).truth(); //=> true
   * d(6).eq(6).truth(); //=> false
   * ```
   */
  eq(other: Operand): DieWithTruth { return this.equality(other, false); }
  /** `self != other`: as `eq`, with the opposite truth value.
   * @example
   * ```ts
   * d(6).ne(3).items(); //=> [[false, 1n], [true, 5n]]
   * d(6).ne(d(8)).truth(); //=> true
   * ```
   */
  ne(other: Operand): DieWithTruth { return this.equality(other, true); }

  private equality(other: unknown, ne: boolean): DieWithTruth {
    const o = nativeDie(other);
    const viaBridge = () => call(ne ? "ne" : "eq", { a: this, b: other }) as DieWithTruth;
    if (o === undefined) return viaBridge();
    const w = this.w;
    // The truth value compares with the operand as it was given (a die is never equal to a plain outcome).
    const same = other instanceof Die ? wasm(() => w.equals(other.w, false)) : false;
    let data: WasmDie;
    try {
      data = wasm(() => w.binary(ne ? raw().BinaryOp.Ne : raw().BinaryOp.Eq, o));
    } catch {
      // Python computes the data lazily: the bridge keeps the error for whoever asks for it, leaving the truth
      // value usable.
      return viaBridge();
    }
    return DieWithTruth.wrapTruth(data, same !== ne);
  }

  // Unary operators, applied to each outcome.

  /** `-self`.
   * @example
   * ```ts
   * d(6).neg().outcomes(); //=> [-6, -5, -4, -3, -2, -1]
   * ```
   */
  neg(): Die { return this.unary("neg"); }
  /** `+self`: each outcome unchanged, for numbers.
   * @example
   * ```ts
   * d(6).pos().equals(d(6)); //=> true
   * ```
   */
  pos(): Die { return this.unary("pos"); }
  /** `~self`: bitwise inversion, `-x - 1` for ints.
   * @example
   * ```ts
   * d(3).invert().outcomes(); //=> [-4, -3, -2]
   * ```
   */
  invert(): Die { return this.unary("invert"); }
  /** `abs(self)`.
   * @example
   * ```ts
   * d(6).sub(d(6)).abs().outcomes(); //=> [0, 1, 2, 3, 4, 5]
   * ```
   */
  abs(): Die { return this.unary("abs"); }
  /** `math.trunc(self)`: each outcome rounded toward zero. Not to be confused with {@link Die.truncate}.
   * @example
   * ```ts
   * d(6).neg().truediv(4).trunc().items(); //=> [[-1, 3n], [0, 3n]]
   * ```
   */
  trunc(): Die { return this.unary("trunc"); }
  /** `math.floor(self)`: each outcome rounded down.
   * @example
   * ```ts
   * d(6).neg().truediv(4).floor().items(); //=> [[-2, 2n], [-1, 4n]]
   * ```
   */
  floor(): Die { return this.unary("floor"); }
  /** `math.ceil(self)`: each outcome rounded up.
   * @example
   * ```ts
   * d(6).truediv(4).ceil().items(); //=> [[1, 4n], [2, 2n]]
   * ```
   */
  ceil(): Die { return this.unary("ceil"); }
  /** `round(die, ndigits)`: each outcome rounded as Python's `round`, so halves go to the even neighbour; to an
   * int without `ndigits`, else to `ndigits` decimals.
   * @example
   * ```ts
   * // 0.5 -> 0, 1.5 -> 2, 2.5 -> 2.
   * d(6).truediv(2).round().items(); //=> [[0, 1n], [1, 1n], [2, 3n], [3, 1n]]
   * d(6).truediv(4).round(1).repr(); //=> "Die({0.2: 1, 0.5: 1, 0.8: 1, 1.0: 1, 1.2: 1, 1.5: 1})"
   * ```
   */
  round(ndigits: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.round(ndigits)));
  }

  /** `self @ other`: roll this die, then roll `other` that many times and sum the outcomes. The sum is computed
   * one at a time, each additional roll on the right, as `reduce`; a negative count sums that many rolls and
   * negates the result.
   * @example
   * ```ts
   * d(2).matmul(6).outcomes(); //=> [6, 12]
   * // Roll a d4 to find how many d6 to roll.
   * d(4).matmul(d(6)).mean().toString(); //=> "35/4"
   * ```
   */
  matmul(other: Operand): Die {
    const o = nativeDie(other);
    if (o === undefined) return call("matmul", { n: this, die: other }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => w.matmul(o)));
  }
  /** `n @ self`: the sum of `n` rolls of this die (`n` an int, or a die rolled first to find the count).
   * @example
   * ```ts
   * d(6).rmatmul(3).mean().toString(); //=> "21/2"
   * d(6).rmatmul(3).equals(Die.sum(3, d(6))); //=> true
   * ```
   */
  rmatmul(n: Int | Die): Die {
    const o = nativeDie(n);
    if (o === undefined) return call("matmul", { n, die: this }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => o.matmul(w)));
  }
  /** `n @ die`: the sum of `n` rolls of `die` (`n` an int, or a die rolled first to find the count). The way to
   * write "3d6".
   * @example
   * ```ts
   * Die.sum(3, d(6)).outcomes().length; //=> 16
   * Die.sum(2, d(6)).probability(7).toString(); //=> "1/6"
   * Die.sum(d(4), d(6)).mean().toString(); //=> "35/4"
   * ```
   */
  static sum(n: Int | Die, die: Die): Die {
    return ops.matmul(n, die);
  }

  // --- Selection and rerolls ------------------------------------------------------------------------------------

  /** `Population._select_outcomes(which, star)`: one flag per outcome. A function is called on each outcome in
   * order (unpacked if `star`, which is inferred from its `length` as Python counts parameters); an array is a
   * collection the outcome must be `in` (Python's `==`). `undefined` where the bridge must do it (a Map, an
   * outcome a starred function cannot unpack, ...). */
  private selection(which: unknown, star: unknown): Uint8Array | undefined {
    if (typeof which === "function") {
      const f = which as (...args: unknown[]) => unknown;
      const starred = star === null || star === undefined ? f.length > 1 : pyBool(star);
      const outcomes: unknown[] = this.outcomes();
      const args = outcomes.map((o) => (starred ? unpacked(o) : [o]));
      if (args.some((a) => a === undefined)) return undefined;
      return Uint8Array.from(args, (a) => (pyBool(f(...(a as unknown[]))) ? 1 : 0));
    }
    if (Array.isArray(which) && which.every(isPlainValue)) {
      const w = this.w;
      return Uint8Array.from(wasm(() => w.memberships(which)));
    }
    return undefined;
  }

  /** `die.reroll(which, star=, depth=)`: rerolls the selected outcomes.
   *
   * If the reroll would never end (every outcome selected, unlimited depth), the result has no outcomes.
   *
   * @param which - The outcomes to reroll: an array of them, a function returning whether to reroll an outcome,
   *   or null for the lowest outcome.
   * @param options - Keyword arguments.
   * @param options.star - Whether to unpack each outcome into the function's arguments; inferred from the
   *   function's `length` when not given.
   * @param options.depth - Required, as in Python: the maximum number of rerolls, or Infinity (Python's 'inf')
   *   for no limit.
   * @throws IcepoolError `ValueError` for a negative depth.
   * @example
   * ```ts
   * // Reroll 1s until they stop coming up.
   * d(6).reroll([1], { depth: Infinity }).outcomes(); //=> [2, 3, 4, 5, 6]
   * // Reroll a 1 once: it stays only if rolled twice.
   * d(6).reroll([1], { depth: 1 }).probability(1).toString(); //=> "1/36"
   * d(6).reroll((x: number) => x <= 2, { depth: 1 }).probability(1).toString(); //=> "1/18"
   * ```
   */
  reroll(which: Which = null, { star = null, depth }: { star?: boolean | null; depth?: Int } = {}): Die {
    const d = depthArg(depth);
    if (d !== undefined) {
      const mask = which === null ? this.lowest1() : this.selection(which, star);
      if (mask !== undefined) {
        const w = this.w;
        return Die.wrap(wasm(() => w.reroll(mask, d ?? undefined)));
      }
    }
    return call("reroll", { die: this, which: list(which), star, depth: inf(depth) }) as Die;
  }
  /** `die.filter(which, star=, depth=)`: rerolls until getting one of the selected outcomes; the complement of
   * `reroll`.
   *
   * @param which - The outcomes to keep: an array of them, or a function returning whether to accept an outcome.
   * @param options - Keyword arguments.
   * @param options.star - As for {@link Die.reroll}.
   * @param options.depth - Required: the maximum number of rerolls, or Infinity for no limit.
   * @example
   * ```ts
   * d(6).filter((x: number) => x % 2 === 0, { depth: Infinity }).outcomes(); //=> [2, 4, 6]
   * d(6).filter([5, 6], { depth: 1 }).items(); //=> [[1, 4n], [2, 4n], [3, 4n], [4, 4n], [5, 10n], [6, 10n]]
   * ```
   */
  filter(which: Which, { star = null, depth }: { star?: boolean | null; depth?: Int } = {}): Die {
    const d = depthArg(depth);
    const mask = d !== undefined && which !== null ? this.selection(which, star) : undefined;
    if (mask !== undefined) {
      const w = this.w;
      return Die.wrap(wasm(() => w.reroll(mask.map((m) => 1 - m), d ?? undefined)));
    }
    return call("filter", { die: this, which: list(which), star, depth: inf(depth) }) as Die;
  }
  /** The selection of the lowest outcome alone (`undefined` for an empty die: the bridge raises). */
  private lowest1(): Uint8Array | undefined {
    const n = this.size;
    if (n === 0) return undefined;
    const mask = new Uint8Array(n);
    mask[0] = 1;
    return mask;
  }
  /** `die.truncate(min_outcome, max_outcome)`: the outcomes within the range, endpoints included; a null bound
   * leaves that side as it is. This effectively rerolls the outcomes outside the range; to replace them with the
   * nearest endpoint instead, use {@link Die.clip}. Not to be confused with {@link Die.trunc}.
   * @example
   * ```ts
   * d(6).truncate(2, 5).items(); //=> [[2, 1n], [3, 1n], [4, 1n], [5, 1n]]
   * d(6).truncate(3).outcomes(); //=> [3, 4, 5, 6]
   * ```
   */
  truncate(min: Outcome = null, max: Outcome = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.truncate(min, max)));
  }
  /** `die.clip(min_outcome, max_outcome)`: each outcome outside the range replaced by the nearest endpoint, so
   * quantity bunches up at the endpoints; a null bound leaves that side as it is. To reroll the outcomes outside
   * the range instead, use {@link Die.truncate}.
   * @example
   * ```ts
   * d(6).clip(2, 5).items(); //=> [[2, 2n], [3, 1n], [4, 1n], [5, 2n]]
   * d(6).clip(null, 4).items(); //=> [[1, 1n], [2, 1n], [3, 1n], [4, 3n]]
   * ```
   */
  clip(min: Outcome = null, max: Outcome = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.clip(min, max)));
  }
  /** `die.split(which, star=)`: `[selected, rest]`, a die of the selected outcomes and a die of the others. The
   * denominators of the two add up to this die's. To split more than two ways, use {@link Die.groupBy}.
   *
   * @param which - The outcomes to select: an array of them, or a function returning whether to select one.
   * @param options - Keyword arguments.
   * @param options.star - As for {@link Die.reroll}.
   * @example
   * ```ts
   * const [high, low] = d(6).split((x: number) => x > 4);
   * high.outcomes(); //=> [5, 6]
   * low.outcomes(); //=> [1, 2, 3, 4]
   * ```
   */
  split(which: Which = null, { star = null }: { star?: boolean | null } = {}): [Die, Die] {
    const mask = which === null ? undefined : this.selection(which, star);
    if (mask !== undefined) {
      const w = this.w;
      const [selected, rest] = wasm(() => w.split(mask));
      return [Die.wrap(selected), Die.wrap(rest)];
    }
    return call("split", { die: this, which: list(which), star }) as [Die, Die];
  }
  /** `die.explode(which, star=, depth=9, end=None)`: the selected outcomes are rolled again and added to the
   * total.
   *
   * @param which - The outcomes that explode: an array of them, a function returning whether one explodes, or
   *   null for the highest outcome.
   * @param options - Keyword arguments.
   * @param options.star - As for {@link Die.reroll}.
   * @param options.depth - The maximum number of extra rolls, not counting the first (default 9).
   * @param options.end - What further explosions count as once `depth` is reached: zero when null; `Reroll` makes
   *   one final roll, rerolling until an outcome that does not explode.
   * @throws IcepoolError `ValueError` for a negative depth.
   * @example
   * ```ts
   * d(6).explode([6], { depth: 1 }).outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
   * d(6).explode().mean().toString(); //=> "84652645/20155392"
   * ```
   */
  explode(
    which: Which = null,
    { star = null, depth = 9, end = null }: { star?: boolean | null; depth?: Int; end?: Operand | typeof Reroll | null } = {},
  ): Die {
    return call("explode", { die: this, which: list(which), star, depth, end }) as Die;
  }

  // --- map and friends: `die.map(f, ...extraArgs, { star, repeat })` is `map(f, die, ...extraArgs, ...)` --------

  /** `die.map(repl, *extra_args, star=, repeat=, again_count=, ...)`: maps the outcomes of this die to other
   * outcomes; the same as {@link map}`(repl, die, ...extraArgs, options)`, which documents the options.
   *
   * `repl` is a function called with each outcome (and one outcome of each extra argument, for every joint
   * outcome), or a Map from old outcomes to new ones (unmapped outcomes stay the same). A new outcome may be a die,
   * `Reroll` (rerolls that old outcome), or an Again expression. With `repeat`, the map is applied that many times
   * (Infinity for the limit), which models a Markov process.
   * @example
   * ```ts
   * d(6).map((x: number) => x * 2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
   * // An extra argument: the higher of two d6.
   * d(6).map((x: number, y: number) => Math.max(x, y), d(6)).probability(6).toString(); //=> "11/36"
   * d(6).map((x: number) => (x === 1 ? Reroll : x)).outcomes(); //=> [2, 3, 4, 5, 6]
   * ```
   */
  map(f: Repl, ...rest: unknown[]): Die {
    return map(f, this, ...rest);
  }
  /** `die.map_and_time(repl, *extra_args, star=, repeat=)`: repeatedly maps the outcomes of this die, the
   * state, while counting time steps; the same as {@link mapAndTime}`(repl, die, ...extraArgs, options)`.
   *
   * The outcomes of the result are `[state, time]`, where `time` is the number of steps needed to reach an
   * absorbing state (one that only leads to itself), or `repeat`, whichever is less. It returns early once
   * every state is absorbed, so `repeat` can be as high as the longest time of interest. Extra arguments are
   * rerolled at every step.
   * @example
   * ```ts
   * // Add d2 to a total of 0 until it reaches 2.
   * const process = new Die([0]).mapAndTime((x: number) => (x >= 2 ? x : d(2).add(x)), { repeat: 3 });
   * process.items(); //=> [[[2, 1], 2n], [[2, 2], 1n], [[3, 2], 1n]]
   * ```
   */
  mapAndTime(f: Callback<MapResult>, ...rest: unknown[]): Die {
    return mapAndTime(f, this, ...rest);
  }
  /** `die.mean_time_to_absorb(repl, *extra_args, star=)` (EXPERIMENTAL in icepool): the mean number of steps for
   * the process starting from this die to reach an absorbing state (one that maps to itself with certainty); the
   * same as {@link meanTimeToAbsorb}`(repl, die, ...extraArgs, options)`. `Restart` is not supported.
   * @example
   * ```ts
   * new Die([0]).meanTimeToAbsorb((x: number) => (x >= 2 ? x : d(2).add(x))).toString(); //=> "3/2"
   * ```
   */
  meanTimeToAbsorb(f: Callback<MapResult>, ...rest: unknown[]): Fraction {
    return meanTimeToAbsorb(f, this, ...rest);
  }
  /** `die.mean_time_to_sum(target)`: the mean number of rolls until the running total is at least `target`, as
   * a Fraction.
   * @throws IcepoolError `ValueError` if this die has negative outcomes; `ZeroDivisionError` if its mean is 0.
   * @example
   * ```ts
   * d(6).meanTimeToSum(6).toString(); //=> "16807/7776"
   * d(6).meanTimeToSum(0).toString(); //=> "0"
   * ```
   */
  meanTimeToSum(target: Int): Fraction {
    return (call("mean_time_to_sum", { die: this, targets: List.of(target) }) as Fraction[])[0];
  }
  /** `die.time_to_sum(target, max_time=None, dnf=None)`: a die of the number of rolls until the running total is
   * at least `target`.
   *
   * @param target - The total to stop at.
   * @param maxTime - The maximum number of rolls. Required if some outcome is not positive; by default the most
   *   rolls the target can take.
   * @param dnf - The time given to the cases that do not reach the target in `maxTime` rolls; `maxTime` when
   *   null, `Reroll` to drop those cases.
   * @throws IcepoolError `ValueError` without `maxTime` when some outcome is zero or negative.
   * @example
   * ```ts
   * d(6).timeToSum(3).items(); //=> [[1, 144n], [2, 66n], [3, 6n]]
   * d(6).timeToSum(6).mean().toString(); //=> "16807/7776"
   * // Of the 36 sequences of 2 rolls, the 6 that reach 10.
   * d(6).timeToSum(10, 2, Reroll).items(); //=> [[2, 6n]]
   * ```
   */
  timeToSum(target: Int, maxTime: Int | null = null, dnf: Int | typeof Reroll | null = null): Die {
    return call("time_to_sum", { die: this, target, max_time: maxTime, dnf }) as Die;
  }
  /** `die.group_by(key_map, star=)`: the dice of the outcomes with the same key, by key (in order of first
   * appearance; keys equal by Python's `==`). `key` is called on each outcome (unpacked if `star`), or a Map
   * looked up with `key.get(o, o)` (outcomes not in it are their own key). The denominators of the dice add up to
   * this die's, which is useful for the law of total probability.
   * @example
   * ```ts
   * const byRemainder = d(10).groupBy((x: number) => x % 3);
   * [...byRemainder.keys()]; //=> [1, 2, 0]
   * byRemainder.get(0)!.outcomes(); //=> [3, 6, 9]
   * ```
   */
  groupBy(key: Callback<Outcome> | Map<unknown, Outcome>, { star = null }: { star?: boolean | null } = {}): Map<Outcome, Die> {
    if (typeof key === "function") {
      const f = key as (...args: unknown[]) => unknown;
      const starred = star === null || star === undefined ? f.length > 1 : pyBool(star);
      const outcomes: unknown[] = this.outcomes();
      const args = outcomes.map((o) => (starred ? unpacked(o) : [o]));
      if (!args.some((a) => a === undefined)) {
        const keys = args.map((a) => f(...(a as unknown[])));
        if (keys.every(isPlainValue)) {
          const w = this.w;
          const groups = wasm(() => w.groupBy(keys)) as [Outcome, WasmDie][];
          return new Map(groups.map(([k, d]) => [k, Die.wrap(d)]));
        }
        // Keys the core cannot hold (dice, Reroll): the bridge groups by them, given as a Map of outcome -> key.
        key = new Map(outcomes.map((o, i) => [o, keys[i] as Outcome]));
      }
    }
    return new Map(call("group_by", { die: this, key, star }) as [Outcome, Die][]);
  }
  /** `die.group_by[index]`: {@link Die.groupBy} with the outcome's element at `index` as the key, for dice of
   * tuples.
   * @example
   * ```ts
   * const groups = new Die([[1, 2], [1, 3], [2, 3]]).groupByIndex(0);
   * groups.get(1)!.outcomes(); //=> [[1, 2], [1, 3]]
   * groups.get(2)!.outcomes(); //=> [[2, 3]]
   * ```
   */
  groupByIndex(index: Int): Map<Outcome, Die> {
    return new Map(call("group_by_index", { die: this, index }) as [Outcome, Die][]);
  }
  /** `die.is_in(outcomes)`: a die of whether the roll is `in` the outcomes (Python's `==`).
   * @example
   * ```ts
   * d(6).isIn([5, 6]).items(); //=> [[false, 4n], [true, 2n]]
   * ```
   */
  isIn(outcomes: readonly Outcome[]): Die {
    const mask = Array.isArray(outcomes) ? this.selection(outcomes, false) : undefined;
    if (mask === undefined) return call("is_in", { die: this, outcomes: list(outcomes) }) as Die;
    // `self.map(lambda x: x in outcomes)`.
    return new Die(Array.from(mask, (m) => m === 1), this.quantities());
  }
  /** `die.count(rolls, outcomes)`: rolls this die `rolls` times and counts how many are `in` the outcomes.
   * @example
   * ```ts
   * // Successes on 5+ with 3d6.
   * d(6).count(3, [5, 6]).items(); //=> [[0, 64n], [1, 96n], [2, 48n], [3, 8n]]
   * // No 6 in 4d6.
   * d(6).count(4, [6]).probability(0).toString(); //=> "625/1296"
   * ```
   */
  count(rolls: Int, outcomes: readonly Outcome[]): Die {
    if (!Array.isArray(outcomes)) return call("die_count", { die: this, rolls, outcomes: list(outcomes) }) as Die;
    return this.isIn(outcomes).rmatmul(rolls);
  }
  /** `die.if_else(outcome_if_true, outcome_if_false, again_count=, again_depth=, again_end=)`: the ternary
   * operator; truthy outcomes become `ifTrue`, falsy ones `ifFalse` (either may be a die). The Again options are
   * forwarded to the final die constructor.
   * @example
   * ```ts
   * // On 11+ on a d20, roll a d8.
   * d(20).ge(11).ifElse(d(8), 0).mean().toString(); //=> "9/4"
   * new Die([0, 1, 2]).ifElse("yes", "no").items(); //=> [["no", 1n], ["yes", 2n]]
   * ```
   */
  ifElse(ifTrue: Operand, ifFalse: Operand, { againCount = null, againDepth = null, againEnd = null }: AgainOptions = {}): Die {
    return call("if_else", {
      die: this,
      if_true: ifTrue,
      if_false: ifFalse,
      count: againCount,
      depth: againDepth,
      end: againEnd,
    }) as Die;
  }

  // --- Rolling several ------------------------------------------------------------------------------------------

  /** `Die.highest(rolls, keep=None, drop=None)`: roll several of this die and return the highest result, or the
   * sum of some of the highest. The outcomes should support addition and multiplication if `keep != 1`.
   *
   * `keep` and `drop` work together:
   * - if neither is given, the single highest die is taken;
   * - if only `keep` is given, the `keep` highest dice are summed;
   * - if only `drop` is given, the `drop` highest dice are dropped and the rest summed;
   * - if both are given, the `drop` highest dice are dropped, then the next `keep` highest summed.
   *
   * @param rolls - The number of dice to roll.
   *
   * @example
   * ```ts
   * d(6).highest(2).repr(); //=> "Die({1: 1, 2: 3, 3: 5, 4: 7, 5: 9, 6: 11})"
   * d(6).highest(4, 3).mean().toString(); //=> "15869/1296"
   * ```
   */
  highest(rolls: Int, keep: Int | null = null, drop: Int | null = null): Die {
    return call("die_highest", { die: this, rolls, keep, drop }) as Die;
  }
  /** `Die.lowest(rolls, keep=None, drop=None)`: roll several of this die and return the lowest result, or the sum
   * of some of the lowest. The outcomes should support addition and multiplication if `keep != 1`.
   *
   * `keep` and `drop` work together:
   * - if neither is given, the single lowest die is taken;
   * - if only `keep` is given, the `keep` lowest dice are summed;
   * - if only `drop` is given, the `drop` lowest dice are dropped and the rest summed;
   * - if both are given, the `drop` lowest dice are dropped, then the next `keep` lowest summed.
   *
   * @param rolls - The number of dice to roll.
   *
   * @example
   * ```ts
   * d(6).lowest(2).repr(); //=> "Die({1: 11, 2: 9, 3: 7, 4: 5, 5: 3, 6: 1})"
   * d(6).lowest(3, 1, 1).mean().toString(); //=> "7/2"
   * ```
   */
  lowest(rolls: Int, keep: Int | null = null, drop: Int | null = null): Die {
    return call("die_lowest", { die: this, rolls, keep, drop }) as Die;
  }
  /** `Die.middle(rolls, keep=1, *, tie='error')`: roll several of this die and sum the sorted results in the
   * middle. The outcomes should support addition and multiplication if `keep != 1`.
   *
   * @param rolls - The number of dice to roll.
   * @param keep - The number of outcomes to sum. If this is greater than `rolls`, all are kept.
   * @param options - `tie`: what to do if `keep` is odd but `rolls` is even, or vice versa: `"error"` (the
   *   default) raises, `"high"` takes the higher outcome, `"low"` the lower.
   * @throws IndexError (an `IcepoolError`) for an ambiguous middle with `tie: "error"`.
   *
   * @example
   * ```ts
   * d(6).middle(3).mean().toString(); //=> "7/2"
   * d(6).middle(4, 1); //=> throws IndexError
   * d(6).middle(4, 1, { tie: "high" }).mean().toString(); //=> "1771/432"
   * ```
   */
  middle(rolls: Int, keep: Int = 1, { tie = "error" }: { tie?: "error" | "high" | "low" } = {}): Die {
    return call("die_middle", { die: this, rolls, keep, tie }) as Die;
  }
  /** `Die.keep(rolls, index)`: roll several of this die, sort the results and sum the selected ones.
   *
   * Either `rolls` is an array of counts, one per sorted roll from lowest to highest (each roll is counted that
   * many times, which could be multiple or negative times), and there is no index; or `rolls` is the number of
   * dice to roll and `index` is one of:
   * - an int: only the roll at that sorted position (negative counts from the highest);
   * - an array of counts, one per sorted roll. Up to one `Ellipsis` may be used: it stands for as many zero counts
   *   as make up the difference to `rolls` (e.g. `[1, Ellipsis, 1]` on five dice acts as `[1, 0, 0, 0, 1]`). If
   *   the array is longer than `rolls`, elements are dropped on the side of the `Ellipsis` (`[Ellipsis, 1, 2, 3]`
   *   on two dice acts as `[2, 3]`); an `Ellipsis` in the middle acts as the sum of the two one-sided forms.
   *
   * For a slice (Python's `die.keep(rolls, slice(...))`) see {@link Die.keepSlice}.
   *
   * @throws ValueError (an `IcepoolError`) if `rolls` is an int and no index is given, or if both are arrays.
   *
   * @example
   * ```ts
   * // 4d6, drop the lowest.
   * d(6).keep([0, 1, 1, 1]).mean().toString(); //=> "15869/1296"
   * // Highest plus lowest of 5d6.
   * d(6).keep(5, [1, Ellipsis, 1]).mean().toString(); //=> "7"
   * // The lowest of 3d6.
   * d(6).keep(3, 0).equals(d(6).lowest(3)); //=> true
   * ```
   */
  keep(rolls: Int | readonly Int[], index: Int | readonly (Int | typeof Ellipsis)[] | null = null): Die {
    return call("die_keep", { die: this, rolls, index }) as Die;
  }
  /** `Die.keep(rolls, slice(start, stop, step))`: roll `rolls` of this die, sort the results and sum those in the
   * slice (indices from lowest to highest; negative ones count from the highest; `null` is an omitted bound),
   * each counted once.
   *
   * @example
   * ```ts
   * // 4d6, drop the lowest: Python's d6.keep(4, slice(1, None)).
   * d(6).keepSlice(4, 1).equals(d(6).highest(4, 3)); //=> true
   * d(6).keepSlice(3, 1, 2).equals(d(6).middle(3)); //=> true
   * ```
   */
  keepSlice(rolls: Int, start: Int | null = null, stop: Int | null = null, step: Int | null = null): Die {
    return call("die_keep_slice", { die: this, rolls, start, stop, step }) as Die;
  }
  /** `Die.sequence(rolls)`: the possible sequences produced by rolling this die `rolls` times, as a die of arrays
   * (Python's tuples) in the order rolled. For 0 rolls the result is `[]` itself (Python's empty
   * `cartesian_product()` is `()`), not a die.
   *
   * This is extremely expensive computationally. If possible, use `reduce()` instead; if you don't care about
   * order, {@link Die.pool} is better.
   *
   * @example
   * ```ts
   * d(2).sequence(2).outcomes(); //=> [[1, 1], [1, 2], [2, 1], [2, 2]]
   * ```
   */
  sequence(rolls: Int): Die {
    return call("die_sequence", { die: this, rolls }) as Die;
  }
  /** `Die.pool(rolls=1)`: a {@link Pool} of copies of this die.
   *
   * You might keep from the pool immediately afterwards, e.g. `d(6).pool(5).keep([-1, Ellipsis, 1])` takes the
   * difference between the highest and lowest of 5d6.
   *
   * @param rolls - The number of copies of this die in the pool; or an array of one count per die, acting as a
   *   keep tuple (`Ellipsis` is not allowed here, as the array's length is the size of the pool).
   *
   * @example
   * ```ts
   * d(6).pool(3).sum().equals(d(6).add(d(6)).add(d(6))); //=> true
   * // 4d6, drop the lowest.
   * d(6).pool([0, 1, 1, 1]).sum().mean().toString(); //=> "15869/1296"
   * d(6).pool(5).keep([-1, Ellipsis, 1]).sum().outcomes(); //=> [0, 1, 2, 3, 4, 5]
   * ```
   */
  pool(rolls: Int | readonly Int[] = 1): Pool {
    return call("die_pool", { die: this, rolls: Array.isArray(rolls) ? list(rolls) : rolls }) as Pool;
  }

  // --- Pools from a die (EXPERIMENTAL in icepool) ---------------------------------------------------------------

  /** `Die.map_to_pool(repl=None, *extra_args, star=None)`: EXPERIMENTAL. Maps the outcomes of this die to pools,
   * as {@link mapToPool}`(repl, die, ...extraArgs)`: for each outcome, `repl` returns the contents of a pool (an
   * array of dice or outcomes, a Map from them to counts, or a multiset expression) or `Reroll`; the result is
   * the mixture of those pools.
   *
   * If `repl` is `null`, the outcomes themselves (which must then be arrays or Maps) are used to construct a
   * mixture of pools directly, similar to the inverse of `pool.expand()`. This is not particularly efficient
   * since it does not make much use of dynamic programming.
   *
   * @param rest - Extra arguments for `repl`, then optionally `{ star }`: whether to spread tuple outcomes into
   *   separate arguments (`null`: guessed from the function).
   *
   * @example
   * ```ts
   * // Roll a d3, then that many d6.
   * d(3).mapToPool((n: number) => Array(n).fill(d(6))).sum().mean().toString(); //=> "7"
   * ```
   */
  mapToPool(f: Repl | null = null, ...rest: unknown[]): MultisetExpression {
    const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
    return call("die_map_to_pool", { die: this, f, args: List.from(args), star }) as MultisetExpression;
  }
  /** `Die.explode_to_pool(rolls=1, outcomes=None, *, star=None, depth=9)`: EXPERIMENTAL. Causes outcomes to be
   * rolled again, keeping each explosion as an individual die in a pool.
   *
   * The result is a mixture of pools: not technically a {@link Pool}, though it supports most of the same
   * operations.
   *
   * @param rolls - The number of initial dice.
   * @param outcomes - Which outcomes explode: an array of outcomes, or a function that takes an outcome and
   *   returns `true` if it should explode. `null` (the default) explodes the max outcome.
   * @param options - `star`: whether to spread tuple outcomes into separate arguments of an `outcomes` function
   *   (`null`: guessed from the function). `depth`: the maximum depth of explosions for an individual die.
   *
   * @example
   * ```ts
   * // One d6 exploding once on a 6: a 6 is never the final total.
   * d(6).explodeToPool(1, [6], { depth: 1 }).sum().outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
   * // The highest two of 3d6, each exploding up to twice.
   * d(6).explodeToPool(3, null, { depth: 2 }).highest(2).sum().mean().toString(); //=> "1253/144"
   * ```
   */
  explodeToPool(rolls: Int = 1, outcomes: Which = null, { star = null, depth = 9 }: { star?: boolean | null; depth?: Int } = {}): MultisetExpression {
    return call("explode_to_pool", { die: this, rolls, outcomes: list(outcomes), star, depth }) as MultisetExpression;
  }
  /** `Die.reroll_to_pool(rolls, outcomes=None, *, max_rerolls, star=None, depth=1, mode='random')`: EXPERIMENTAL.
   * Applies a limited number of rerolls shared across a pool of `rolls` dice. Each die can only be rerolled once
   * (effectively `depth: 1`), and no more than `maxRerolls` dice may be rerolled.
   *
   * The result is a mixture of pools: not technically a {@link Pool}, though it supports most of the same
   * operations.
   *
   * @param rolls - How many dice in the pool.
   * @param outcomes - Which outcomes are eligible to be rerolled: an array of outcomes, or a function that takes an
   *   outcome and returns `true` if it could be rerolled. `null` or `undefined`: the single minimum outcome.
   * @param options - `maxRerolls` (required): the maximum total number of rerolls; with `Infinity` this is the same
   *   as `die.reroll(outcomes, { star, depth }).pool(rolls)`. `star`: whether to spread tuple outcomes into
   *   separate arguments of an `outcomes` function (`null`: guessed from the function). `depth`: EXTRA
   *   EXPERIMENTAL, the maximum depth of rerolls (an int or `Infinity`). `mode`: how dice are selected for
   *   rerolling if more are eligible than `maxRerolls`: `"random"` (the default) chooses uniformly at random,
   *   `"low"` the lowest eligible dice, `"high"` the highest; `"drop"` drops every die that ended up on an
   *   eligible outcome (both those that rolled one initially and were not rerolled, and those rerolled into one
   *   again), which can be considerably more efficient than the other modes.
   *
   * @example
   * ```ts
   * // 3d6, rerolling at most one 1.
   * d(6).rerollToPool(3, [1], { maxRerolls: 1 }).sum().mean().toString(); //=> "4991/432"
   * // 2d6, rerolling the lowest 1 or 2.
   * d(6).rerollToPool(2, [1, 2], { maxRerolls: 1, mode: "low" }).sum().mean().toString(); //=> "293/36"
   * ```
   */
  rerollToPool(
    rolls: Int,
    outcomes: Which | undefined,
    { maxRerolls, star = null, depth = 1, mode = "random" }: {
      maxRerolls: Int;
      star?: boolean | null;
      depth?: Int;
      mode?: "random" | "low" | "high" | "drop";
    },
  ): MultisetExpression {
    return call("reroll_to_pool", {
      die: this,
      rolls,
      outcomes: list(outcomes ?? null),
      max_rerolls: inf(maxRerolls),
      star,
      depth: inf(depth),
      mode,
    }) as MultisetExpression;
  }

  // --- Quantities and outcomes ----------------------------------------------------------------------------------

  /** `Die.simplify()`: divides all quantities by their greatest common divisor.
   *
   * @example
   * ```ts
   * new Die([2, 4], [2, 4]).simplify().repr(); //=> "Die({2: 1, 4: 2})"
   * ```
   */
  simplify(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.simplify()));
  }
  /** `Die.zero()`: zeros all outcomes of this die, by multiplying them by `0`. The result has the same
   * denominator.
   *
   * @throws ValueError (an `IcepoolError`) if the zeros did not resolve to a single outcome.
   *
   * @example
   * ```ts
   * d(6).zero().repr(); //=> "Die({0: 6})"
   * ```
   */
  zero(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.zero()));
  }
  /** `Die.zero_outcome()`: a zero outcome for this die, e.g. `0` for a die whose outcomes are ints.
   *
   * @example
   * ```ts
   * d(6).zeroOutcome(); //=> 0
   * ```
   */
  zeroOutcome(): Outcome {
    const w = this.w;
    return wasm(() => w.zeroOutcome()) as Outcome;
  }
  /** `Die.sign()`: outcomes become 1 if greater than `zero()`, -1 if less than `zero()`, and 0 otherwise.
   *
   * For floats, +0.0, -0.0 and NaN all become 0.
   *
   * @example
   * ```ts
   * new Die([-2, 0, 3]).sign().repr(); //=> "Die({-1: 1, 0: 1, 1: 1})"
   * ```
   */
  sign(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.sign()));
  }
  /** `Die.cmp(other)`: a die with outcomes 1, -1 and 0, whose quantities are those of `true` in `this > other`,
   * `this < other`, and the remainder respectively.
   *
   * @example
   * ```ts
   * d(6).cmp(3).repr(); //=> "Die({-1: 2, 0: 1, 1: 3})"
   * d(6).cmp(d(6)).repr(); //=> "Die({-1: 15, 0: 6, 1: 15})"
   * ```
   */
  cmp(other: Operand): Die {
    const o = nativeDie(other);
    if (o === undefined) return call("die_cmp", { die: this, other }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => w.cmp(o)));
  }
  /** `Die.append(outcome, quantity=1)`: this die with `quantity` more of `outcome`. A negative quantity removes
   * quantity instead (but not below zero).
   *
   * @example
   * ```ts
   * d(4).append(4).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 2})"
   * d(4).append(5, 2).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 1, 5: 2})"
   * ```
   */
  append(outcome: Outcome, quantity: Int = 1): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.append(outcome, quantity)));
  }
  /** `Die.remove(outcome, quantity=None)`: this die with `quantity` less of `outcome`; `null` (the default)
   * removes all of it. A negative quantity adds quantity instead.
   *
   * @example
   * ```ts
   * d(4).remove(4).repr(); //=> "Die({1: 1, 2: 1, 3: 1})"
   * d(4).append(4).remove(4, 1).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 1})"
   * ```
   */
  remove(outcome: Outcome, quantity: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.remove(outcome, quantity)));
  }
  /** `Die.to_one_hot(outcomes=None)`: converts the outcomes of this die to a one-hot representation: each outcome
   * becomes a {@link Vector} of bools with `true` at `outcomes.indexOf(outcome)` and `false` elsewhere (all
   * `false` if the outcome is not in `outcomes`).
   *
   * @param outcomes - The outcomes to index; `null` (the default) uses this die's outcomes.
   *
   * @example
   * ```ts
   * d(2).toOneHot().repr(); //=> "Die({Vector((False, True)): 1, Vector((True, False)): 1})"
   * ```
   */
  toOneHot(outcomes: readonly Outcome[] | null = null): Die {
    return call("to_one_hot", { population: this, outcomes: list(outcomes) }) as Die;
  }
  /** `Die.unary_operator(op)`: performs `op` on each outcome; outcomes that map to the same result have their
   * quantities merged. This is what the unary operators (`neg`, `abs`, `round`, ...) do, so it is mostly useful
   * for an operation they don't cover; {@link Die.map} is the more general tool.
   *
   * @example
   * ```ts
   * d(6).unaryOperator((x: number) => x % 2).repr(); //=> "Die({0: 3, 1: 3})"
   * ```
   */
  unaryOperator(op: Callback<Outcome>): Die {
    const f = op as (outcome: unknown) => unknown;
    return new Die(this.outcomes().map((o) => f(o) as Outcome), this.quantities());
  }
  /** `Die.binary_operator(other, op)`: performs `op` on each pair of outcomes of this die and `other`, with the
   * product of their quantities; equal results have their quantities merged. This is what the binary operators
   * (`add`, `lt`, `cmp`, ...) do, so it is mostly useful for an operation they don't cover.
   *
   * @example
   * ```ts
   * d(4).binaryOperator(d(4), (a: number, b: number) => Math.max(a, b)).repr(); //=> "Die({1: 1, 2: 3, 3: 5, 4: 7})"
   * ```
   */
  binaryOperator(other: Die, op: Callback<Outcome>): Die {
    const f = op as (a: unknown, b: unknown) => unknown;
    const results: Outcome[] = [];
    const quantities: bigint[] = [];
    for (const [a, qa] of this.items()) {
      for (const [b, qb] of other.items()) {
        results.push(f(a, b) as Outcome);
        quantities.push(qa * qb);
      }
    }
    return new Die(results, quantities);
  }
  /** `Die.pad_to_denominator(denominator, outcome)`: changes the denominator to `denominator` by changing the
   * quantity of `outcome`. If that quantity becomes zero, the outcome is removed.
   *
   * @throws ValueError (an `IcepoolError`) if this would require a negative quantity of `outcome`.
   *
   * @example
   * ```ts
   * // A d4 that fails (0) a third of the time.
   * d(4).padToDenominator(6, 0).repr(); //=> "Die({0: 2, 1: 1, 2: 1, 3: 1, 4: 1})"
   * d(4).padToDenominator(2, 4); //=> throws ValueError
   * ```
   */
  padToDenominator(denominator: Int, outcome: Outcome): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.padToDenominator(denominator, outcome)));
  }
  /** `Die.multiply_to_denominator(denominator)`: multiplies all quantities to reach the target denominator.
   *
   * @throws ValueError (an `IcepoolError`) if this cannot be done with an integer scale.
   *
   * @example
   * ```ts
   * d(4).multiplyToDenominator(12).repr(); //=> "Die({1: 3, 2: 3, 3: 3, 4: 3})"
   * d(4).multiplyToDenominator(10); //=> throws ValueError
   * ```
   */
  multiplyToDenominator(denominator: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.multiplyToDenominator(denominator)));
  }
  /** `Die.multiply_quantities(scale)`: multiplies all quantities by an integer.
   *
   * @example
   * ```ts
   * d(4).multiplyQuantities(3).repr(); //=> "Die({1: 3, 2: 3, 3: 3, 4: 3})"
   * ```
   */
  multiplyQuantities(scale: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.multiplyQuantities(scale)));
  }
  /** `Die.divide_quantities(divisor)`: divides all quantities by an integer, rounding down. Resulting zero
   * quantities are dropped. A divisor of 0 returns the die unchanged.
   *
   * @example
   * ```ts
   * new Die([1, 2, 3], [4, 5, 6]).divideQuantities(5).repr(); //=> "Die({2: 1, 3: 1})"
   * ```
   */
  divideQuantities(divisor: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.divideQuantities(divisor)));
  }
  /** `Die.modulo_quantities(divisor)`: the remainder of all quantities divided by an integer. Resulting zero
   * quantities are dropped.
   *
   * @example
   * ```ts
   * new Die([1, 2, 3], [4, 5, 6]).moduloQuantities(5).repr(); //=> "Die({1: 4, 3: 1})"
   * ```
   */
  moduloQuantities(divisor: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.moduloQuantities(divisor)));
  }
  /** `Die.stochastic_round(*, max_denominator=None)`: randomly rounds outcomes up or down to the nearest integer
   * according to the two distances: `x` is rounded up with probability `x - floor(x)` and down otherwise.
   *
   * @param options - `maxDenominator`: if given, each rounding uses the outcome's fraction limited to this
   *   denominator (Python's `Fraction.limit_denominator`); otherwise the exact value.
   *
   * @example
   * ```ts
   * new Die([1.5]).stochasticRound().repr(); //=> "Die({1: 1, 2: 1})"
   * new Die([new Fraction(1, 2), new Fraction(3, 2)]).stochasticRound().repr(); //=> "Die({0: 1, 1: 2, 2: 1})"
   * ```
   */
  stochasticRound({ maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
    return call("die_stochastic_round", { die: this, max_denominator: maxDenominator }) as Die;
  }
  /** `Die.equals(other, *, simplify=False)`: `true` iff both dice have the same outcomes and quantities.
   *
   * This is `false` if `other` is not a die, even if it would convert to an equal die. The truth value of a
   * {@link DieWithTruth} does not matter. A die with a zero-quantity outcome is not equal to one without that
   * outcome.
   *
   * @param options - `simplify`: if `true`, the dice are simplified before comparing; otherwise e.g. a 2:2 coin is
   *   not equal to a 1:1 coin.
   *
   * @example
   * ```ts
   * d(6).equals(d(6)); //=> true
   * d(6).equals(6); //=> false
   * new Die([1, 1, 2, 2]).equals(d(2)); //=> false
   * new Die([1, 1, 2, 2]).equals(d(2), { simplify: true }); //=> true
   * ```
   */
  equals(other: unknown, { simplify = false }: { simplify?: boolean } = {}): boolean {
    if (!(other instanceof Die)) {
      // Python asks `isinstance(other, Die)` before looking at any data, even its own (this only checks that
      // the die was not freed).
      void this.w;
      return false;
    }
    const [w, o] = [this.w, other.w];
    return wasm(() => w.equals(o, simplify));
  }

  // --- The mapping protocol: outcomes -> quantities (BigInts) ----------------------------------------------------

  /** `Die.items()`: the `[outcome, quantity]` pairs in sorted order.
   *
   * @example
   * ```ts
   * d(3).items(); //=> [[1, 1n], [2, 1n], [3, 1n]]
   * ```
   */
  items(): [T, bigint][] {
    const w = this.w;
    return wasm(() => w.items()) as [T, bigint][];
  }
  /** `Die.outcomes()`: the outcomes in ascending order. These are also the `keys` of the mapping.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).outcomes().length; //=> 11
   * new Die([3, 1, 2, 1]).outcomes(); //=> [1, 2, 3]
   * ```
   */
  outcomes(): T[] {
    const w = this.w;
    return wasm(() => w.outcomes()) as T[];
  }
  /** `Die.keys()`: the outcomes in sorted order, as {@link Die.outcomes} (the preferred name).
   *
   * @example
   * ```ts
   * d(3).keys(); //=> [1, 2, 3]
   * ```
   */
  keys(): T[] {
    return this.outcomes();
  }
  /** `Die.values()`: the quantities in outcome order, as {@link Die.quantities}.
   *
   * @example
   * ```ts
   * new Die([1, 2, 2]).values(); //=> [1n, 2n]
   * ```
   */
  values(): bigint[] {
    return this.quantities();
  }
  /** Python's `die[outcome]`: the quantity of an outcome.
   *
   * @throws KeyError (an `IcepoolError`) if `outcome` is not an outcome of the die; {@link Die.quantity} returns
   *   `0n` instead.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).get(7); //=> 6n
   * d(6).get(7); //=> throws KeyError
   * ```
   */
  get(outcome: Outcome): bigint {
    const w = this.w;
    return wasm(() => w.get(outcome)) as bigint;
  }
  /** Python's `outcome in die`: whether `outcome` is an outcome of the die.
   *
   * @example
   * ```ts
   * d(6).has(6); //=> true
   * d(6).has(7); //=> false
   * ```
   */
  has(outcome: Outcome): boolean {
    const w = this.w;
    return wasm(() => w.has(outcome));
  }
  /** Python's `len(die)`: the number of outcomes. For the total quantity, use {@link Die.denominator}. */
  get size(): number {
    return this.w.size;
  }
  /** Python's `iter(die)`: the outcomes in sorted order.
   *
   * @example
   * ```ts
   * [...d(3)]; //=> [1, 2, 3]
   * ```
   */
  [Symbol.iterator](): Iterator<T> {
    return this.outcomes()[Symbol.iterator]();
  }

  // --- Statistics: exact ones are Fractions (or ints), the rest floats (numbers) ---------------------------------

  /** `Die.mean()`: the mean of the outcomes, weighted by their quantities: a {@link Fraction} for int (or
   * Fraction) outcomes, a float otherwise.
   *
   * @example
   * ```ts
   * d(6).mean().toString(); //=> "7/2"
   * d(6).add(d(6)).mean().toString(); //=> "7"
   * ```
   */
  mean(): Fraction | number { return this.stat((w) => w.mean()) as Fraction | number; }
  /** `Die.variance()`: the population variance (not the sample variance); a {@link Fraction} for int (or
   * Fraction) outcomes, a float otherwise.
   *
   * @example
   * ```ts
   * d(6).variance().toString(); //=> "35/12"
   * ```
   */
  variance(): Fraction | number { return this.stat((w) => w.variance()) as Fraction | number; }
  /** `Die.standard_deviation()`: the square root of the {@link Die.variance}, a float.
   *
   * @example
   * ```ts
   * Number(d(6).standardDeviation()).toFixed(4); //=> "1.7078"
   * ```
   */
  standardDeviation(): number { return this.stat((w) => w.standardDeviation()) as number; }
  /** `Die.sd()`: the standard deviation, as {@link Die.standardDeviation}.
   *
   * @example
   * ```ts
   * Number(d(6).add(d(6)).sd()).toFixed(4); //=> "2.4152"
   * ```
   */
  sd(): number { return this.standardDeviation(); }
  /** `Die.skewness()`: the third standardized moment, a float.
   *
   * @example
   * ```ts
   * // A symmetric die has no skew; the highest of 2d6 leans high.
   * Number(d(6).skewness()); //=> 0
   * Number(d(6).highest(2).skewness()) < 0; //=> true
   * ```
   */
  skewness(): number { return this.stat((w) => w.skewness()) as number; }
  /** `Die.excess_kurtosis()`: the fourth standardized moment minus 3 (that of a normal distribution), a float.
   *
   * @example
   * ```ts
   * Number(d(6).excessKurtosis()).toFixed(4); //=> "-1.2686"
   * ```
   */
  excessKurtosis(): number { return this.stat((w) => w.excessKurtosis()) as number; }
  /** `Die.mode()`: the most common outcome(s), sorted from lowest to highest.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).mode(); //=> [7]
   * new Die([1, 2, 2, 3, 3]).mode(); //=> [2, 3]
   * ```
   */
  mode(): T[] { return this.stat((w) => w.mode()) as T[]; }
  /** `Die.modal_quantity()`: the highest quantity of any single outcome.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).modalQuantity(); //=> 6n
   * ```
   */
  modalQuantity(): bigint { return this.stat((w) => w.modalQuantity()) as bigint; }
  /** `Die.median()`: the median, taking the mean in case of a tie.
   *
   * This fails if the outcomes do not support addition and division; in this case use {@link Die.medianLow} or
   * {@link Die.medianHigh} instead.
   *
   * @example
   * ```ts
   * d(6).median(); //=> 3.5
   * Number(d(6).add(d(6)).median()); //=> 7
   * ```
   */
  median(): Outcome { return this.quantile(1, 2); }
  /** `Die.median_low()`: the median, taking the lower outcome in case of a tie.
   *
   * @example
   * ```ts
   * d(6).medianLow(); //=> 3
   * ```
   */
  medianLow(): Outcome { return this.quantileLow(1, 2); }
  /** `Die.median_high()`: the median, taking the higher outcome in case of a tie.
   *
   * @example
   * ```ts
   * d(6).medianHigh(); //=> 4
   * ```
   */
  medianHigh(): Outcome { return this.quantileHigh(1, 2); }
  /** `Die.min_outcome()`: the least outcome.
   *
   * @example
   * ```ts
   * d(6).sub(3).minOutcome(); //=> -2
   * ```
   */
  minOutcome(): T { return this.stat((w) => w.minOutcome()) as T; }
  /** `Die.max_outcome()`: the greatest outcome.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).maxOutcome(); //=> 12
   * ```
   */
  maxOutcome(): T { return this.stat((w) => w.maxOutcome()) as T; }
  /** `Die.denominator()`: the sum of all quantities (e.g. weights or duplicates). For the number of distinct
   * outcomes, use {@link Die.size}.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).denominator(); //=> 36n
   * new Die([1, 2, 2]).denominator(); //=> 3n
   * ```
   */
  denominator(): bigint { return this.stat((w) => w.denominator()) as bigint; }
  /** `Die.is_empty()`: `true` iff this die has no outcomes.
   *
   * @example
   * ```ts
   * new Die([]).isEmpty(); //=> true
   * d(6).isEmpty(); //=> false
   * ```
   */
  isEmpty(): boolean { return this.size === 0; }
  /** `Die.quantile(n, d=100)`: the outcome `n / d` of the way through the CDF, taking the mean in case of a tie.
   *
   * This fails if the outcomes do not support addition and division; in this case use {@link Die.quantileLow} or
   * {@link Die.quantileHigh} instead.
   *
   * @example
   * ```ts
   * // The 90th percentile of a d20.
   * d(20).quantile(90); //=> 18.5
   * ```
   */
  quantile(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantile(n, d)) as Outcome; }
  /** `Die.quantile_low(n, d=100)`: the outcome `n / d` of the way through the CDF, taking the lesser in case of a
   * tie.
   *
   * @example
   * ```ts
   * d(20).quantileLow(90); //=> 18
   * ```
   */
  quantileLow(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantileLow(n, d)) as Outcome; }
  /** `Die.quantile_high(n, d=100)`: the outcome `n / d` of the way through the CDF, taking the greater in case of
   * a tie.
   *
   * @example
   * ```ts
   * d(20).quantileHigh(90); //=> 19
   * ```
   */
  quantileHigh(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantileHigh(n, d)) as Outcome; }
  /** `Die.standardized_moment(k)`: the `k`-th moment about the mean, divided by the standard deviation to the
   * `k`-th power; a float.
   *
   * @example
   * ```ts
   * Number(d(6).standardizedMoment(2)); //=> 1
   * ```
   */
  standardizedMoment(k: Int): number { return this.stat((w) => w.standardizedMoment(k)) as number; }
  /** `Die.entropy(base=2.0)`: the entropy of a random sample from this die.
   *
   * @param base - The logarithm base. The default 2 gives the entropy in bits.
   *
   * @example
   * ```ts
   * Number(d(8).entropy()); //=> 3
   * d(8).entropy(4); //=> 1.5
   * ```
   */
  entropy(base: number | Float = new Float(2)): number { return this.stat((w) => w.entropy(base)) as number; }
  /** `Die.kolmogorov_smirnov(other)`: the Kolmogorov–Smirnov statistic, the maximum absolute difference between
   * the CDFs of the two dice.
   *
   * @example
   * ```ts
   * d(6).kolmogorovSmirnov(d(4)).toString(); //=> "1/3"
   * ```
   */
  kolmogorovSmirnov(other: Die): Fraction | number {
    const o = other.w;
    return this.stat((w) => w.kolmogorovSmirnov(o)) as Fraction | number;
  }
  /** `Die.cramer_von_mises(other)`: the Cramér–von Mises statistic, the sum of the squared differences between
   * the CDFs of the two dice.
   *
   * @example
   * ```ts
   * d(6).cramerVonMises(d(4)).toString(); //=> "17/72"
   * ```
   */
  cramerVonMises(other: Die): Fraction | number {
    const o = other.w;
    return this.stat((w) => w.cramerVonMises(o)) as Fraction | number;
  }
  /** `Die.covariance(i, j)`: the covariance of elements `i` and `j` of tuple outcomes; a {@link Fraction} for int
   * (or Fraction) elements, a float otherwise.
   *
   * @example
   * ```ts
   * // Two independent dice don't covary; an element with itself gives the variance.
   * tupleize(d(6), d(6)).covariance(0, 1).toString(); //=> "0"
   * d(6).map((x: number) => [x, x]).covariance(0, 1).toString(); //=> "35/12"
   * ```
   */
  covariance(i: Int, j: Int): Fraction | number { return this.stat((w) => w.covariance(i, j)) as Fraction | number; }
  /** `Die.correlation(i, j)`: the correlation of elements `i` and `j` of tuple outcomes, a float.
   *
   * @example
   * ```ts
   * // A d6 and the number on its opposite face.
   * Number(d(6).map((x: number) => [x, 7 - x]).correlation(0, 1)); //=> -1
   * ```
   */
  correlation(i: Int, j: Int): number { return this.stat((w) => w.correlation(i, j)) as number; }
  /** `Die.common_outcome_length()`: the length common to all outcomes, or `null` if outcomes have no lengths or
   * different lengths.
   *
   * @example
   * ```ts
   * tupleize(d(6), d(6)).commonOutcomeLength(); //=> 2
   * d(6).commonOutcomeLength(); //=> null
   * ```
   */
  commonOutcomeLength(): number | null { return this.stat((w) => w.commonOutcomeLength() ?? null) as number | null; }
  /** `Die.nearest(comparison, outcome)`: the nearest outcome of this die fitting the comparison, or `null` if
   * there is none. For example, `"<="` finds the greatest outcome that is not greater than `outcome`.
   *
   * @param comparison - One of `"<="`, `"<"`, `">="`, `">"`.
   * @throws ValueError (an `IcepoolError`) for any other comparison.
   *
   * @example
   * ```ts
   * const evens = new Die([2, 4, 6, 8]);
   * evens.nearest("<=", 5); //=> 4
   * evens.nearest(">", 5); //=> 6
   * evens.nearest("<", 2); //=> null
   * ```
   */
  nearest(comparison: Comparison, outcome: Outcome): T | null {
    return this.stat((w) => w.nearest(comparison, outcome)) as T | null;
  }

  /** `Die.quantity(comparison, outcome)`: the quantity of a single outcome, `0n` if it is not an outcome of the
   * die; or, given a comparison first, the total quantity of the outcomes fitting the comparison to `outcome`.
   *
   * @param args - `(outcome)` or `(comparison, outcome)`; `comparison` is one of `"=="`, `"!="`, `"<="`, `"<"`,
   *   `">="`, `">"`.
   *
   * @example
   * ```ts
   * d(6).quantity(3); //=> 1n
   * d(6).quantity(7); //=> 0n
   * d(6).quantity("<=", 3); //=> 3n
   * d(6).add(d(6)).quantity(">=", 10); //=> 6n
   * ```
   */
  quantity(...args: [Outcome] | [Comparison, Outcome]): bigint {
    const [c, outcome] = comparisonArgs(args);
    return this.stat((w) => w.quantity(c, outcome)) as bigint;
  }
  /** `Die.quantities(comparison=None)`: the quantities in outcome order; or, given a comparison, the total
   * quantity fitting the comparison for each outcome (e.g. `"<="` gives the cumulative distribution).
   *
   * @param comparison - One of `"=="` (the same as `null`), `"!="`, `"<="`, `"<"`, `">="`, `">"`.
   *
   * @example
   * ```ts
   * new Die([1, 2, 2, 3]).quantities(); //=> [1n, 2n, 1n]
   * d(4).quantities("<="); //=> [1n, 2n, 3n, 4n]
   * d(4).quantities(">="); //=> [4n, 3n, 2n, 1n]
   * ```
   */
  quantities(comparison: Comparison | null = null): bigint[] {
    return this.stat((w) => w.quantities(comparison ?? undefined)) as bigint[];
  }
  /** `Die.probability(comparison, outcome, *, percent=False)`: the probability of a single outcome (zero if it is
   * not an outcome of the die); or, given a comparison first, the total probability of the outcomes fitting the
   * comparison to `outcome`.
   *
   * @param args - `(outcome)` or `(comparison, outcome)` (`comparison` one of `"=="`, `"!="`, `"<="`, `"<"`,
   *   `">="`, `">"`), then optionally `{ percent }`: if `true`, the result is a percentage as a float; otherwise
   *   a {@link Fraction}.
   *
   * @example
   * ```ts
   * d(6).probability(3).toString(); //=> "1/6"
   * d(6).probability(">=", 5).toString(); //=> "1/3"
   * d(6).probability(">=", 5, { percent: true }); //=> 33.33333333333333
   * ```
   */
  probability(
    ...args: [Outcome] | [Comparison, Outcome] | [Outcome, { percent?: boolean }] | [Comparison, Outcome, { percent?: boolean }]
  ): Fraction | number {
    const [a, { percent = false }] = opts<{ percent: boolean }>(args);
    const [c, outcome] = comparisonArgs(a as [Outcome] | [Comparison, Outcome]);
    return this.stat((w) => w.probability(c, outcome, percent)) as Fraction | number;
  }
  /** `Die.probabilities(comparison=None, *, percent=False)`: the probabilities in outcome order; or, given a
   * comparison, the total probability fitting the comparison for each outcome (e.g. `"<="` gives the CDF).
   *
   * @param comparison - One of `"=="` (the same as `null`), `"!="`, `"<="`, `"<"`, `">="`, `">"`.
   * @param options - `percent`: if `true`, the results are percentages as floats; otherwise
   *   {@link Fraction}s.
   *
   * @example
   * ```ts
   * d(4).probabilities().map(String); //=> ["1/4", "1/4", "1/4", "1/4"]
   * d(4).probabilities("<=").map(String); //=> ["1/4", "1/2", "3/4", "1"]
   * d(4).probabilities(null, { percent: true }).map(Number); //=> [25, 25, 25, 25]
   * ```
   */
  probabilities(comparison: Comparison | null = null, { percent = false }: { percent?: boolean } = {}): (Fraction | number)[] {
    return this.stat((w) => w.probabilities(comparison ?? undefined, percent)) as (Fraction | number)[];
  }
  /** `Die.quantity_where(which, star=None)`: the total quantity of the outcomes for which `which` returns `true`.
   *
   * @param options - `star`: whether to spread tuple outcomes into separate arguments of `which` (`null`: guessed
   *   from the function).
   *
   * @example
   * ```ts
   * d(6).add(d(6)).quantityWhere((x: number) => x % 2 === 0); //=> 18n
   * // The first of two d6 beats the second.
   * tupleize(d(6), d(6)).quantityWhere((a: number, b: number) => a > b); //=> 15n
   * ```
   */
  quantityWhere(which: Callback, { star = null }: { star?: boolean | null } = {}): bigint {
    const mask = typeof which === "function" ? this.selection(which, star) : undefined;
    if (mask === undefined) return BigInt(call("quantity_where", { die: this, which, star }) as Int);
    const w = this.w;
    return wasm(() => w.quantityWhere(mask)) as bigint;
  }
  /** `Die.probability_where(which, star=None, percent=False)`: the total probability of the outcomes for which
   * `which` returns `true`.
   *
   * @param options - `star`: whether to spread tuple outcomes into separate arguments of `which` (`null`: guessed
   *   from the function). `percent`: if `true`, the result is a percentage as a float; otherwise a
   *   {@link Fraction}.
   *
   * @example
   * ```ts
   * d(6).add(d(6)).probabilityWhere((x: number) => x >= 10).toString(); //=> "1/6"
   * tupleize(d(6), d(6)).probabilityWhere((a: number, b: number) => a > b).toString(); //=> "5/12"
   * ```
   */
  probabilityWhere(which: Callback, { star = null, percent = false }: { star?: boolean | null; percent?: boolean } = {}): Fraction | number {
    const mask = typeof which === "function" ? this.selection(which, star) : undefined;
    if (mask === undefined) return call("probability_where", { die: this, which, star, percent }) as Fraction | number;
    const w = this.w;
    return wasm(() => w.probabilityWhere(mask, pyBool(percent))) as Fraction | number;
  }

  private stat(f: (w: WasmDie) => unknown): unknown {
    const w = this.w;
    return wasm(() => f(w));
  }

  // --- Tuple outcomes -------------------------------------------------------------------------------------------

  /** Python's `list(die.marginals)`: one die per element of the (tuple) outcomes, as many as the shortest
   * outcome has elements.
   *
   * @example
   * ```ts
   * tupleize(d(4), d(6)).marginals().map((m) => m.maxOutcome()); //=> [4, 6]
   * ```
   */
  marginals(): Die[] {
    const w = this.w;
    const n = wasm(() => w.marginalsLength());
    return Array.from({ length: n }, (_, i) => this.marginal(i));
  }
  /** Python's `die.marginals[index]`: the die of element `index` of the (tuple) outcomes (negative counts from
   * the end).
   *
   * @example
   * ```ts
   * tupleize(d(4), d(6)).marginal(1).equals(d(6), { simplify: true }); //=> true
   * tupleize(d(4), d(6)).marginal(-1).denominator(); //=> 24n
   * ```
   */
  marginal(index: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.marginal(index)));
  }
  /** Python's `die.marginals[start:stop:step]`: the die of a slice of the (tuple) outcomes, as tuples; `null` is
   * an omitted bound.
   *
   * @example
   * ```ts
   * const three = tupleize(d(4), d(6), d(8));
   * three.marginalSlice(1).maxOutcome(); //=> [6, 8]
   * three.marginalSlice(null, null, 2).maxOutcome(); //=> [4, 8]
   * ```
   */
  marginalSlice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.marginalSlice(start, stop, step)));
  }

  /** `Die.sample()`: one outcome at random, weighted by the quantities. The random numbers are Python's (its
   * Mersenne Twister seeded with `seed`, a random seed if not given), so a seed gives the same rolls as
   * `random.seed(seed)` in Python. Not cryptographically secure.
   *
   * @example
   * ```ts
   * d(6).sample({ seed: 42 }); //=> 6
   * d(6).has(d(6).sample()); //=> true
   * ```
   */
  sample(options?: { seed?: Int }): T;
  /** `n` outcomes at random, as `n` calls of Python's `die.sample()` after `random.seed(seed)`.
   *
   * @example
   * ```ts
   * d(6).sample({ seed: 42, n: 5 }); //=> [6, 1, 1, 6, 3]
   * ```
   */
  sample(options: { seed?: Int; n: Int }): T[];
  sample({ seed = Math.floor(Math.random() * 2 ** 32), n }: { seed?: Int; n?: Int } = {}): T | T[] {
    const rolls = call("sample", { die: this, seed, n: n ?? 1 }) as T[];
    return n === undefined ? rolls[0] : rolls;
  }

  // --- Formatting -----------------------------------------------------------------------------------------------

  /** Python's `type(die).__name__`. @internal */
  protected get typeName(): string {
    return "Die";
  }
  /** Python's `format(die, spec)` (`Die.format(spec)`): this die as a table.
   *
   * `spec` starts with the output format: `md` for Markdown (the default), `bbcode` for BBCode, `csv` for
   * comma-separated values, or `html` for HTML. It may continue with a `:` and the requested columns:
   * - `o`: outcomes; `*o`: outcomes, unpacked if applicable;
   * - `q==`, `q<=`, `q>=`: quantities ==, <= or >= each outcome;
   * - `p==`, `p<=`, `p>=`: probabilities (0-1);
   * - `%==`, `%<=`, `%>=`: probabilities (0%-100%);
   * - `i==`, `i<=`, `i>=`: EXPERIMENTAL, "1 in N".
   *
   * Columns may be separated with `|`. The default (`null` or `""`) is `md:*o|q==|%==`; the quantities are left
   * out of the default columns if any is 10**30 or greater.
   *
   * @throws ValueError (an `IcepoolError`) for an unknown output format or more than one `:`.
   *
   * @example
   * ```ts
   * d(4).format("csv:o|q<=").split("\r\n"); //=> ["Outcome,Quantity <=", "1,1", "2,2", "3,3", "4,4", ""]
   * ```
   */
  format(spec: string | null = null): string {
    const w = this.w;
    const name = this.typeName;
    return wasm(() => w.format(spec ?? "", name));
  }
  /** Python's `str(die)`: the default {@link Die.format}, a Markdown table.
   *
   * @example
   * ```ts
   * d(2).toString().split("\n")[0]; //=> "Die with denominator 2"
   * ```
   */
  toString(): string {
    return this.format(null);
  }
  /** Python's `repr(die)`: `Die({outcome: quantity, ...})` with Python's reprs of the outcomes.
   *
   * @example
   * ```ts
   * new Die([1, 2, 2]).repr(); //=> "Die({1: 1, 2: 2})"
   * d(2).eq(1).repr(); //=> "DieWithTruth({False: 1, True: 1})"
   * ```
   */
  repr(): string {
    const w = this.w;
    const name = this.typeName;
    return wasm(() => w.repr(name));
  }
}

/** `(outcome)` or `(comparison, outcome)`, as `quantity` and `probability` take them. */
function comparisonArgs(args: readonly unknown[]): [string | undefined, unknown] {
  if (args.length === 1) return [undefined, args[0]];
  if (args.length === 2) {
    if (typeof args[0] !== "string") throw new TypeError("icepool: a comparison must be a string such as '<='");
    // A second argument of null is Python's one-argument form.
    return args[1] === null ? [undefined, args[0]] : [args[0], args[1]];
  }
  throw new TypeError("icepool: quantity() takes 1 or 2 positional arguments");
}

/** `icepool.DieWithTruth`: the result of `die.eq(x)` / `die.ne(x)`, a die of bools that also has a truth value:
 * {@link DieWithTruth.truth}, whether the two dice are (or are not) {@link Die.equals}.
 *
 * As in Python, the die's data is evaluated lazily, since the caller may only be interested in the truth value:
 * if computing the data raises, the error comes back only when the data is used. It is not constructed directly.
 *
 * @example
 * ```ts
 * const same = d(6).eq(d(6));
 * same.repr(); //=> "DieWithTruth({False: 30, True: 6})"
 * same.truth(); //=> true
 * d(6).ne(d(6)).truth(); //=> false
 * ```
 */
export class DieWithTruth extends Die<boolean> {
  /** @internal */
  private truthValue = false;
  /** The handle of a result whose data raised (kept so the error comes back when the data is used). @internal */
  private errorHandle = -1;

  /** Wraps a core die with its truth value. @internal */
  static wrapTruth(w: WasmDie, truth: boolean): DieWithTruth {
    const die = Object.create(DieWithTruth.prototype) as DieWithTruth;
    die.core = w;
    die.truthValue = truth;
    die.errorHandle = -1;
    return die;
  }

  /** A result whose data raised: the data's error comes back from every use but `truth()` and `equals`. @internal */
  static failed(handle: number, truth: boolean): DieWithTruth {
    const die = Object.create(DieWithTruth.prototype) as DieWithTruth;
    die.core = null;
    die.truthValue = truth;
    die.errorHandle = handle;
    failedRegistry.register(die, handle);
    return die;
  }

  /** @internal */
  override get w(): WasmDie {
    if (this.core === null && this.errorHandle >= 0) {
      const h = this.errorHandle;
      // Throws the data's error.
      return wasm((m) => m.WasmDie.fromHandle(h));
    }
    return super.w;
  }

  /** @internal */
  override [ENCODE](temps: number[] | null): string {
    if (this.core === null && this.errorHandle >= 0) return `{"$h":${this.errorHandle}}`;
    const w = this.w;
    const h = wasm((m) => m.dieHandle(w, this.truthValue));
    if (temps === null) return `{"$take":${h}}`;
    temps.push(h);
    return `{"$h":${h}}`;
  }

  /** `Die.equals(other, *, simplify=False)`: as {@link Die.equals}; the truth value does not matter, only the
   * outcomes and quantities.
   *
   * @example
   * ```ts
   * d(2).eq(1).equals(new Die([false, true])); //=> true
   * d(6).eq(d(6)).equals(new Die([true])); //=> false
   * ```
   */
  override equals(other: unknown, options: { simplify?: boolean } = {}): boolean {
    // `isinstance(other, Die)` comes first: a result whose data raised is never compared with a non-die.
    if (!(other instanceof Die)) return false;
    return super.equals(other, options);
  }

  /** Python's `bool(die == x)` / `bool(die != x)`: whether the dice compared are (or are not) equal, as
   * {@link Die.equals}. In Python, this is what makes `==` between dice usable in an `if`; other dice have no
   * truth value.
   *
   * @example
   * ```ts
   * d(6).eq(d(6)).truth(); //=> true
   * d(6).eq(d(6).add(0)).truth(); //=> true
   * d(6).eq(d(4)).truth(); //=> false
   * d(6).ne(d(4)).truth(); //=> true
   * ```
   */
  truth(): boolean {
    return this.truthValue;
  }

  /** @internal */
  protected override get typeName(): string {
    return "DieWithTruth";
  }
}

/** Frees the kept handle of a collected `DieWithTruth` whose data raised. */
const failedRegistry = new FinalizationRegistry<number>((h) => wasm((w) => w.release(h)));

// The bridge's dice become typed dice.
registerKind("die", (h) => {
  try {
    return Die.wrap(wasm((w) => w.WasmDie.fromHandle(h)));
  } finally {
    wasm((w) => w.release(h));
  }
});
registerKind("die_truth", (h, truth) => {
  let core: WasmDie;
  try {
    core = wasm((w) => w.WasmDie.fromHandle(h));
  } catch {
    return DieWithTruth.failed(h, truth);
  }
  wasm((w) => w.release(h));
  return DieWithTruth.wrapTruth(core, truth);
});

// --- Making dice --------------------------------------------------------------------------------------------------

/** A core die made for a constructor (as opposed to an existing `Die` the constructor returns as it is). */
class WasmDieBox {
  readonly die: WasmDie;
  constructor(die: WasmDie) {
    this.die = die;
  }
}

/** `Die(outcomes, times, again options)`: a new core die, an existing die (Python returns `Die([die])` itself), or
 * the bridge's result for what the core cannot take directly. */
function makeDie(outcomes: DieOutcomes, times: Int | readonly Int[] | undefined, again: AgainOptions): WasmDieBox | Die {
  const { againCount, againDepth, againEnd } = again;
  const hasAgain = againCount !== undefined || againDepth !== undefined || againEnd !== undefined;
  const native = !hasAgain && nativeDieArgs(outcomes, times);
  if (native) {
    const [args, weights] = native;
    // A single die with weight 1 is returned as it is.
    if (args.length === 1 && args[0] instanceof Die && BigInt(weights[0]) === 1n) return args[0];
    const builder = wasm((w) => new w.DieBuilder());
    try {
      args.forEach((arg, i) => {
        const weight = weights[i];
        if (arg instanceof Die) wasm(() => builder.die(arg.w, weight));
        else if (arg === Reroll || arg === Restart) wasm(() => builder.reroll(weight));
        else wasm(() => builder.outcome(arg, weight));
      });
      return new WasmDieBox(wasm(() => builder.build()));
    } finally {
      builder.free();
    }
  }
  return bridgeDie(outcomes, times, again, hasAgain);
}

/** The outcomes and weights, when the core can take them directly: an array of plain values, dice and
 * `Reroll`s, or a Map of plain keys (strings or numbers, which a JS Map and a Python dict key alike), with int
 * times. `false` for anything else (Again, tuples holding dice, odd times), which the bridge handles. */
function nativeDieArgs(outcomes: DieOutcomes, times: Int | readonly Int[] | undefined): [unknown[], Int[]] | false {
  let keys: unknown[];
  let base: Int[] | null = null;
  if (outcomes instanceof Map) {
    if (times !== undefined) return false;
    keys = [...outcomes.keys()];
    base = [...outcomes.values()];
    const plainKey = (k: unknown) => typeof k === "string" || (typeof k === "number" && !Number.isNaN(k));
    if (!keys.every(plainKey) || !base.every((q) => isInt(q) && q >= 0)) return false;
  } else if (Array.isArray(outcomes)) {
    keys = outcomes;
  } else {
    return false;
  }
  const arg = (x: unknown) => x instanceof Die || x === Reroll || x === Restart || isPlainValue(x);
  if (!keys.every(arg)) return false;
  let weights: Int[];
  if (times === undefined) weights = keys.map(() => 1n);
  else if (isInt(times)) weights = keys.map(() => times);
  else if (Array.isArray(times) && times.length === keys.length && times.every(isInt)) weights = [...times];
  else return false;
  if (!weights.every((t) => t >= 0)) return false;
  if (base !== null) weights = weights.map((t, i) => BigInt(t) * BigInt((base as Int[])[i]));
  return [keys, weights];
}

/** `Die(...)` through the bridge (the runner's nodes do Python's juggling for the cases above). */
function bridgeDie(outcomes: DieOutcomes, times: Int | readonly Int[] | undefined, again: AgainOptions, hasAgain: boolean): Die {
  const nul = <X>(x: X | undefined): X | null => (x === undefined ? null : x);
  if (outcomes instanceof Map) {
    if (times !== undefined || hasAgain) throw new TypeError("icepool: a Map of quantities takes no times or again");
    return call("weights", { items: List.from(Array.from(outcomes, ([k, v]) => [k, v])) }) as Die;
  }
  const xs = list(outcomes);
  if (hasAgain) {
    if (times !== undefined) throw new TypeError("icepool: times together with again options are not supported");
    const { againCount, againDepth, againEnd } = again;
    if (againCount !== undefined && againCount !== null) {
      return call("again_count", { outcomes: xs, count: againCount, depth: nul(againDepth), end: nul(againEnd) }) as Die;
    }
    return call("again", { outcomes: xs, depth: nul(againDepth), end: nul(againEnd) }) as Die;
  }
  if (times !== undefined) return call("die_times", { outcomes: xs, times: list(times) }) as Die;
  return call("die", { outcomes: xs }) as Die;
}

/** `icepool.Die(outcomes, times, options)` without `new`: the same arguments and result as `new Die(...)`.
 *
 * @example
 * ```ts
 * die([1, 2, 2]).repr(); //=> "Die({1: 1, 2: 2})"
 * die([1, 2], [1, 3]).repr(); //=> "Die({1: 1, 2: 3})"
 * die(new Map([[1, 1], [2, 2]])).equals(die([1, 2, 2])); //=> true
 * ```
 */
export function die(outcomes: DieOutcomes, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions): Die {
  return new Die(outcomes, times, options);
}

/** `icepool.d(sides)`: a fair die numbered 1 to `sides`. `d(0)` is an error.
 *
 * @example
 * ```ts
 * d(6).outcomes(); //=> [1, 2, 3, 4, 5, 6]
 * d(6).denominator(); //=> 6n
 * d(0); //=> throws ValueError
 * ```
 */
export function d(sides: Int): Die<number> {
  return Die.wrap(wasm((w) => w.WasmDie.d(sides))) as Die<number>;
}
/** `icepool.z(sides)`: a fair die numbered 0 to `sides - 1`. `z(0)` is an error.
 *
 * @example
 * ```ts
 * z(6).outcomes(); //=> [0, 1, 2, 3, 4, 5]
 * z(10).mean().toString(); //=> "9/2"
 * z(0); //=> throws ValueError
 * ```
 */
export function z(sides: Int): Die<number> {
  return Die.wrap(wasm((w) => w.WasmDie.z(sides))) as Die<number>;
}
