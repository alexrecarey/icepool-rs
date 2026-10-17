// icepool's free functions (function.py, map_tools/, population/again.py, from_rv), and Python's operators on any
// operands. INTERIM (task C6): through the bridge (rpc.ts) unless noted.

import { Die, DieWithTruth, list, opts, type Callback, type MapResult, type Operand, type Repl } from "./die.js";
import type { MultisetExpression } from "./multiset.js";
import { call, callWith, enc, kwargsText } from "./rpc.js";
import { Fraction, List, Symbols, Vector, valueTruth, type Float, type Int, type Outcome } from "./values.js";
import { wasm } from "./wasm.js";

// --- Dice ---------------------------------------------------------------------------------------------------------

/** A coin with probability `n / d` of `true`; `n` may also be a float or a Fraction. */
export function coin(n: Int | number | Fraction, d: Int = 1, { maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
  return call("coin", { n, d, max_denominator: maxDenominator }) as Die;
}
/** A die of one-hot tuples: `sides` outcomes, each a tuple with one `true`. */
export function oneHot(sides: Int): Die {
  return call("one_hot", { sides }) as Die;
}
/** `x` rounded up with probability `x - floor(x)`, else down. */
export function stochasticRound(x: number | Fraction | Float, { maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
  return call("stochastic_round", { x, max_denominator: maxDenominator }) as Die;
}
/** A die from cumulative quantities (or dice representing the cumulative distribution at each outcome). */
export function fromCumulative(outcomes: readonly Outcome[], cumulative: readonly (Int | Die)[], { reverse = false }: { reverse?: boolean } = {}): Die {
  return call("from_cumulative", { outcomes: list(outcomes), cumulative: list(cumulative), reverse }) as Die;
}

/** A random variable as `fromRv` takes it (duck-typed like `scipy.stats`): `cdf` of a sequence; a `pdf` makes it
 * continuous. */
export interface RandomVariable {
  cdf(xs: Outcome[], kwargs?: { [kwarg: string]: unknown }): Iterable<number | Fraction | Float>;
  pdf?: unknown;
}

/** `icepool.from_rv(rv, outcomes, denominator, **kwargs)`: a die from a random variable's CDF, as Python's, which
 * duck-types `scipy.stats` objects: `rv.cdf(xs, kwargs)` takes an array (a `List`) and returns an iterable of
 * probabilities, called once; an rv with a `pdf` property is continuous (the CDF is taken at the midpoints between
 * the outcomes). `kwargs` reach `cdf` as a trailing options object, when there are any. */
export function fromRv(rv: RandomVariable, outcomes: readonly Outcome[], denominator: Int, kwargs: { [kwarg: string]: unknown } = {}): Die {
  if (rv === null || typeof rv !== "object" || typeof rv.cdf !== "function") {
    throw new TypeError("icepool: fromRv takes an object with a cdf method");
  }
  // `hasattr(rv, 'pdf')`.
  const pdf = "pdf" in rv;
  const cdf = (...args: [Outcome[], { [kwarg: string]: unknown }?]) => rv.cdf(...args);
  return callWith((temps) => {
    const text = `{"cdf":${enc(cdf, temps)},"pdf":${pdf},"outcomes":${enc(outcomes, temps)},"denominator":${enc(denominator, temps)},"kwargs":${kwargsText(kwargs, temps)}}`;
    return wasm((w) => w.from_rv(text));
  }) as Die;
}

// --- map and friends ----------------------------------------------------------------------------------------------

/** `icepool.map(f, ...args, { star, repeat, ...kwargs })`: `repeat` an int or Infinity; any other option is passed
 * to `f` as a keyword argument (a trailing options object in JS). */
export function map(f: Repl, ...rest: unknown[]): Die {
  const [args, { star = null, repeat, ...kwargs }] = opts<{ star: boolean | null; repeat: Int; [kwarg: string]: unknown }>(rest);
  const a = List.from(args);
  if (repeat !== undefined) {
    return call("map_repeat", { f, args: a, star, repeat: repeat === Infinity ? "inf" : repeat }) as Die;
  }
  if (Object.keys(kwargs).length) {
    return call("map_kw", { f, args: a, star, kwargs: List.from(Object.entries(kwargs)) }) as Die;
  }
  return call("map", { f, args: a, star }) as Die;
}

/** `icepool.map_and_time(f, initial, ...args, { star, repeat })`: a die of `[state, time to absorb]`. */
export function mapAndTime(f: Callback<MapResult>, initial: Operand, ...rest: unknown[]): Die {
  const [args, { star = null, repeat = "inf" }] = opts<{ star: boolean | null; repeat: Int | "inf" }>(rest);
  return call("map_and_time", {
    f,
    initial,
    args: List.from(args),
    star,
    repeat: repeat === Infinity ? "inf" : repeat,
  }) as Die;
}

/** `icepool.mean_time_to_absorb(f, initial, ...args, { star })`: the expected number of steps to absorb. */
export function meanTimeToAbsorb(f: Callback<MapResult>, initial: Operand, ...rest: unknown[]): Fraction {
  const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
  return call("mean_time_to_absorb", { f, initial, args: List.from(args), star }) as Fraction;
}

/** `icepool.map_to_pool(f, ...args, { star })` (EXPERIMENTAL in icepool): `f`'s result for each joint outcome of
 * the args (dice expand, anything else as it is) is a pool (what `new Pool(...)` takes, a `List` of dice or
 * outcomes, or a multiset expression; `Reroll` drops the outcome); the result is the mixture of those pools. */
export function mapToPool(f: Repl, ...rest: unknown[]): MultisetExpression {
  const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
  return call("map_to_pool", { f, args: List.from(args), star }) as MultisetExpression;
}

/** `icepool.map_function(f, { star, ...kwargs })`: `f` as a function of dice (decorator form: `star` is kept). */
export function mapFunction(f: Callback<MapResult>, { star = null, ...kwargs }: { star?: boolean | null; [kwarg: string]: unknown } = {}): (...args: unknown[]) => Die {
  return (...args: unknown[]) =>
    call("map_function", {
      f,
      args: List.from(args),
      star,
      kwargs: List.from(Object.entries(kwargs)),
      decorator: true,
    }) as Die;
}

/** `icepool.reduce(f, dice, initial=)`: `f` applied cumulatively to the dice' outcomes. */
export function reduce(f: Callback<MapResult>, dice: readonly Operand[], { initial = null }: { initial?: Operand | null } = {}): Die {
  return call("reduce", { f, dice: list(dice), initial }) as Die;
}
/** `icepool.accumulate(f, dice, initial=)`: each intermediate result of `reduce`. */
export function accumulate(f: Callback<MapResult>, dice: readonly Operand[], { initial = null }: { initial?: Operand | null } = {}): Die[] {
  return call("accumulate", { f, dice: list(dice), initial }) as Die[];
}

// --- Several dice -------------------------------------------------------------------------------------------------

/** The options of `highest` and `lowest`. */
export interface KeepOptions {
  keep?: Int | null;
  drop?: Int | null;
  default?: Outcome;
}
/** The options of `middle`. */
export interface MiddleOptions {
  keep?: Int;
  tie?: "error" | "high" | "low";
  default?: Outcome;
}

function keepFunction(op: "highest" | "lowest" | "middle") {
  return (...rest: unknown[]): Die => {
    const [args, o] = opts<KeepOptions & MiddleOptions>(rest);
    const nul = <X>(x: X | undefined): X | null => (x === undefined ? null : x);
    const extra = op === "middle" ? { keep: o.keep ?? 1, tie: o.tie ?? "error" } : { keep: nul(o.keep), drop: nul(o.drop) };
    return call(op, { args: List.from(args), ...extra, default: nul(o.default) }) as Die;
  };
}
/** `highest(...dice, { keep, drop, default })`: the sum of the highest rolls among the dice. */
export const highest: (...args: (Operand | KeepOptions)[]) => Die = keepFunction("highest");
/** `lowest(...dice, { keep, drop, default })`: the sum of the lowest rolls among the dice. */
export const lowest: (...args: (Operand | KeepOptions)[]) => Die = keepFunction("lowest");
/** `middle(...dice, { keep, tie, default })`. */
export const middle: (...args: (Operand | MiddleOptions)[]) => Die = keepFunction("middle");

/** A die of tuples from dice and outcomes. */
export function tupleize(...args: Operand[]): Die {
  return call("tupleize", { args: List.from(args) }) as Die;
}
/** A die of Vectors from dice and outcomes. */
export function vectorize(...args: Operand[]): Die {
  return call("vectorize", { args: List.from(args) }) as Die;
}
/** The die whose CDF is the pointwise maximum of the dice' CDFs. */
export function pointwiseMax(...dice: Die[]): Die {
  return call("pointwise", { args: List.from(dice), max: true, spread: true }) as Die;
}
/** The die whose CDF is the pointwise minimum of the dice' CDFs. */
export function pointwiseMin(...dice: Die[]): Die {
  return call("pointwise", { args: List.from(dice), max: false, spread: true }) as Die;
}
/** The highest outcome among dice and outcomes. */
export function maxOutcome(...args: Operand[]): Outcome {
  return call("extreme_outcome", { args: List.from(args), max: true, spread: true }) as Outcome;
}
/** The lowest outcome among dice and outcomes. */
export function minOutcome(...args: Operand[]): Outcome {
  return call("extreme_outcome", { args: List.from(args), max: false, spread: true }) as Outcome;
}
/** The integers from the lowest to the highest of the arguments' ints. */
export function consecutive(...args: readonly Int[][]): Int[] {
  return call("consecutive", { args: List.from(args.map(list)) }) as Int[];
}
/** The sorted union of the arguments' outcomes. */
export function sortedUnion(...args: readonly Outcome[][]): Outcome[] {
  return call("sorted_union", { args: List.from(args.map(list)) }) as Outcome[];
}
/** The dice (outcomes are made dice) scaled to a common denominator, in proportion to the weights. */
export function harmonizeDenominators(dice: readonly Operand[], weights: readonly Int[] | null = null): Die[] {
  return call("harmonize_denominators", { dice: list(dice), weights: list(weights) }) as Die[];
}
/** A probability as "1 in N" text. */
export function formatProbabilityInverse(probability: number | Fraction, intStart: Int = 20): string {
  return call("format_probability_inverse", { probability, args: List.of(intStart) }) as string;
}

/** Python's `bool(x)` of a Vector, Symbols, a `DieWithTruth` or an `Again` expression (from `eq` / `ne`). */
export function truth(x: unknown): boolean {
  if (x instanceof DieWithTruth) return x.truth();
  if (x instanceof Vector || x instanceof Symbols) return valueTruth(x);
  return call("truth", { x }) as boolean;
}

// --- Python's operators on any operands ---------------------------------------------------------------------------

type BinaryName =
  | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
  | "lt" | "le" | "gt" | "ge" | "eq" | "ne";
type UnaryName = "neg" | "pos" | "abs" | "invert" | "trunc" | "floor" | "ceil";

const BINARY: BinaryName[] = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor", "lt", "le", "gt", "ge", "eq", "ne"];
const UNARY: UnaryName[] = ["neg", "pos", "abs", "invert", "trunc", "floor", "ceil"];

/** Python's operators on any operands (plain values, dice, Again expressions): `ops.add(3, die)` is `3 + die`.
 * A die on the left is its method; anything else goes through the bridge, which does Python's dispatch. */
export const ops = {
  ...(Object.fromEntries(
    BINARY.map((op) => [op, (a: unknown, b: unknown): unknown => (a instanceof Die ? a[op](b as Operand) : call(op, { a, b }))]),
  ) as Record<BinaryName, (a: unknown, b: unknown) => unknown>),
  ...(Object.fromEntries(
    UNARY.map((op) => [op, (a: unknown): unknown => (a instanceof Die ? a[op]() : call(op, { a }))]),
  ) as Record<UnaryName, (a: unknown) => unknown>),
  /** `round(a, ndigits)`. */
  round(a: unknown, ndigits: Int | null = null): unknown {
    return a instanceof Die ? a.round(ndigits) : call("round", { a, ndigits });
  },
  /** `n @ die`. */
  matmul(n: unknown, die: unknown): Die {
    if (die instanceof Die) return die.rmatmul(n as Int | Die);
    if (n instanceof Die) return n.matmul(die as Operand);
    return call("matmul", { n, die }) as Die;
  },
};
