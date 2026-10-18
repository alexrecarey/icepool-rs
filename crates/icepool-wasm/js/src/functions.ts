// icepool's free functions (function.py, expand.py, map_tools/, from_rv), and Python's operators on any operands.
// As the Python facade does (python/icepool/function.py), the TypeScript does each one's argument juggling and calls
// typed exports (crates/icepool-wasm/src/api/function.rs) for the work. INTERIM (task C6): `repeat`, `mapAndTime`,
// `meanTimeToAbsorb`, `mapToPool`, `fromRv` and `highest` / `lowest` / `middle` still go through the bridge (rpc.ts).

import { AgainExpression } from "./again.js";
import { Die, dictItems, opts, pyBool, unpack, type AgainOptions, type Callback, type MapResult, type Operand, type Repl } from "./die.js";
import { IcepoolError } from "./errors.js";
import type { MultisetExpression } from "./multiset.js";
import { Handle, call } from "./rpc.js";
import { Break, Fraction, List, Reroll, Restart, Vector, pyTypeName, type Float, type Int, type Outcome } from "./values.js";
import { binary, matmul, round, unary, type BinaryName, type UnaryName } from "./operators.js";
import { isTrap, wasm } from "./wasm.js";
import type { WasmDie } from "../pkg-node/icepool_wasm.js";

/** Python's `isinstance(x, int)` (a bool is one). */
const isPyInt = (x: unknown): x is Int | boolean => typeof x === "bigint" || typeof x === "boolean" || Number.isInteger(x);

/** `implicit_convert_to_die(x)`: a die as it is, anything else `Die([x])`; an Again expression cannot be. */
function toDie(x: unknown): Die {
  if (x instanceof Die) return x;
  if (x instanceof AgainExpression) {
    throw new IcepoolError("WrongType", "ImplicitConversionError", "Again expression cannot be implicitly converted to a Die.");
  }
  return new Die([x as MapResult]);
}

/** A die argument for a typed export (`DiceArgs`): each die and plain outcome, in order. */
function diceArgs(args: readonly unknown[]): import("../pkg-node/icepool_wasm.js").DiceArgs {
  const built = wasm((w) => new w.DiceArgs());
  try {
    for (const arg of args) {
      if (arg instanceof Die) wasm(() => built.die(arg.w));
      else wasm(() => built.outcome(arg));
    }
  } catch (e) {
    built.free();
    throw e;
  }
  return built;
}

/** Runs `f` on a `DiceArgs` of `args`, then frees it. */
function withDiceArgs<T>(args: readonly unknown[], f: (built: import("../pkg-node/icepool_wasm.js").DiceArgs) => T): T {
  const built = diceArgs(args);
  try {
    return wasm(() => f(built));
  } finally {
    built.free();
  }
}

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
  if (!isPyInt(n)) {
    if (pyBool(binary("ne", d, 1))) {
      throw new IcepoolError("InvalidArgument", "ValueError", "If a non-int numerator is provided, a denominator must not be provided.");
    }
    // `Fraction(n)`, limited to `max_denominator` if given.
    [n, d] = wasm((w) => w.fractionParts(n, maxDenominator)) as [Int, Int];
  }
  return new Die(new Map(wasm((w) => w.coinData(n, d)) as [Outcome, Int][]));
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
  return Die.wrap(wasm((w) => w.oneHot(sides)));
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
  const integerPart = unary("floor", x);
  const fractionalPart = binary("sub", x, integerPart);
  return binary("add", integerPart, coin(fractionalPart as number, 1, { maxDenominator })) as Die;
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
  if (outcomes.length === 0) return new Die(new Map());
  if (reverse) [outcomes, cumulative] = [[...outcomes].reverse(), [...cumulative].reverse()];
  let prev: unknown = 0;
  const d = new Map<unknown, unknown>();
  if (cumulative.length === 0) throw new IcepoolError("IndexOutOfRange", "IndexError", "list index out of range");
  const first = cumulative[0];
  if (first instanceof Die) {
    const dice = harmonizeDenominators(cumulative as Operand[]);
    outcomes.forEach((outcome, i) => {
      if (i >= dice.length) return;
      const quantity = dice[i].quantity("!=", false);
      d.set(outcome, binary("sub", quantity, prev));
      prev = quantity;
    });
  } else if (isPyInt(first)) {
    outcomes.forEach((outcome, i) => {
      if (i >= cumulative.length) return;
      d.set(outcome, binary("sub", cumulative[i], prev));
      prev = cumulative[i];
    });
  } else {
    throw new IcepoolError("WrongType", "TypeError", "Unsupported type <class 'list'> for cumulative values.");
  }
  return new Die(d as Map<unknown, Int>);
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
  const cdf = (xs: Outcome[]) => Array.from(Object.keys(kwargs).length ? rv.cdf(xs, kwargs) : rv.cdf(xs));
  // `int(round(x * denominator))`: Python's `round` of a float or Fraction is an int.
  const quantity = (x: unknown) => round(binary("mul", x, denominator)) as Int;
  let quantitiesLe: Int[];
  // `hasattr(rv, 'pdf')`: continuous distributions use the midpoints between the outcomes.
  if ("pdf" in rv) {
    const midpoints = outcomes.slice(1).map((b, i) => binary("truediv", binary("add", outcomes[i], b), 2) as Outcome);
    quantitiesLe = [...cdf(midpoints).map(quantity), denominator];
  } else {
    quantitiesLe = cdf([...outcomes]).map(quantity);
  }
  return fromCumulative(outcomes, quantitiesLe);
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
  const [args, options] = opts<{ star: boolean | null; repeat: Int; [kwarg: string]: unknown } & AgainOptions>(rest);
  const { star = null, repeat = null, againCount = null, againDepth = null, againEnd = null, ...kwargs } = options;
  if (repeat === null) {
    const mapped = mapSimple(f, args, star, kwargs, { againCount, againDepth, againEnd });
    if (mapped !== undefined) return mapped;
    if (againCount !== null || againDepth !== null || againEnd !== null) {
      throw new IcepoolError("Unported", "NotImplementedError", "map of multiset expressions or decks with again options");
    }
  }
  // With `repeat` Python ignores the again options.
  if (repeat !== null) {
    const repeated = mapRepeat(f, args, star, kwargs, repeat);
    if (repeated !== undefined) return repeated;
  }
  const a = List.from(args);
  if (repeat !== null) {
    return call("map_repeat", { f, args: a, star, repeat: repeat === Infinity ? "inf" : repeat }) as Die;
  }
  if (Object.keys(kwargs).length) {
    return call("map_kw", { f, args: a, star, kwargs: List.from(Object.entries(kwargs)) }) as Die;
  }
  return call("map", { f, args: a, star }) as Die;
}

/** map_tools/core_impl.py `map_simple(repl, *args, star=star, **kwargs)` with the again options: `f` (a JS
 * function, or a Map: `mapping.get(outcome, outcome)`, as common.py `transition_and_star` makes it) is called with
 * every joint outcome of the arguments (dice expand into their outcomes, zero quantities included, the first
 * argument slowest; anything else is passed as it is), unpacking the first if `star` (inferred from `f.length`, as
 * Python counts parameters), and the results become `Die(results, quantities, again options)`: dice among them are
 * mixed in, `Reroll` drops out, Again expressions are rolled again, and a `Break` is unwrapped. `undefined` where
 * the bridge must do it (multiset expression or deck arguments: task C6i). */
function mapSimple(
  f: Repl,
  args: unknown[],
  star: unknown,
  kwargs: Record<string, unknown>,
  again: AgainOptions,
): Die | undefined {
  const hasKwargs = Object.keys(kwargs).length > 0;
  if (args.length === 0) {
    if (f instanceof Map) throw new IcepoolError("InvalidArgument", "ValueError", "If no arguments are given, repl must be a callable.");
    // `Die([repl(**kwargs)])`, where a `Break` is not unwrapped.
    if (typeof f !== "function") throw new IcepoolError("WrongType", "TypeError", `'${pyTypeName(f)}' object is not callable`);
    const fn = f as (...args: unknown[]) => unknown;
    return new Die([(hasKwargs ? fn(kwargs) : fn()) as MapResult]);
  }
  if (args.some((a) => a instanceof Handle)) return undefined;
  const items = args.map((a): [unknown, bigint][] => (a instanceof Die ? a.items() : [[a, 1n]]));
  let transition: (xs: unknown[]) => unknown;
  let starred = false;
  if (f instanceof Map) {
    if (args.length !== 1) throw new IcepoolError("InvalidArgument", "ValueError", "If a mapping is provided for repl, len(args) must be 1.");
    if (hasKwargs) {
      throw new IcepoolError("WrongType", "TypeError", `<lambda>() got an unexpected keyword argument '${Object.keys(kwargs)[0]}'`);
    }
    const values = mappingGet(f, items[0].map(([o]) => o));
    transition = (xs) => values.get(xs[0]);
  } else if (typeof f === "function") {
    const fn = f as (...args: unknown[]) => unknown;
    // Keyword arguments reach `f` as a trailing options object, when there are any.
    transition = (xs) => (hasKwargs ? fn(...xs, kwargs) : fn(...xs));
    starred = star === null || star === undefined ? fn.length > args.length : pyBool(star);
  } else {
    throw notARepl();
  }
  const results: MapResult[] = [];
  const quantities: bigint[] = [];
  for (const [outcomes, quantity] of jointItems(items)) {
    const xs = starred ? [...unpack(outcomes[0]), ...outcomes.slice(1)] : outcomes;
    let result = transition(xs);
    // `Break()` stands for the first outcome (before unpacking), `Break(x)` for `x`.
    if (result instanceof Break) result = result.outcome === null ? outcomes[0] : result.outcome;
    results.push(result as MapResult);
    quantities.push(quantity);
  }
  return new Die(results, quantities, again);
}

/** `mapping.get(outcome, outcome)` for each of `outcomes`, a Python dict lookup (keys compared with Python's `==`
 * and hash: `1`, `1.0` and `true` are one key), as a JS Map from each outcome (by identity) to its value. @internal */
export function mappingGet(mapping: Map<unknown, unknown>, outcomes: unknown[]): Map<unknown, unknown> {
  const [keys, values] = dictItems(mapping);
  // An outcome is never equal to a die or an Again expression: only the other keys can match.
  const plain = keys.flatMap((k, i) => (k instanceof Die || k instanceof AgainExpression ? [] : [i]));
  const found = wasm((w) => w.dictLookup(plain.map((i) => keys[i]), outcomes));
  return new Map(outcomes.map((o, j) => [o, found[j] < 0 ? o : values[plain[found[j]]]]));
}

/** common.py `transition_and_star`'s error for a `repl` that is neither a function nor a Map. */
const notARepl = () => new IcepoolError("WrongType", "TypeError", "repl must be a callable or a mapping.");

/** `itertools.product` of `[outcome, quantity]` lists: each joint outcome with the product of the quantities,
 * the last list varying fastest. */
function* jointItems(items: [unknown, bigint][][]): Generator<[unknown[], bigint]> {
  if (items.length === 0) {
    yield [[], 1n];
    return;
  }
  const [first, ...rest] = items;
  for (const [outcome, quantity] of first) {
    for (const [outcomes, q] of jointItems(rest)) yield [[outcome, ...outcomes], quantity * q];
  }
}

/** `map(repl, *args, repeat=repeat)` on the core's `TransitionCache`: Python's checks on `repeat`, then the Markov
 * process from `Die([first_arg])` (`Infinity` / `'inf'`: the absorbing Markov chain). `undefined` where the bridge
 * must do it (multiset expression or deck arguments: task C6i). */
function mapRepeat(f: Repl, args: unknown[], star: unknown, kwargs: Record<string, unknown>, repeat: unknown): Die | undefined {
  if (args.length === 0) throw new IcepoolError("InvalidArgument", "ValueError", "If no arguments are given, repeat cannot be used.");
  const [first, ...extra] = args;
  const cache = first instanceof Handle ? undefined : transitionCache(f, extra, star, kwargs);
  if (cache === undefined) return undefined;
  if (repeat === Infinity || repeat === "inf") return absorbingMarkovChain(cache, first)[0];
  if (pyBool(binary("lt", repeat, 0))) throw new IcepoolError("InvalidArgument", "ValueError", "repeat cannot be negative.");
  if (pyBool(binary("eq", repeat, 0))) return new Die([first as MapResult]);
  const initialDie = new Die([first as MapResult]);
  return Die.wrap(wasm((w) => w.mapRepeat(cache.transition, cache.quantities, initialDie.w, rangeRepeat(repeat))));
}

/** markov_chain.py `absorbing_markov_chain_impl(transition_cache, initial_state)`: the absorption die and the mean
 * absorption time (`null` if a `Restart` was possible). */
function absorbingMarkovChain(cache: TransitionCacheArgs, initial: unknown): [Die, Fraction | null] {
  const initialDie = new Die([initial as MapResult]);
  const [die, time] = wasm((w) => w.absorbingMarkovChain(cache.transition, cache.quantities, initialDie.w)) as [WasmDie, Fraction | null];
  return [Die.wrap(die), time];
}

/** `range(repeat)`'s count for the core: an int (negative: none; past the core's `u32`, as many as it takes to
 * converge), `undefined` for anything else, where Python's `range` raises `TypeError`. */
function rangeRepeat(repeat: unknown): number | undefined {
  if (typeof repeat === "boolean") return Number(repeat);
  if (!isPyInt(repeat)) return undefined;
  return Math.max(0, Math.min(Number(repeat), 2 ** 32 - 1));
}

/** What the core's `TransitionCache` takes: the transition object and the joint quantities of the extra
 * arguments. */
interface TransitionCacheArgs {
  transition: { step(state: unknown, i: number): unknown[] };
  quantities: bigint[];
}

/** map_tools `TransitionCache(repl, *extra_args, star=star, **kwargs)` for the core: `step(state, i)` calls `repl`
 * (a function, unpacking the state if `star`; or a Map, `mapping.get(state, state)`, as common.py
 * `transition_and_star` makes it) on the state and the `i`th joint outcome of the extra arguments, keyword
 * arguments as a trailing options object, and tells the core what it returned. `undefined` where the bridge must
 * do it (multiset expression or deck arguments: task C6i). */
function transitionCache(f: Repl, extraArgs: unknown[], star: unknown, kwargs: Record<string, unknown>): TransitionCacheArgs | undefined {
  if (extraArgs.some((a) => a instanceof Handle)) return undefined;
  const joint = [...jointItems(extraArgs.map((a): [unknown, bigint][] => (a instanceof Die ? a.items() : [[a, 1n]])))];
  const hasKwargs = Object.keys(kwargs).length > 0;
  let transition: (state: unknown, extra: unknown[]) => unknown;
  if (f instanceof Map) {
    if (extraArgs.length !== 0) throw new IcepoolError("InvalidArgument", "ValueError", "If a mapping is provided for repl, len(args) must be 1.");
    transition = (state) => {
      if (hasKwargs) throw new IcepoolError("WrongType", "TypeError", `<lambda>() got an unexpected keyword argument '${Object.keys(kwargs)[0]}'`);
      return mappingGet(f, [state]).get(state);
    };
  } else if (typeof f === "function") {
    const fn = f as (...args: unknown[]) => unknown;
    const starred = star === null || star === undefined ? fn.length > extraArgs.length + 1 : pyBool(star);
    transition = (state, extra) => {
      const first = starred ? unpack(state) : [state];
      return hasKwargs ? fn(...first, ...extra, kwargs) : fn(...first, ...extra);
    };
  } else {
    throw notARepl();
  }
  const step = (state: unknown, i: number): unknown[] => {
    try {
      const result = transition(state, joint[i][0]);
      if (result === Reroll) return [true, "reroll"];
      if (result === Restart) return [true, "restart"];
      if (result instanceof Break) return result.outcome === null ? [true, "break"] : [true, "break", ...transitionTarget(result.outcome)];
      return [true, "go", ...transitionTarget(result)];
    } catch (e) {
      // A trap below must abandon the WASM frames above it too.
      if (isTrap(e)) throw e;
      return [false, e];
    }
  };
  return { transition: { step }, quantities: joint.map(([, q]) => q) };
}

/** Where a transition goes, for the core: `["reroll"]`, `["die", copy]` (the core takes the copy over), or
 * `["outcome", x]`. */
function transitionTarget(x: unknown): [string, unknown] {
  if (x === Reroll || x === Restart) return ["reroll", null];
  if (x instanceof Die) return ["die", x.w.copy()];
  return ["outcome", x];
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
  const [args, { star = null, repeat = "inf", ...kwargs }] = opts<{ star: boolean | null; repeat: Int | "inf"; [kwarg: string]: unknown }>(rest);
  const cache = initial instanceof Handle ? undefined : transitionCache(f, args, star, kwargs);
  if (cache !== undefined) {
    const initialDie = new Die([initial]);
    return Die.wrap(wasm((w) => w.mapAndTime(cache.transition, cache.quantities, initialDie.w, rangeRepeat(repeat))));
  }
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
  const [args, { star = null, ...kwargs }] = opts<{ star: boolean | null; [kwarg: string]: unknown }>(rest);
  const cache = initial instanceof Handle ? undefined : transitionCache(f, args, star, kwargs);
  if (cache === undefined) return call("mean_time_to_absorb", { f, initial, args: List.from(args), star }) as Fraction;
  const time = absorbingMarkovChain(cache, initial)[1];
  if (time === null) throw new IcepoolError("NotImplementedUpstream", "NotImplementedError", "Restart not implemented for mean_time_to_absorb.");
  return time;
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
  // `partial(map, function, star=star, **kwargs)`: keyword arguments of the call override the bound ones.
  return (...rest: unknown[]) => {
    const [args, options] = opts<Record<string, unknown>>(rest);
    return map(f, ...args, { star, ...kwargs, ...options });
  };
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
  const [result, rest] = reduceStart(dice, initial);
  if (result === undefined) throw new IcepoolError("InvalidArgument", "StopIteration", "");
  return rest.reduce((acc: Die, die) => map(f, acc, die), result);
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
  const [first, rest] = reduceStart(dice, initial);
  if (first === undefined) return [];
  const results = [first];
  for (const die of rest) results.push(map(f, results[results.length - 1], die));
  return results;
}

/** The start of `reduce` / `accumulate`: `initial` (or else the first die) as a die, and the dice after it. */
function reduceStart(dice: readonly Operand[], initial: Operand | null): [Die | undefined, unknown[]] {
  const all = iterable(dice);
  if (initial !== null) return [toDie(initial), all];
  return all.length ? [toDie(all[0]), all.slice(1)] : [undefined, []];
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
  return cartesianProduct(args, "tuple") as Die;
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
  return cartesianProduct(args, "vector") as Die;
}

/** expand.py `cartesian_product(*args, outcome_type=...)`: `Reroll` (or `Restart`) if an argument is one; the
 * outcome itself (a tuple or `Vector` of the arguments) if none is a population; else the die of the joint
 * outcomes. */
function cartesianProduct(args: readonly unknown[], type: "tuple" | "vector"): unknown {
  const reroll = args.find((arg) => arg === Reroll || arg === Restart);
  if (reroll !== undefined) return reroll;
  // Decks and multiset expressions expand too: the bridge has those.
  if (args.some((arg) => arg instanceof Handle)) return call(type === "tuple" ? "tupleize" : "vectorize", { args: List.from(args) });
  if (!args.some((arg) => arg instanceof Die)) return type === "tuple" ? [...args] : new Vector(args as Outcome[]);
  return Die.wrap(withDiceArgs(args, (built) => (type === "tuple" ? built.tupleize() : built.vectorize())));
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
  return pointwise(dice, true);
}

/** `pointwise_max(arg0, *more_args)` / `pointwise_min`: the dice (a single argument is iterated over)
 * harmonized, then the core's pointwise extreme. */
function pointwise(dice: readonly Operand[], max: boolean): Die {
  if (dice.length === 0) {
    const name = max ? "pointwise_max" : "pointwise_min";
    throw new IcepoolError("WrongType", "TypeError", `${name}() missing 1 required positional argument: 'arg0'`);
  }
  const harmonized = harmonizeDenominators(dice.length === 1 ? (iterable(dice[0]) as Operand[]) : dice);
  return Die.wrap(withDiceArgs(harmonized, (built) => built.pointwise(max)));
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
  return pointwise(dice, false);
}
/** `icepool.max_outcome(*args)`: the maximum possible outcome among the dice and single outcomes.
 *
 * @example
 * ```ts
 * maxOutcome(d(6), d(8), 3); //=> 8
 * ```
 */
export function maxOutcome(...args: Operand[]): Outcome {
  return wasm((w) => w.minMax(iterOutcomes(args), true)) as Outcome;
}

/** function.py `_iter_outcomes(*args)`: the outcomes of each die among the arguments, the other arguments as they
 * are (a single argument is iterated over). */
function iterOutcomes(args: readonly unknown[]): unknown[] {
  const iterated = args.length === 1 ? iterable(args[0]) : args;
  return iterated.flatMap((arg) => (arg instanceof Die ? arg.outcomes() : [arg]));
}

/** Python's `iter(x)` for a sequence argument: an array's elements, a die's outcomes, a str's characters. */
function iterable(x: unknown): unknown[] {
  if (Array.isArray(x) || typeof x === "string") return [...x];
  if (x instanceof Die) return x.outcomes();
  throw new IcepoolError("WrongType", "TypeError", "object is not iterable");
}
/** `icepool.min_outcome(*args)`: the minimum possible outcome among the dice and single outcomes.
 *
 * @example
 * ```ts
 * minOutcome(d(6), 0); //=> 0
 * ```
 */
export function minOutcome(...args: Operand[]): Outcome {
  return wasm((w) => w.minMax(iterOutcomes(args), false)) as Outcome;
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
  // `itertools.chain(*args)`, twice: the second chain only if the first had something.
  const first = args.flatMap(iterable);
  const second = first.length ? args.flatMap(iterable) : [];
  return wasm((w) => w.consecutive(first, second)) as Int[];
}
/** `icepool.sorted_union(*args)`: merges sets of outcomes into a sorted sequence without duplicates.
 *
 * @example
 * ```ts
 * sortedUnion([3, 1], [2, 3]); //=> [1, 2, 3]
 * ```
 */
export function sortedUnion(...args: readonly Outcome[][]): Outcome[] {
  if (args.length === 0) return [];
  return wasm((w) => w.sortedUnion(args.map(iterable))) as Outcome[];
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
  const converted = iterable(dice).map(toDie);
  // Python zips the weights with the denominators twice (`zip` stops at the shorter).
  const denominators = converted.map((die) => die.denominator());
  const ws = weights === null ? converted.map(() => 1) : iterable(weights);
  const zipped = ws.slice(0, denominators.length);
  const scaleFactors = wasm((w) => w.weightedLcm(denominators, zipped, zipped));
  return converted.slice(0, scaleFactors.length).map((die, i) => die.multiplyQuantities(scaleFactors[i] as Int));
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
  return wasm((w) => w.formatProbabilityInverse(probability, intStart));
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
  return pyBool(x);
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
    return matmul(n, die) as Die;
  },
};
