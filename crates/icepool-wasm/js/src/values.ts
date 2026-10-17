// Python's value types that JS has no counterpart for, and the sentinels (`Reroll`, `Order`, ...).
//
//   int       a JS number when it is a safe integer, else a BigInt (both go in as ints)
//   float     a JS number when it is not integral; an integral float (2.0, -0.0) comes out as a `Float`, so that it
//             goes back in as a float. Any non-integral number, Infinity and NaN go in as floats.
//   tuple     a JS array (a Python list, where an argument must be one, is a `List`)
//   Fraction, complex, Vector, Symbols: the classes below. None is null.

import { call } from "./rpc.js";
import type { Die } from "./die.js";

/** Anything that can be an outcome. */
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

/** An int argument: a JS integer or a BigInt. */
export type Int = number | bigint;

/** A Python float that happens to be integral (`2.0`), so that it stays a float. Behaves as its number. */
export class Float {
  readonly value: number;
  constructor(value: number) {
    this.value = Number(value);
  }
  valueOf(): number {
    return this.value;
  }
  toString(): string {
    return pyFloatRepr(this.value);
  }
}

/** An exact fraction (Python's `Fraction`), normalised by the core when it goes in. */
export class Fraction {
  readonly numerator: bigint;
  readonly denominator: bigint;
  constructor(numerator: Int, denominator: Int = 1n) {
    this.numerator = BigInt(numerator);
    this.denominator = BigInt(denominator);
  }
  valueOf(): number {
    return Number(this.numerator) / Number(this.denominator);
  }
  toString(): string {
    return this.denominator === 1n ? `${this.numerator}` : `${this.numerator}/${this.denominator}`;
  }
}

/** A Python list, where an argument must be a list rather than a tuple (a JS array is a tuple). */
export class List<T = unknown> extends Array<T> {}

/** `icepool.Break(outcome)`, returned by a `map` function to stop the repetition (by default with the state
 * itself). */
export class Break {
  readonly outcome: Outcome | Die;
  constructor(outcome: Outcome | Die = null) {
    this.outcome = outcome;
  }
}

/** Python's `complex`: `String(z)` is Python's repr, e.g. `(1+2j)`, `-0j`. */
export class Complex {
  readonly re: number;
  readonly im: number;
  constructor(re: number, im = 0) {
    this.re = re;
    this.im = im;
  }
  toString(): string {
    return pyComplexRepr(this.re, this.im);
  }
}

/** A Python object JS has no type for: it can be read, not passed back. */
export class Opaque {
  readonly repr: string;
  constructor(repr: string) {
    this.repr = repr;
  }
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
/** `icepool.Reroll`: as an outcome, or returned by a `map` function, it drops out (the die is rerolled). */
export const Reroll = new Special("Reroll");
/** `icepool.Restart`: returned by a `map` function with `repeat`, it restarts the whole process. */
export const Restart = new Special("Restart");
/** `...`, for keep tuples: `pool.keep([Ellipsis, 1])`. */
export const Ellipsis = new Special("Ellipsis");

/** `icepool.Order`. */
export const Order = Object.freeze({ Ascending: 1, Descending: -1, Any: 0 } as const);
/** `Order.Ascending` (1), `Order.Descending` (-1) or `Order.Any` (0). */
export type OrderValue = 1 | -1 | 0;

/** Python's float repr: shortest round-trip digits, exponent form below 1e-4 or from 1e16 (CPython's 'r'). */
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

/** Python's `repr(complex)`: `(re+imj)`, or just `imj` when the real part is +0.0; no trailing `.0`. */
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
  add(other: unknown): Self;
  sub(other: unknown): Self;
  mul(other: unknown): Self;
  truediv(other: unknown): Self;
  floordiv(other: unknown): Self;
  mod(other: unknown): Self;
  pow(other: unknown): Self;
  lshift(other: unknown): Self;
  rshift(other: unknown): Self;
  and(other: unknown): Self;
  or(other: unknown): Self;
  xor(other: unknown): Self;
  lt(other: unknown): Outcome;
  le(other: unknown): Outcome;
  gt(other: unknown): Outcome;
  ge(other: unknown): Outcome;
  eq(other: unknown): Outcome;
  ne(other: unknown): Outcome;
  neg(): Self;
  pos(): Self;
  invert(): Self;
  abs(): Self;
  trunc(): Self;
  floor(): Self;
  ceil(): Self;
}

const BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor"] as const;
const COMPARE = ["lt", "le", "gt", "ge", "eq", "ne"] as const;
const UNARY = ["neg", "pos", "invert", "abs", "trunc", "floor", "ceil"] as const;

export interface Vector extends ValueOperators<Vector> {}
/** `icepool.Vector`: elementwise arithmetic; comparisons give vectors with a truth value (`truth(v)`). */
export class Vector implements Iterable<Outcome> {
  /** `null` for a `VectorWithTruthOnly` (a comparison between vectors of different lengths). */
  readonly items: Outcome[] | null;
  readonly hasTruth: boolean;
  readonly truthValue: Outcome;

  constructor(items: Iterable<Outcome>) {
    this.items = Array.from(items);
    this.hasTruth = false;
    this.truthValue = null;
  }
  /** A vector from the core: `items` null for a `VectorWithTruthOnly`. */
  static fromCore(items: Outcome[] | null, hasTruth: boolean, truthValue: Outcome): Vector {
    const v = new Vector([]);
    Object.assign(v, { items, hasTruth, truthValue });
    return v;
  }
  get length(): number {
    return this.items?.length ?? 0;
  }
  [Symbol.iterator](): Iterator<Outcome> {
    return (this.items ?? [])[Symbol.iterator]();
  }
  /** `truediv`. */
  div(other: unknown): Vector {
    return this.truediv(other);
  }
  round(ndigits: Int | null = null): Vector {
    return call("coll_unary", { a: this, name: "round", ndigits }) as Vector;
  }
  append(x: Outcome): Vector {
    return call("vector_method", { v: this, name: "append", arg: x }) as Vector;
  }
  concatenate(other: Iterable<Outcome>): Vector {
    return call("vector_method", { v: this, name: "concatenate", arg: other }) as Vector;
  }
  toString(): string {
    return call("vector_method", { v: this, name: "str", arg: null }) as string;
  }
  repr(): string {
    return call("vector_method", { v: this, name: "repr", arg: null }) as string;
  }
}

export interface Symbols extends ValueOperators<Symbols> {}
/** `icepool.Symbols`: a multiset of characters, from a string such as "aab-c". */
export class Symbols {
  /** Python's `str()`, `null` if that raises (non-int counts). */
  readonly str: string | null;
  /** The dict's `[char, count]` items in its order, once the core made it (`null` for one made from a string). */
  readonly items: readonly (readonly [string, Int])[] | null;

  constructor(str: string) {
    this.str = String(str);
    this.items = null;
  }
  /** A Symbols from the core. */
  static fromCore(items: readonly (readonly [string, Int])[], str: string | null): Symbols {
    const s = new Symbols("");
    Object.assign(s, { items, str });
    return s;
  }
  /** `Symbols(x)` from anything Python takes (an iterable of characters, a Map of counts). */
  static from(x: string | readonly string[] | Map<string, Int>): Symbols {
    return call("symbols", { x: x instanceof Map ? new SymbolsMapping(x) : x }) as Symbols;
  }
  /** Calls the method `name` of Python's Symbols. */
  method(name: string, arg: unknown = null): unknown {
    return call("symbols_method", { s: this, name, arg });
  }
  issubset(other: unknown): boolean {
    return this.method("issubset", other) as boolean;
  }
  issuperset(other: unknown): boolean {
    return this.method("issuperset", other) as boolean;
  }
  isdisjoint(other: unknown): boolean {
    return this.method("isdisjoint", other) as boolean;
  }
  countSubset(divisor: unknown, { emptyDivisor = null }: { emptyDivisor?: Int | null } = {}): Int {
    return this.method("count_subset", [divisor, emptyDivisor]) as Int;
  }
  hasNegativeCounts(): boolean {
    return this.method("has_negative_counts") as boolean;
  }
  size(): Int {
    return this.method("size") as Int;
  }
  multiplyCounts(n: Int): Symbols {
    return this.method("multiply_counts", n) as Symbols;
  }
  divideCounts(n: unknown): Symbols {
    return this.method("divide_counts", n) as Symbols;
  }
  moduloCounts(n: Int): Symbols {
    return this.method("modulo_counts", n) as Symbols;
  }
  additiveUnion(...others: unknown[]): Symbols {
    return this.method("additive_union", others) as Symbols;
  }
  difference(...others: unknown[]): Symbols {
    return this.method("difference", others) as Symbols;
  }
  intersection(...others: unknown[]): Symbols {
    return this.method("intersection", others) as Symbols;
  }
  union(...others: unknown[]): Symbols {
    return this.method("union", others) as Symbols;
  }
  symmetricDifference(other: unknown): Symbols {
    return this.method("symmetric_difference", other) as Symbols;
  }
  /** The count of a character. */
  get(c: string): Int {
    return this.method("getitem", c) as Int;
  }
  keys(): string[] {
    return this.method("keys") as string[];
  }
  values(): Int[] {
    return this.method("values") as Int[];
  }
  toString(): string {
    return this.str ?? "";
  }
  repr(): string {
    return this.method("repr") as string;
  }
}

for (const cls of [Vector, Symbols]) {
  const proto = cls.prototype as unknown as Record<string, unknown>;
  for (const op of [...BINARY, ...COMPARE]) {
    proto[op] = function (this: Vector | Symbols, other: unknown) {
      return call("coll_binary", { a: this, b: other, name: op });
    };
  }
  for (const op of UNARY) {
    proto[op] = function (this: Vector | Symbols) {
      return call("coll_unary", { a: this, name: op, ndigits: null });
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
  return call("coll_truth", { x }) as boolean;
}
