// icepool's free functions (function.py, map_tools/, population/again.py, from_rv), and Python's operators on any
// operands. INTERIM (task C6): through the bridge (rpc.ts) unless noted.

import { Die, DieWithTruth, list, opts, pyBool, unpacked, type Callback, type MapResult, type Operand, type Repl } from "./die.js";
import type { MultisetExpression } from "./multiset.js";
import { Handle, call, callWith, enc, kwargsText } from "./rpc.js";
import { Break, Fraction, List, Symbols, Vector, valueTruth, type Float, type Int, type Outcome } from "./values.js";
import { binary, round, unary, type BinaryName, type UnaryName } from "./operators.js";
import { wasm } from "./wasm.js";

// --- Dice ---------------------------------------------------------------------------------------------------------

/** `icepool.coin(n, d=1, max_denominator=None)`: a die that rolls `true` with probability `n / d`, and `false`
 * otherwise. If `n <= 0` or `n >= d` the result has only one outcome.
 *
 * @param n - An int numerator, or a non-integer probability (a float or a Fraction).
 * @param d - An int denominator. Must not be given if `n` is not an int.
 * @param options - `maxDenominator`: for a non-int `n`, the probability is first limited to this denominator
 *   (Python's `Fraction.limit_denominator`).
 * @throws IcepoolError `ValueError` if `d` is given with a non-int `n`.
 * @example
 * ```ts
 * coin(1, 3).items(); //=> [[false, 2n], [true, 1n]]
 * coin(0.25).repr(); //=> "Die({False: 3, True: 1})"
 * coin(0.3333, 1, { maxDenominator: 10 }).repr(); //=> "Die({False: 2, True: 1})"
 * coin(1, 1).outcomes(); //=> [true]
 * ```
 */
export function coin(n: Int | number | Fraction, d: Int = 1, { maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
  return call("coin", { n, d, max_denominator: maxDenominator }) as Die;
}
/** `icepool.one_hot(sides)`: a die of `Vector` outcomes with one element set to `true` uniformly at random and the
 * rest `false`.
 *
 * This is an easy (if somewhat expensive) way of representing how many dice in a pool rolled each number: the
 * outcomes of `ops.matmul(10, oneHot(6))` are the `(ones, twos, threes, fours, fives, sixes)` rolled in 10d6.
 *
 * @example
 * ```ts
 * oneHot(3).probability(new Vector([false, true, false])).toString(); //=> "1/3"
 * // How many ones, twos and threes 2d3 rolled.
 * ops.matmul(2, oneHot(3)).probability(new Vector([1, 1, 0])).toString(); //=> "2/9"
 * ```
 */
export function oneHot(sides: Int): Die {
  return call("one_hot", { sides }) as Die;
}
/** `icepool.stochastic_round(x, max_denominator=None)`: randomly rounds a value up or down to the nearest integer
 * according to the two distances.
 *
 * Specifically, rounds `x` up with probability `x - floor(x)` and down otherwise, producing a die with up to two
 * outcomes.
 *
 * @param options - `maxDenominator`: if given, the rounding probability is limited to this denominator (Python's
 *   `Fraction.limit_denominator`); otherwise it is exact.
 * @example
 * ```ts
 * stochasticRound(2.25).repr(); //=> "Die({2: 3, 3: 1})"
 * stochasticRound(new Fraction(7, 2)).items(); //=> [[3, 1n], [4, 1n]]
 * ```
 */
export function stochasticRound(x: number | Fraction | Float, { maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
  return call("stochastic_round", { x, max_denominator: maxDenominator }) as Die;
}
/** `icepool.from_cumulative(outcomes, cumulative, reverse=False)`: a die from a sequence of cumulative values.
 *
 * @param outcomes - The outcomes of the resulting die. Sorted order is recommended but not necessary.
 * @param cumulative - The cumulative values (inclusive) of the outcomes in the order they are given: int
 *   cumulative quantities, or dice (of booleans) representing the cumulative distribution at that point.
 * @param options - `reverse`: if true, both arrays are reversed first. This allows e.g. constructing a die from a
 *   survival distribution.
 * @throws IcepoolError `TypeError` if the cumulative values are neither ints nor dice.
 * @example
 * ```ts
 * fromCumulative([1, 2, 3], [1, 3, 6]).repr(); //=> "Die({1: 1, 2: 2, 3: 3})"
 * // The same die from the survival distribution (quantities >= each outcome).
 * fromCumulative([1, 2, 3], [6, 5, 3], { reverse: true }).repr(); //=> "Die({1: 1, 2: 2, 3: 3})"
 * // A coin flip is the chance of rolling <= 1.
 * fromCumulative([1, 2], [coin(1, 2), coin(1, 1)]).repr(); //=> "Die({1: 1, 2: 1})"
 * ```
 */
export function fromCumulative(outcomes: readonly Outcome[], cumulative: readonly (Int | Die)[], { reverse = false }: { reverse?: boolean } = {}): Die {
  return call("from_cumulative", { outcomes: list(outcomes), cumulative: list(cumulative), reverse }) as Die;
}

/** A random variable as `fromRv` takes it (duck-typed like `scipy.stats`): `cdf` of a sequence; a `pdf` makes it
 * continuous.
 *
 * @example
 * ```ts
 * import { fromRv, type RandomVariable } from "icepool-wasm";
 * // A discrete uniform distribution on 1..4.
 * const uniform4: RandomVariable = { cdf: (xs) => xs.map((x) => Math.min(Math.max(Number(x), 0), 4) / 4) };
 * fromRv(uniform4, [1, 2, 3, 4], 4).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 1})"
 * ```
 */
export interface RandomVariable {
  /** The cumulative distribution at each of `xs` (an array), as probabilities; `kwargs` are `fromRv`'s, when there
   * are any. */
  cdf(xs: Outcome[], kwargs?: { [kwarg: string]: unknown }): Iterable<number | Fraction | Float>;
  /** Present (with any value) on a continuous distribution, as `hasattr(rv, 'pdf')` in Python. */
  pdf?: unknown;
}

/** `icepool.from_rv(rv, outcomes, denominator, **kwargs)`: a die from a random variable's CDF, as Python's, which
 * duck-types `scipy.stats` objects: `rv.cdf(xs, kwargs)` takes an array and returns an iterable of probabilities,
 * called once; an rv with a `pdf` property is continuous (the CDF is taken at the midpoints between the outcomes).
 * Each quantity is the CDF times `denominator`, rounded, so some outcomes may be omitted if their probability is
 * too small compared to the denominator.
 *
 * @param outcomes - The outcomes of the resulting die: ints if the distribution is discrete.
 * @param denominator - The denominator of the resulting die.
 * @param kwargs - Passed to `rv.cdf` as a trailing options object, when there are any.
 * @example
 * ```ts
 * // A discrete uniform distribution on 1..4.
 * const uniform4 = { cdf: (xs: number[]) => xs.map((x) => Math.min(Math.max(x, 0), 4) / 4) };
 * fromRv(uniform4, [1, 2, 3, 4], 4).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 1})"
 * // A continuous uniform distribution on [0, 1], cut at the midpoints 0.25 and 0.75.
 * const unit = { cdf: (xs: number[]) => xs.map((x) => Math.min(Math.max(x, 0), 1)), pdf: true };
 * fromRv(unit, [0, 0.5, 1], 4).repr(); //=> "Die({0: 1, 0.5: 2, 1: 1})"
 * ```
 */
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

/** `icepool.map(repl, *args, star=None, repeat=None, **kwargs)`: applies `f(outcomeOfDie0, outcomeOfDie1, ...)` for
 * all joint outcomes, returning a die. See {@link mapFunction} for a decorator-like version.
 *
 * `map` is flexible but not very efficient for more than a few dice. If at all possible, use {@link reduce},
 * multiset expression methods and/or evaluators. Even `Pool.expand()` (which sorts rolls) is more efficient than
 * `map` on the dice in order. `Again` can be used, but not combined with `repeat`.
 *
 * In JS, `x + d(6)` is not dice arithmetic: inside `f`, combine dice with their methods (`d(6).add(x)`).
 *
 * @param f - One of:
 *   - a function taking one outcome per argument and producing a new outcome;
 *   - a `Map` from old outcomes to new ones (Python's dict; unmapped outcomes stay the same), with exactly one
 *     argument.
 *   As with the `Die` constructor, the new outcomes may be dice; `Reroll` rerolls that old outcome; arrays
 *   containing dice are `tupleize`d into dice of tuples.
 * @param rest - The arguments, then an optional options object. `f` is called with all joint outcomes of the
 *   arguments: a single outcome is passed as it is, a die sends each of its outcomes, a multiset expression each
 *   sorted tuple of outcomes (as its `expand()`). Options:
 *   - `star`: if true, the first argument's outcomes are unpacked before being given to `f`. If not given it is
 *     inferred from `f.length` (as Python counts the required positional parameters) and the number of
 *     arguments.
 *   - `repeat`: if given, `map` is repeated this many times with the same arguments on the result, except that
 *     the first argument is replaced by the previous result: the result of a Markov process. It stops early once
 *     every state is absorbing (only leads to itself); `Break(state)` forces an absorption, `Reroll` rerolls the
 *     current step, `Restart` restarts the process (conditioning against that sequence of transitions).
 *     EXPERIMENTAL: `Infinity` gives the result as if repeated forever, in simplest form.
 *   - Any other option is passed to a function `f` as a keyword argument: a trailing options object. Unlike the
 *     arguments, dice are passed to it as they are. Since `f.length` counts that parameter, pass `star: false` (or
 *     give the parameter a default, `{ bonus } = ...`) so that it is not taken as a starred `f`.
 * @example
 * ```ts
 * // The higher of two d6, as highest(d(6), d(6)).
 * map((a, b) => Math.max(a, b), d(6), d(6)).probability(6).toString(); //=> "11/36"
 * // `f` takes two parameters but gets one argument, so the tuple outcomes of `tupleize` are unpacked (star).
 * map((a, b) => a * b, tupleize(d(2), d(3))).outcomes(); //=> [1, 2, 3, 4, 6]
 * // A keyword argument, reaching `f` as a trailing options object.
 * map((x, { bonus }) => x + bonus, d(6), { bonus: 2, star: false }).outcomes(); //=> [3, 4, 5, 6, 7, 8]
 * // A Map: unmapped outcomes stay the same.
 * map(new Map([[1, 6]]), d(6)).repr(); //=> "Die({2: 1, 3: 1, 4: 1, 5: 1, 6: 2})"
 * // Add d2 until the total reaches 3.
 * map((x) => (x >= 3 ? x : d(2).add(x)), 0, { repeat: Infinity }).repr(); //=> "Die({3: 5, 4: 3})"
 * ```
 */
export function map(f: Repl, ...rest: unknown[]): Die {
  const [args, { star = null, repeat, ...kwargs }] = opts<{ star: boolean | null; repeat: Int; [kwarg: string]: unknown }>(rest);
  if (repeat === undefined && typeof f === "function") {
    const mapped = mapCallable(f as (...args: unknown[]) => unknown, args, star, kwargs);
    if (mapped !== undefined) return mapped;
  }
  const a = List.from(args);
  if (repeat !== undefined) {
    return call("map_repeat", { f, args: a, star, repeat: repeat === Infinity ? "inf" : repeat }) as Die;
  }
  if (Object.keys(kwargs).length) {
    return call("map_kw", { f, args: a, star, kwargs: List.from(Object.entries(kwargs)) }) as Die;
  }
  return call("map", { f, args: a, star }) as Die;
}

/** `icepool.map(f, *args, star=star, **kwargs)` for a JS function `f` (map_tools/function.py, core_impl.py
 * `map_simple`): `f` is called with every joint outcome of the arguments (dice expand into their outcomes, zero
 * quantities included, the first argument slowest; anything else is passed as it is), unpacking the first if
 * `star` (inferred from `f.length`, as Python counts parameters), and the results become `Die(results,
 * quantities)`: dice among them are mixed in, `Reroll` drops out, and a `Break` is unwrapped. `undefined` where
 * the bridge must do it (multiset expression or deck arguments, an outcome a starred `f` cannot unpack). */
function mapCallable(
  f: (...args: unknown[]) => unknown,
  args: unknown[],
  star: unknown,
  kwargs: Record<string, unknown>,
): Die | undefined {
  // Keyword arguments reach `f` as a trailing options object, when there are any.
  const call = (xs: unknown[]) => (Object.keys(kwargs).length ? f(...xs, kwargs) : f(...xs));
  // Without arguments: `Die([repl(**kwargs)])`, where a `Break` is not unwrapped.
  if (args.length === 0) return new Die([call([]) as MapResult]);
  if (args.some((a) => a instanceof Handle)) return undefined;
  const starred = star === null || star === undefined ? f.length > args.length : pyBool(star);
  const items = args.map((a): [unknown, bigint][] => (a instanceof Die ? a.items() : [[a, 1n]]));
  if (starred && !items[0].every(([o]) => unpacked(o) !== undefined)) return undefined;
  const results: MapResult[] = [];
  const quantities: bigint[] = [];
  for (const [outcomes, quantity] of cartesianProduct(items)) {
    const xs = starred ? [...(unpacked(outcomes[0]) as unknown[]), ...outcomes.slice(1)] : outcomes;
    let result = call(xs);
    // `Break()` stands for the first outcome (before unpacking), `Break(x)` for `x`.
    if (result instanceof Break) result = result.outcome === null ? outcomes[0] : result.outcome;
    results.push(result as MapResult);
    quantities.push(quantity);
  }
  return new Die(results, quantities);
}

/** `itertools.product` of `[outcome, quantity]` lists: each joint outcome with the product of the quantities,
 * the last list varying fastest. */
function* cartesianProduct(items: [unknown, bigint][][]): Generator<[unknown[], bigint]> {
  if (items.length === 0) {
    yield [[], 1n];
    return;
  }
  const [first, ...rest] = items;
  for (const [outcome, quantity] of first) {
    for (const [outcomes, q] of cartesianProduct(rest)) yield [[outcome, ...outcomes], quantity * q];
  }
}

/** `icepool.map_and_time(repl, initial_state, *extra_args, star=None, repeat)`: repeatedly maps outcomes of the
 * state to other outcomes, while also counting time steps. This is useful for representing processes.
 *
 * The outcomes of the result are `[state, time]`, where `time` is the number of repeats needed to reach an
 * absorbing outcome (one that only leads to itself), or `repeat`, whichever is less. It returns early at a fixed
 * point, so `repeat` can be the maximum time you could be interested in without extra computation.
 *
 * @param f - A function returning a new outcome for each old outcome, or a `Map` from old outcomes to new ones
 *   (unmapped outcomes stay the same). New outcomes may be dice; `Reroll` rerolls that old outcome.
 * @param initial - The initial state: a single state or a die.
 * @param rest - Extra arguments, as for {@link map} (rerolled at every time step), then an options object:
 *   `star` as for `map`, and `repeat`, the maximum number of steps (an int; Python requires it).
 * @example
 * ```ts
 * // Add d2 until the total reaches 3: the final total and how many d2 it took.
 * const process = mapAndTime((x) => (x >= 3 ? x : d(2).add(x)), 0, { repeat: 10 });
 * process.items(); //=> [[[3, 2], 4n], [[3, 3], 1n], [[4, 2], 2n], [[4, 3], 1n]]
 * ```
 */
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

/** `icepool.mean_time_to_absorb(repl, initial_state, *extra_args, star=None)` (EXPERIMENTAL in icepool): the mean
 * time for the process to reach an absorbing state, one that maps to itself with probability 1.
 *
 * @param f - As for {@link mapAndTime}; `Restart` is not supported.
 * @param initial - The initial state: a single state or a die.
 * @param rest - Extra arguments, as for {@link map} (rerolled at every time step), then optionally `{ star }`.
 * @example
 * ```ts
 * // Rolling a d6 until it shows a 6 takes 6 rolls on average.
 * meanTimeToAbsorb((x) => (x === 6 ? x : d(6)), 0).toString(); //=> "6"
 * meanTimeToAbsorb((x) => (x >= 3 ? x : d(2).add(x)), 0).toString(); //=> "9/4"
 * ```
 */
export function meanTimeToAbsorb(f: Callback<MapResult>, initial: Operand, ...rest: unknown[]): Fraction {
  const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
  return call("mean_time_to_absorb", { f, initial, args: List.from(args), star }) as Fraction;
}

/** `icepool.map_to_pool(f, ...args, { star })` (EXPERIMENTAL in icepool): `f`'s result for each joint outcome of
 * the args (dice expand, anything else as it is) is a pool (what `new Pool(...)` takes, a `List` of dice or
 * outcomes, or a multiset expression; `Reroll` drops the outcome); the result is the mixture of those pools.
 * `f` may also be a `Map` from old outcomes to pools, with exactly one argument. `star` is as for {@link map}.
 *
 * The result is not technically a `Pool`, though it supports most of the same operations.
 *
 * @throws IcepoolError `ValueError` if the denominator cannot be made consistent with the mixture of pools.
 * @example
 * ```ts
 * // Roll d2 to choose 1d6 or 2d6, keep the highest.
 * const best = mapToPool((n: number) => Array(n).fill(d(6)), d(2)).highest(1).sum();
 * best.probability(6).toString(); //=> "17/72"
 * ```
 */
export function mapToPool(f: Repl, ...rest: unknown[]): MultisetExpression {
  const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
  return call("map_to_pool", { f, args: List.from(args), star }) as MultisetExpression;
}

/** `icepool.map_function(f, { star, ...kwargs })`: turns a function of outcomes into a function of dice, a reusable
 * {@link map} (decorator form: `star` and the keyword arguments are bound). The result of a call is a die. This
 * produces behaviour similar to AnyDice functions, though with different typing rules among other differences.
 *
 * @example
 * ```ts
 * const larger = mapFunction((a: number, b: number) => Math.max(a, b));
 * larger(d(6), d(6)).equals(highest(d(6), d(6))); //=> true
 * // Bound keyword arguments reach `f` as a trailing options object.
 * const times10 = mapFunction((x: number, { k }: { k: number }) => x * k, { star: false, k: 10 });
 * times10(d(4)).outcomes(); //=> [10, 20, 30, 40]
 * ```
 */
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

/** `icepool.reduce(function, dice, initial=None)`: applies a function of two arguments cumulatively to a sequence of
 * dice, from left to right, like JS's `Array.prototype.reduce`.
 *
 * @param f - Takes an outcome from each of two dice and produces an outcome of the same type. It may also return
 *   `Reroll`, in which case the entire sequence is effectively rerolled.
 * @param options - `initial`: if given, placed at the front of the sequence.
 * @example
 * ```ts
 * reduce((a: number, b: number) => Math.max(a, b), [d(6), d(6), d(6)]).equals(highest(d(6), d(6), d(6))); //=> true
 * reduce((a: number, b: number) => a + b, [d(4), d(4)], { initial: 10 }).outcomes()[0]; //=> 12
 * ```
 */
export function reduce(f: Callback<MapResult>, dice: readonly Operand[], { initial = null }: { initial?: Operand | null } = {}): Die {
  return call("reduce", { f, dice: list(dice), initial }) as Die;
}
/** `icepool.accumulate(function, dice, initial=None)`: as {@link reduce}, but returning each intermediate result
 * (Python's `itertools.accumulate`, with no default function and `reduce`'s parameter order). There is one result
 * per die, and one more if `initial` is given.
 *
 * @example
 * ```ts
 * const totals = accumulate((a: number, b: number) => a + b, [d(2), d(2)]);
 * totals.map((die) => die.repr()); //=> ["Die({1: 1, 2: 1})", "Die({2: 1, 3: 2, 4: 1})"]
 * ```
 */
export function accumulate(f: Callback<MapResult>, dice: readonly Operand[], { initial = null }: { initial?: Operand | null } = {}): Die[] {
  return call("accumulate", { f, dice: list(dice), initial }) as Die[];
}

// --- Several dice -------------------------------------------------------------------------------------------------

/** The options of {@link highest} and {@link lowest}. `keep` and `drop` work together: with neither, the single
 * highest (lowest) die is taken; with only `keep`, the `keep` highest dice are summed; with only `drop`, the `drop`
 * highest dice are dropped and the rest summed; with both, `drop` dice are dropped, then the next `keep` summed.
 *
 * @example
 * ```ts
 * import { d, highest, type KeepOptions } from "icepool-wasm";
 * const options: KeepOptions = { keep: 3 };
 * highest(d(6), d(6), d(6), d(6), options).mean().toString(); //=> "15869/1296"
 * ```
 */
export interface KeepOptions {
  /** The number of dice to sum (after dropping). */
  keep?: Int | null;
  /** The number of dice dropped first, from the kept end. */
  drop?: Int | null;
  /** The outcome of the result when no dice are given (else that throws `ValueError`). */
  default?: Outcome;
}
/** The options of {@link middle}.
 *
 * @example
 * ```ts
 * import { d, highest, middle, type MiddleOptions } from "icepool-wasm";
 * const options: MiddleOptions = { tie: "high" };
 * middle(d(6), d(6), options).equals(highest(d(6), d(6))); //=> true
 * ```
 */
export interface MiddleOptions {
  /** The number of middle outcomes to sum (default 1). */
  keep?: Int;
  /** When `keep` and the number of dice differ in parity: `"error"` (default) throws `IndexError`, `"high"` takes
   * the higher candidate, `"low"` the lower. */
  tie?: "error" | "high" | "low";
  /** The outcome of the result when no dice are given (else that throws `ValueError`). */
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
/** `icepool.highest(*args, keep=None, drop=None, default=None)`: the highest outcome among the rolls, or the sum of
 * some of the highest (see {@link KeepOptions}). The arguments are dice or single outcomes, then an optional
 * options object. The outcomes should support addition and multiplication if `keep` is not 1.
 *
 * @throws IcepoolError `ValueError` if no dice are given and no `default`.
 * @example
 * ```ts
 * // 4d6 keep highest 3.
 * highest(d(6), d(6), d(6), d(6), { keep: 3 }).mean().toString(); //=> "15869/1296"
 * highest(d(6), 3).outcomes(); //=> [3, 4, 5, 6]
 * highest(d(6), d(6), d(6), { drop: 1 }).equals(lowest(d(6), d(6), d(6), { keep: 2 })); //=> true
 * highest(); //=> throws ValueError
 * ```
 */
export const highest: (...args: (Operand | KeepOptions)[]) => Die = keepFunction("highest");
/** `icepool.lowest(*args, keep=None, drop=None, default=None)`: the lowest outcome among the rolls, or the sum of
 * some of the lowest (see {@link KeepOptions}). The arguments are dice or single outcomes, then an optional
 * options object. The outcomes should support addition and multiplication if `keep` is not 1.
 *
 * @throws IcepoolError `ValueError` if no dice are given and no `default`.
 * @example
 * ```ts
 * // Rolling 2d20 with disadvantage.
 * lowest(d(20), d(20)).mean().toString(); //=> "287/40"
 * lowest({ default: 0 }).outcomes(); //=> [0]
 * ```
 */
export const lowest: (...args: (Operand | KeepOptions)[]) => Die = keepFunction("lowest");
/** `icepool.middle(*args, keep=1, tie='error', default=None)`: the middle outcome among the rolls, or the sum of
 * some of the middle (see {@link MiddleOptions}). The arguments are dice or single outcomes, then an optional
 * options object.
 *
 * @throws IcepoolError `ValueError` if no dice are given and no `default`; `IndexError` if the middle is
 *   ambiguous and `tie` is `"error"`.
 * @example
 * ```ts
 * middle(d(20), d(20), d(20)).mean().toString(); //=> "21/2"
 * middle(d(6), d(6)); //=> throws IndexError
 * middle(d(6), d(6), { tie: "low" }).equals(lowest(d(6), d(6))); //=> true
 * ```
 */
export const middle: (...args: (Operand | MiddleOptions)[]) => Die = keepFunction("middle");

/** `icepool.tupleize(*args)`: the Cartesian product of the arguments (dice and single outcomes) as a die of tuples
 * (arrays). If no argument is a die, the result is a plain tuple, as in Python.
 *
 * @example
 * ```ts
 * tupleize(d(6), 0).outcomes().slice(0, 2); //=> [[1, 0], [2, 0]]
 * tupleize(d(2), d(2)).repr(); //=> "Die({(1, 1): 1, (1, 2): 1, (2, 1): 1, (2, 2): 1})"
 * ```
 */
export function tupleize(...args: Operand[]): Die {
  return call("tupleize", { args: List.from(args) }) as Die;
}
/** `icepool.vectorize(*args)`: the Cartesian product of the arguments (dice and single outcomes) as a die of
 * `Vector`s, which add, multiply and compare element by element. If no argument is a die, the result is a plain
 * `Vector`, as in Python.
 *
 * @example
 * ```ts
 * vectorize(d(2), d(2)).probability(new Vector([1, 2])).toString(); //=> "1/4"
 * // Two d6 rolls together with a count of the rolls.
 * vectorize(d(6), 1).add(vectorize(d(6), 1)).probability(new Vector([7, 2])).toString(); //=> "1/6"
 * ```
 */
export function vectorize(...args: Operand[]): Die {
  return call("vectorize", { args: List.from(args) }) as Die;
}
/** `icepool.pointwise_max(*args)`: selects the highest chance of rolling >= each outcome among the dice (naming not
 * finalized in icepool).
 *
 * For each outcome, the chance of the result rolling >= that outcome is the highest chance of rolling >= it among
 * the arguments; equivalently, any quantile of the result is the highest of that quantile among the arguments.
 * This is useful for choosing between moves when trying to reach a threshold that is known but could change with
 * the situation.
 *
 * @example
 * ```ts
 * pointwiseMax(d(4).add(1), d(6)).repr(); //=> "Die({2: 3, 3: 3, 4: 2, 5: 2, 6: 2})"
 * ```
 */
export function pointwiseMax(...dice: Die[]): Die {
  return call("pointwise", { args: List.from(dice), max: true, spread: true }) as Die;
}
/** `icepool.pointwise_min(*args)`: selects the highest chance of rolling <= each outcome among the dice (naming not
 * finalized in icepool).
 *
 * For each outcome, the chance of the result rolling <= that outcome is the highest chance of rolling <= it among
 * the arguments; equivalently, any quantile of the result is the lowest of that quantile among the arguments.
 * This is useful for choosing between moves when trying to stay under a threshold that is known but could change.
 *
 * @example
 * ```ts
 * pointwiseMin(d(4).add(1), d(6)).repr(); //=> "Die({1: 2, 2: 2, 3: 2, 4: 3, 5: 3})"
 * ```
 */
export function pointwiseMin(...dice: Die[]): Die {
  return call("pointwise", { args: List.from(dice), max: false, spread: true }) as Die;
}
/** `icepool.max_outcome(*args)`: the maximum possible outcome among the dice and single outcomes.
 *
 * @example
 * ```ts
 * maxOutcome(d(6), d(8), 3); //=> 8
 * ```
 */
export function maxOutcome(...args: Operand[]): Outcome {
  return call("extreme_outcome", { args: List.from(args), max: true, spread: true }) as Outcome;
}
/** `icepool.min_outcome(*args)`: the minimum possible outcome among the dice and single outcomes.
 *
 * @example
 * ```ts
 * minOutcome(d(6), 0); //=> 0
 * ```
 */
export function minOutcome(...args: Operand[]): Outcome {
  return call("extreme_outcome", { args: List.from(args), max: false, spread: true }) as Outcome;
}
/** `icepool.consecutive(*args)`: a minimal sequence of consecutive ints covering the argument sets (empty if they
 * are all empty).
 *
 * @example
 * ```ts
 * consecutive([1, 5], [3, 8]); //=> [1, 2, 3, 4, 5, 6, 7, 8]
 * ```
 */
export function consecutive(...args: readonly Int[][]): Int[] {
  return call("consecutive", { args: List.from(args.map(list)) }) as Int[];
}
/** `icepool.sorted_union(*args)`: merges sets of outcomes into a sorted sequence without duplicates.
 *
 * @example
 * ```ts
 * sortedUnion([3, 1], [2, 3]); //=> [1, 2, 3]
 * ```
 */
export function sortedUnion(...args: readonly Outcome[][]): Outcome[] {
  return call("sorted_union", { args: List.from(args.map(list)) }) as Outcome[];
}
/** `icepool.harmonize_denominators(dice, weights=None)`: scales the quantities of the dice so that the
 * denominators are proportional to the given weights.
 *
 * @param dice - Dice, or single outcomes (made dice).
 * @param weights - The target relative denominators. If not given, all dice are scaled to the same denominator.
 * @example
 * ```ts
 * harmonizeDenominators([d(4), d(6)]).map((die) => die.denominator()); //=> [12n, 12n]
 * harmonizeDenominators([d(4), d(6), 1], [1, 2, 1]).map((die) => die.denominator()); //=> [12n, 24n, 12n]
 * ```
 */
export function harmonizeDenominators(dice: readonly Operand[], weights: readonly Int[] | null = null): Die[] {
  return call("harmonize_denominators", { dice: list(dice), weights: list(weights) }) as Die[];
}
/** `icepool.format_probability_inverse(probability, int_start=20)` (EXPERIMENTAL in icepool): formats the inverse
 * of a probability as "1 in N", or "n/a" outside (0, 1].
 *
 * @param intStart - If N = 1 / probability is between this and a million times this, it is formatted as an
 *   integer; otherwise as a float with precision at least 1 part in `intStart`.
 * @example
 * ```ts
 * formatProbabilityInverse(new Fraction(1, 36)); //=> "1 in 36"
 * formatProbabilityInverse(0.3); //=> "1 in 3.3"
 * formatProbabilityInverse(1e-9); //=> "1 in 1.00e+09"
 * formatProbabilityInverse(2); //=> "n/a"
 * ```
 */
export function formatProbabilityInverse(probability: number | Fraction, intStart: Int = 20): string {
  return call("format_probability_inverse", { probability, args: List.of(intStart) }) as string;
}

/** Python's `bool(x)` of a Vector, Symbols, a `DieWithTruth` or an `Again` expression (from `eq` / `ne`): JS has no
 * operator overloading, so where Python writes `if die_a == die_b:`, JS writes `if (truth(dieA.eq(dieB)))`.
 *
 * @example
 * ```ts
 * truth(d(6).eq(d(6))); //=> true
 * truth(d(6).eq(d(6).add(1))); //=> false
 * truth(new Vector([1, 2]).eq(new Vector([1, 3]))); //=> false
 * ```
 */
export function truth(x: unknown): boolean {
  if (x instanceof DieWithTruth) return x.truth();
  if (x instanceof Vector || x instanceof Symbols) return valueTruth(x);
  return call("truth", { x }) as boolean;
}

// --- Python's operators on any operands ---------------------------------------------------------------------------

const BINARY: BinaryName[] = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor", "lt", "le", "gt", "ge", "eq", "ne"];
const UNARY: UnaryName[] = ["neg", "pos", "abs", "invert", "trunc", "floor", "ceil"];

/** Python's operators on any operands (plain values, dice, Again expressions): `ops.add(3, die)` is `3 + die`.
 * Useful where the left operand may be a plain value, which has no methods, and for Python semantics on plain
 * numbers (floor division, modulo, banker's rounding).
 *
 * Its members, each Python's operator of the same name:
 * - `add(a, b)`: `a + b`; `sub`: `a - b`; `mul`: `a * b`; `truediv`: `a / b`; `floordiv`: `a // b`; `mod`: `a % b`;
 *   `pow`: `a ** b`.
 * - `lshift`: `a << b`; `rshift`: `a >> b`; `and`: `a & b`; `or`: `a | b`; `xor`: `a ^ b`.
 * - `lt`: `a < b`; `le`: `a <= b`; `gt`: `a > b`; `ge`: `a >= b`; `eq`: `a == b`; `ne`: `a != b` (on dice, a die
 *   of booleans; see {@link truth}).
 * - `neg(a)`: `-a`; `pos`: `+a`; `abs`: `abs(a)`; `invert`: `~a`; `trunc`, `floor`, `ceil`: `math.trunc`,
 *   `math.floor`, `math.ceil`.
 * - `round(a, ndigits)`: `round(a, ndigits)`; `matmul(n, die)`: `n @ die`, the sum of `n` rolls of `die`.
 *
 * @example
 * ```ts
 * // The members return `unknown`: assert the type you expect.
 * (ops.add(3, d(6)) as Die).equals(d(6).add(3)); //=> true
 * ops.floordiv(-7, 2); //=> -4
 * ops.mod(-7, 3); //=> 2
 * ops.round(2.5); //=> 2
 * ops.matmul(3, d(6)).equals(d(6).add(d(6)).add(d(6))); //=> true
 * (ops.neg(d(4)) as Die).outcomes(); //=> [-4, -3, -2, -1]
 * ```
 */
export const ops = {
  ...(Object.fromEntries(BINARY.map((op) => [op, (a: unknown, b: unknown) => binary(op, a, b)])) as Record<
    BinaryName,
    (a: unknown, b: unknown) => unknown
  >),
  ...(Object.fromEntries(UNARY.map((op) => [op, (a: unknown) => unary(op, a)])) as Record<UnaryName, (a: unknown) => unknown>),
  /** `round(a, ndigits)`. */
  round(a: unknown, ndigits: Int | null = null): unknown {
    return round(a, ndigits);
  },
  /** `n @ die`. */
  matmul(n: unknown, die: unknown): Die {
    if (die instanceof Die) return die.rmatmul(n as Int | Die);
    if (n instanceof Die) return n.matmul(die as Operand);
    return call("matmul", { n, die }) as Die;
  },
};
