/// <reference lib="esnext.disposable" />
// TypeScript types of the JS facade (core.js; entry points node.js and web.js). Python's dynamic types map to:
//   int -> number | bigint (out: number when safe, else bigint), float -> number | Float, tuple -> array,
//   None -> null, Fraction / Vector / Symbols -> the classes below. Quantities are always bigint.

/** Anything that can be an outcome. */
export type Outcome =
  | number
  | bigint
  | string
  | boolean
  | null
  | Float
  | Fraction
  | Vector
  | Symbols
  | readonly Outcome[];

/** An int argument: a JS integer or a BigInt. */
export type Int = number | bigint;

/** What dice operators and constructors accept on the other side: an outcome, a die, or an Again expression. */
export type Operand = Outcome | Die | AgainExpression;

/** A selection of outcomes (`reroll`, `filter`, `split`, `explode`, ...): a predicate or the outcomes themselves
 * (a Map: its keys, as Python's dict). */
export type Which = ((...outcomes: any[]) => unknown) | readonly Outcome[] | Map<unknown, unknown> | null;

/** `Order.Ascending` (1), `Order.Descending` (-1) or `Order.Any` (0). */
export type OrderValue = 1 | -1 | 0;

export type Comparison = "==" | "!=" | "<=" | "<" | ">=" | ">";

/** What a `map` function may return. */
export type MapResult = Operand | typeof Reroll | typeof Restart | Break | readonly MapResult[];

/** What `map` takes: a function, or a Map (Python's dict: `mapping.get(outcome, outcome)`, keys compared with
 * Python's `==`, so `1`, `1.0` and `true` are one key). */
export type Repl = ((...args: any[]) => MapResult) | Map<unknown, MapResult> | CoreFunction;

// --- Errors -------------------------------------------------------------------------------------------------------

/** An exception icepool raised: `pythonType` is the Python exception (`ValueError`, `TypeError`, `MemoryError`, ...),
 * or `PanicException` for a bug in the core (a trap, after which the instance is still usable). */
export class IcepoolError extends Error {
  readonly pythonType: string;
  /** Python would answer, but the port does not cover the case yet. */
  readonly unimplemented: boolean;
  constructor(pythonType: string, message?: string, unimplemented?: boolean);
}

/** Throw from an evaluator method that cannot handle the given order: the evaluation retries in the other one. */
export class UnsupportedOrder extends Error {
  constructor(message?: string);
}

// --- Values -------------------------------------------------------------------------------------------------------

/** A Python float that is integral (`2.0`, `-0.0`): it stays a float when passed back. */
export class Float {
  readonly value: number;
  constructor(value: number);
  valueOf(): number;
}

/** An exact fraction (Python's `Fraction`). */
export class Fraction {
  readonly numerator: bigint;
  readonly denominator: bigint;
  constructor(numerator: Int, denominator?: Int);
  valueOf(): number;
}

/** A Python list, where an argument must be a list rather than a tuple. */
export class List<T = unknown> extends Array<T> {}

/** `icepool.Break(outcome)`: returned by a `map` function to stop repeating (default: the state itself). */
export class Break {
  readonly outcome: Outcome | Die;
  constructor(outcome?: Outcome | Die);
}

/** Python's `complex` (from a negative number to a fractional power, and arithmetic on it). `String(z)` is
 * Python's repr. */
export class Complex {
  readonly re: number;
  readonly im: number;
  constructor(re: number, im?: number);
}

/** Python's `repr(complex)`. */
export function pyComplexRepr(re: number, im: number): string;

/** A Python object JS has no type for. */
export class Opaque {
  readonly repr: string;
}

declare const special: unique symbol;
interface Special {
  readonly [special]: true;
  readonly name: string;
}
export const Reroll: Special;
export const Restart: Special;
/** `...`, for keep tuples: `pool.keep([Ellipsis, 1])`. */
export const Ellipsis: Special;

export const Order: { readonly Ascending: 1; readonly Descending: -1; readonly Any: 0 };

/** Python's `repr(float)`. */
export function pyFloatRepr(x: number): string;

/** Raw program JSON (the fixture tests' lazy arguments). */
export class RawJSON {
  constructor(text: string);
}

// --- Low level ----------------------------------------------------------------------------------------------------

/** Installs the wasm-bindgen module (node.js / web.js do it). */
export function bind(wasm: unknown): void;
/** Calls a node of the program language by name (harness/grammar/): what every method is built on. */
export function call(op: string, args?: Record<string, unknown>): any;
/** A wire-format literal (`{"float": "1.5"}`, ...) as a JS value. */
export function fromWire(text: string): any;
/** How many core objects JS holds. */
export function liveHandles(): number;
/** One program of the harness's JSON program language in, its JSON result out (one `icepool-run` line). Test-only:
 * builds with the `runner` feature; the package's build throws. */
export function run(programJson: string): string;
/** khepri's `f2f.calculate`, with the same recovery after a trap as every other call. Only builds with the `khepri`
 * feature have it; the package's build throws. */
export function calculate(engineInput: object): {
  rolls: number | bigint;
  outcomes: { player: string; wounds: number; chance: number }[];
};

/** A core object held by JS: freed when garbage-collected, or at once by `release()` (or a `using` declaration). */
export class Handle implements Disposable {
  protected constructor(token: never, handle: number);
  /** The harness's wire encoding of this object, as JSON text. */
  toWire(): string;
  /** Frees the object's WASM memory now. Using it afterwards throws a `TypeError`; releasing it again does nothing. */
  release(): void;
  /** `release()`. */
  [Symbol.dispose](): void;
}

// --- Operators ----------------------------------------------------------------------------------------------------

interface Operators<Self> {
  add(other: Operand): Die;
  sub(other: Operand): Die;
  mul(other: Operand): Die;
  truediv(other: Operand): Die;
  /** `truediv`. */
  div(other: Operand): Die;
  floordiv(other: Operand): Die;
  mod(other: Operand): Die;
  pow(other: Operand): Die;
  lshift(other: Operand): Die;
  rshift(other: Operand): Die;
  and(other: Operand): Die;
  or(other: Operand): Die;
  xor(other: Operand): Die;
  lt(other: Operand): Die;
  le(other: Operand): Die;
  gt(other: Operand): Die;
  ge(other: Operand): Die;
  /** `==`: a die of bools with a truth value (`equals`). */
  eq(other: Operand): DieWithTruth;
  ne(other: Operand): DieWithTruth;
  neg(): Self;
  pos(): Self;
  abs(): Self;
  invert(): Self;
  trunc(): Self;
  floor(): Self;
  ceil(): Self;
  round(ndigits?: Int | null): Self;
}

/** Python's operators on any operands (values, dice, Again expressions). */
export const ops: {
  [op in
    | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
    | "lt" | "le" | "gt" | "ge" | "eq" | "ne"]: (a: unknown, b: unknown) => any;
} & {
  [op in "neg" | "pos" | "abs" | "invert" | "trunc" | "floor" | "ceil"]: (a: unknown) => any;
} & {
  round(a: unknown, ndigits?: Int | null): any;
  /** `n @ die`. */
  matmul(n: Int | Die, die: Die): Die;
};

// --- Die ----------------------------------------------------------------------------------------------------------

export interface AgainOptions {
  againCount?: Int | null;
  againDepth?: Int | null;
  againEnd?: Operand | typeof Reroll | typeof Restart | null;
}

export interface Die extends Operators<Die> {}

/** `icepool.Die`. */
export class Die extends Handle implements Iterable<Outcome> {
  /** `new Die([1, 2, 3])`, `new Die([d6, d8], [2, 1])`, `new Die(new Map([[1, 2n]]))`, again options last. */
  constructor(outcomes: readonly MapResult[] | Map<Outcome, Int> | Die, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions);
  /** `n @ die`: the sum of `n` rolls. */
  static sum(n: Int | Die, die: Die): Die;

  /** `depth` is required (an int, or Infinity). */
  reroll(which?: Which, options?: { star?: boolean | null; depth: Int | "inf" }): Die;
  filter(which: Which, options?: { star?: boolean | null; depth: Int | "inf" }): Die;
  truncate(min?: Outcome, max?: Outcome): Die;
  clip(min?: Outcome, max?: Outcome): Die;
  split(which?: Which, options?: { star?: boolean | null }): [Die, Die];
  explode(which?: Which, options?: { star?: boolean | null; depth?: Int; end?: Operand | typeof Reroll | null }): Die;

  /** `map(f, die, ...extraArgs, options)`. */
  map(f: Repl, ...rest: unknown[]): Die;
  mapAndTime(f: (...args: any[]) => MapResult, ...rest: unknown[]): Die;
  meanTimeToAbsorb(f: (...args: any[]) => MapResult, ...rest: unknown[]): Fraction;
  /** The expected number of rolls to reach a sum of at least `target`. */
  meanTimeToSum(target: Int): Fraction;
  timeToSum(target: Int, maxTime?: Int | null, dnf?: Int | typeof Reroll | null): Die;
  groupBy(key: ((...args: any[]) => Outcome) | Map<unknown, Outcome>, options?: { star?: boolean | null }): Map<Outcome, Die>;
  groupByIndex(index: Int): Map<Outcome, Die>;
  isIn(outcomes: readonly Outcome[]): Die;
  count(rolls: Int, outcomes: readonly Outcome[]): Die;
  ifElse(ifTrue: Operand, ifFalse: Operand, options?: AgainOptions): Die;

  highest(rolls: Int, keep?: Int | null, drop?: Int | null): Die;
  lowest(rolls: Int, keep?: Int | null, drop?: Int | null): Die;
  middle(rolls: Int, keep?: Int, options?: { tie?: "error" | "high" | "low" }): Die;
  pool(rolls?: Int | readonly Int[]): Pool;
  /** `keep(counts)` (one count per sorted roll), or `keep(rolls, index)`: an int index (that sorted roll) or counts. */
  keep(rolls: Int | readonly Int[], index?: Int | readonly (Int | typeof Ellipsis)[] | null): Die;
  /** `die.keep(rolls, slice(start, stop, step))`. */
  keepSlice(rolls: Int, start?: Int | null, stop?: Int | null, step?: Int | null): Die;
  /** Every ordered sequence of `rolls` rolls, as a die of arrays (`[]` itself for 0 rolls). */
  sequence(rolls: Int): Die;

  /** `mapToPool(f, die, ...extraArgs)`; `f` null: the outcomes are the pools' contents. */
  mapToPool(f?: Repl | null, ...rest: unknown[]): MultisetExpression;
  explodeToPool(rolls?: Int, outcomes?: Which, options?: { star?: boolean | null; depth?: Int }): MultisetExpression;
  rerollToPool(
    rolls: Int,
    outcomes: Which | undefined,
    options: { maxRerolls: Int; star?: boolean | null; depth?: Int; mode?: "random" | "low" | "high" | "drop" },
  ): MultisetExpression;

  simplify(): Die;
  zero(): Die;
  zeroOutcome(): Outcome;
  sign(): Die;
  cmp(other: Operand): Die;
  padToDenominator(denominator: Int, outcome: Outcome): Die;
  multiplyToDenominator(denominator: Int): Die;
  multiplyQuantities(scale: Int): Die;
  divideQuantities(divisor: Int): Die;
  moduloQuantities(divisor: Int): Die;
  equals(other: unknown, options?: { simplify?: boolean }): boolean;
  /** Each outcome rounded up with probability `x - floor(x)`, else down. */
  stochasticRound(options?: { maxDenominator?: Int | null }): Die;

  items(): [Outcome, bigint][];
  outcomes(): Outcome[];
  keys(): Outcome[];
  values(): bigint[];
  /** The quantity of an outcome (KeyError if absent). */
  get(outcome: Outcome): bigint;
  has(outcome: Outcome): boolean;
  /** The number of outcomes. */
  readonly size: number;
  [Symbol.iterator](): Iterator<Outcome>;

  mean(): Fraction | number;
  variance(): Fraction | number;
  standardDeviation(): number;
  sd(): number;
  skewness(): number;
  excessKurtosis(): number;
  mode(): Outcome[];
  modalQuantity(): bigint;
  median(): Outcome;
  medianLow(): Outcome;
  medianHigh(): Outcome;
  minOutcome(): Outcome;
  maxOutcome(): Outcome;
  denominator(): bigint;
  isEmpty(): boolean;
  quantile(n: Int, d?: Int): Outcome;
  quantileLow(n: Int, d?: Int): Outcome;
  quantileHigh(n: Int, d?: Int): Outcome;
  standardizedMoment(k: Int): number;
  entropy(base?: number | Float): number;
  kolmogorovSmirnov(other: Die): Fraction;
  cramerVonMises(other: Die): Fraction;
  covariance(i: Int, j: Int): Fraction | number;
  correlation(i: Int, j: Int): number;
  nearest(comparison: Comparison, outcome: Outcome): Outcome;
  commonOutcomeLength(): number | null;
  /** `quantity(outcome)` or `quantity(comparison, outcome)`. */
  quantity(...args: [Outcome] | [Comparison, Outcome]): bigint;
  quantities(comparison?: Comparison | null): bigint[];
  probability(...args: [Outcome] | [Comparison, Outcome] | [Outcome, { percent?: boolean }] | [Comparison, Outcome, { percent?: boolean }]): Fraction | number;
  probabilities(comparison?: Comparison | null, options?: { percent?: boolean }): (Fraction | number)[];
  quantityWhere(which: (...args: any[]) => unknown, options?: { star?: boolean | null }): bigint;
  probabilityWhere(which: (...args: any[]) => unknown, options?: { star?: boolean | null; percent?: boolean }): Fraction | number;

  marginals(): Die[];
  marginal(index: Int): Die;
  marginalSlice(start?: Int | null, stop?: Int | null, step?: Int | null): Die;

  sample(options?: { seed?: Int }): Outcome;
  sample(options: { seed?: Int; n: Int }): Outcome[];
  /** A Markdown / BBCode / CSV / HTML table (`spec` as Python's format spec). */
  format(spec?: string | null): string;
  toString(): string;
  repr(): string;
}

/** `die.eq(x)` / `die.ne(x)`: a die of bools that also has Python's truth value. */
export class DieWithTruth extends Die {
  truth(): boolean;
}

export function die(outcomes: readonly MapResult[] | Map<Outcome, Int> | Die, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions): Die;
export function d(sides: Int): Die;
export function z(sides: Int): Die;
export function coin(n: Int | number | Fraction, d?: Int, options?: { maxDenominator?: Int | null }): Die;
export function oneHot(sides: Int): Die;
export function stochasticRound(x: number | Fraction | Float, options?: { maxDenominator?: Int | null }): Die;
export function fromCumulative(outcomes: readonly Outcome[], cumulative: readonly (Int | Die)[], options?: { reverse?: boolean }): Die;

/** A random variable as `fromRv` takes it (duck-typed like `scipy.stats`): `cdf` of a sequence; a `pdf` makes it
 * continuous. */
export interface RandomVariable {
  cdf(xs: Outcome[], kwargs?: { [kwarg: string]: unknown }): Iterable<number | Fraction | Float>;
  pdf?: unknown;
}
/** `icepool.from_rv(rv, outcomes, denominator, **kwargs)`: kwargs go to `rv.cdf`. */
export function fromRv(rv: RandomVariable, outcomes: readonly Outcome[], denominator: Int, kwargs?: { [kwarg: string]: unknown }): Die;

// --- Free functions -----------------------------------------------------------------------------------------------

/** `map(f, ...args, { star, repeat, ...kwargs })`. */
export function map(f: Repl, ...rest: unknown[]): Die;
/** `map_to_pool(f, ...args, { star })`: `f` returns a pool's contents (a `List`), a multiset expression or Reroll. */
export function mapToPool(f: Repl, ...rest: unknown[]): MultisetExpression;
export function mapAndTime(f: (...args: any[]) => MapResult, initial: Operand, ...rest: unknown[]): Die;
export function meanTimeToAbsorb(f: (...args: any[]) => MapResult, initial: Operand, ...rest: unknown[]): Fraction;
export function mapFunction(f: (...args: any[]) => MapResult, options?: { star?: boolean | null; [kwarg: string]: unknown }): (...args: unknown[]) => Die;
export function reduce(f: (a: any, b: any) => MapResult, dice: readonly Operand[], options?: { initial?: Operand | null }): Die;
export function accumulate(f: (a: any, b: any) => MapResult, dice: readonly Operand[], options?: { initial?: Operand | null }): Die[];
export function highest(...args: (Operand | { keep?: Int | null; drop?: Int | null; default?: Outcome })[]): Die;
export function lowest(...args: (Operand | { keep?: Int | null; drop?: Int | null; default?: Outcome })[]): Die;
export function middle(...args: (Operand | { keep?: Int; tie?: "error" | "high" | "low"; default?: Outcome })[]): Die;
export function tupleize(...args: Operand[]): Die;
export function vectorize(...args: Operand[]): Die;
export function pointwiseMax(...dice: Die[]): Die;
export function pointwiseMin(...dice: Die[]): Die;
export function maxOutcome(...args: Operand[]): Outcome;
export function minOutcome(...args: Operand[]): Outcome;
export function consecutive(...args: readonly Int[][]): Int[];
export function sortedUnion(...args: readonly Outcome[][]): Outcome[];
export function harmonizeDenominators(dice: readonly Operand[], weights?: readonly Int[] | null): Die[];
export function formatProbabilityInverse(probability: number | Fraction, intStart?: Int): string;
/** Python's `bool(x)` of a Vector, Symbols or DieWithTruth. */
export function truth(x: unknown): boolean;

// --- Multisets ----------------------------------------------------------------------------------------------------

/** Where a multiset expression is expected, an array of outcomes is a pool of them. */
export type MultisetArg = MultisetExpression | readonly Outcome[];

/** A `MultisetExpression`: a pool, a deal, or operators over them. */
export class MultisetExpression extends Handle {
  additiveUnion(...others: MultisetArg[]): Expression;
  difference(...others: (MultisetArg | { keepNegativeCounts?: boolean })[]): Expression;
  intersection(...others: MultisetArg[]): Expression;
  union(...others: MultisetArg[]): Expression;
  symmetricDifference(other: MultisetArg): Expression;
  add(other: MultisetArg): MultisetExpression;
  sub(other: MultisetArg): MultisetExpression;
  and(other: MultisetArg): MultisetExpression;
  or(other: MultisetArg): MultisetExpression;
  xor(other: MultisetArg): MultisetExpression;

  mul(n: Int): MultisetExpression;
  neg(): MultisetExpression;
  pos(): MultisetExpression;
  floordiv(n: Int): MultisetExpression;
  mod(n: Int): MultisetExpression;
  unique(n?: Int): MultisetExpression;
  keepCounts(comparison: Comparison, n: Int): MultisetExpression;
  multiplyCounts(n: Int): MultisetExpression;
  divideCounts(n: Int): MultisetExpression;
  moduloCounts(n: Int): MultisetExpression;

  keepOutcomes(target: readonly Outcome[] | ((outcome: any) => unknown) | MultisetExpression): MultisetExpression;
  dropOutcomes(target: readonly Outcome[] | ((outcome: any) => unknown) | MultisetExpression): MultisetExpression;
  forceOrder(order: OrderValue): MultisetExpression;
  /** Each possible multiset weighted 1 (pools, deals and compound keeps only; others raise). */
  weightless(): MultisetExpression;

  maxPairKeep(comparison: Comparison, other: MultisetArg, priority?: "low" | "high" | null): MultisetExpression;
  maxPairDrop(comparison: Comparison, other: MultisetArg, priority?: "low" | "high" | null): MultisetExpression;
  sortPair(comparison: Comparison, other: MultisetArg, order?: OrderValue, extra?: "early" | "late" | "low" | "high" | "equal" | "keep" | "drop"): MultisetExpression;
  sortPairKeepWhile(comparison: Comparison, other: MultisetArg, order?: OrderValue, extra?: "early" | "late" | "low" | "high" | "equal" | "keep" | "drop"): MultisetExpression;
  sortPairDropWhile(comparison: Comparison, other: MultisetArg, order?: OrderValue, extra?: "early" | "late" | "low" | "high" | "equal" | "keep" | "drop"): MultisetExpression;
  versusAll(comparison: Comparison, other: MultisetArg): MultisetExpression;
  versusAny(comparison: Comparison, other: MultisetArg): MultisetExpression;

  sum(): Die;
  size(): Die;
  empty(): Die;
  any(): Die;
  productOfCounts(): Die;
  highestOutcomeAndCount(): Die;
  largestCount(): Die;
  largestCountAndOutcome(): Die;
  largestStraight(): Die;
  allStraights(): Die;
  largestStraightAndOutcome(priority?: "high" | "low"): Die;
  expand(order?: OrderValue): Die;
  allCounts(filter?: Int | "all"): Die;
  countSubset(divisor: MultisetArg, options?: { emptyDivisor?: Int | null }): Die;
  isdisjoint(other: MultisetArg): Die;
  issubset(other: MultisetArg): Die;
  issuperset(other: MultisetArg): Die;
  /** A proper subset (`<`). */
  lt(other: MultisetArg): Die;
  /** A proper superset (`>`). */
  gt(other: MultisetArg): Die;
  equalSet(other: MultisetArg): Die;
  notEqualSet(other: MultisetArg): Die;

  /** `MultisetExpression.map_counts(...args, { function })`. */
  static mapCounts(...args: (MultisetArg | { function: (outcome: any, ...counts: any[]) => Int })[]): Expression;
}

/** A pool or a deal: its sorted rolls can be kept or dropped. */
declare class KeepGenerator extends MultisetExpression {
  highest(keep?: Int | null, drop?: Int | null): this;
  lowest(keep?: Int | null, drop?: Int | null): this;
  middle(keep?: Int, options?: { tie?: "error" | "high" | "low" }): this;
  /** A keep tuple (an array of counts, `Ellipsis` allowed), or an int: that sorted roll as a die. */
  keep(index: readonly (Int | Special)[]): this;
  keep(index: Int): Die;
  /** `pool[index]`. */
  at(index: Int): Die;
  /** `pool[start:stop:step]`. */
  slice(start?: Int | null, stop?: Int | null, step?: Int | null): this;
}

/** `icepool.Pool`. */
export class Pool extends KeepGenerator {
  /** `new Pool([d6, d6, d8])`, `new Pool([d6, d8], [2, 1])`, `new Pool(new Map([[d6, 3]]))`. */
  constructor(dice: readonly Operand[] | Map<Operand, Int>, times?: Int | readonly Int[]);
  info(): [Int[], Int, Int, Int, Outcome[], [Die, Int][]];
  keepTuple(): Int[];
  keepSize(): Int;
  rawSize(): Int;
  denominator(): bigint;
  outcomes(): Outcome[];
}
export function pool(dice: readonly Operand[] | Map<Operand, Int>, times?: Int | readonly Int[]): Pool;
export function dPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool;
export function zPool(dieSizes: readonly Int[] | Map<Int, Int>): Pool;
export function multisetMixture(inners: readonly MultisetExpression[]): Expression;

/** Any other multiset expression. */
export class Expression extends MultisetExpression {}

// --- Decks --------------------------------------------------------------------------------------------------------

/** `icepool.Deck`. */
export class Deck extends Handle implements Iterable<Outcome> {
  /** `new Deck([1, 2, 3], 4)`, `new Deck(new Map([["a", 2]]))`. */
  constructor(outcomes: readonly (Outcome | Die | Deck)[] | Map<Outcome, Int>, times?: Int | readonly Int[]);
  deal(handSize: Int): Deal;
  deal(handSizes: readonly Int[]): MultiDeal;
  /** `deck.deal_groups(...[handSize, groupSize])`. */
  dealGroups(...groups: [Int, Int][]): MultiDeal;
  map(f: (...args: any[]) => MapResult, options?: { star?: boolean | null }): Deck;
  add(other: unknown): Deck;
  sub(other: unknown): Deck;
  and(other: unknown): Deck;
  or(other: unknown): Deck;
  xor(other: unknown): Deck;
  mul(n: Int): Deck;
  floordiv(n: Int): Deck;
  mod(n: Int): Deck;
  symmetricDifference(other: unknown): Deck;
  padToDenominator(denominator: Int, outcome: Outcome): Deck;
  sequence(deals: Int): Die;
  items(): [Outcome, bigint][];
  outcomes(): Outcome[];
  keys(): Outcome[];
  values(): bigint[];
  get(outcome: Outcome): bigint;
  size(): bigint;
  readonly length: number;
  [Symbol.iterator](): Iterator<Outcome>;
  stat(name: string): any;
  mean(): Fraction | number;
  variance(): Fraction | number;
  sd(): number;
  minOutcome(): Outcome;
  maxOutcome(): Outcome;
  denominator(): bigint;
}
export function deck(outcomes: readonly (Outcome | Die | Deck)[] | Map<Outcome, Int>, times?: Int | readonly Int[]): Deck;

/** A hand dealt from a deck. */
export class Deal extends KeepGenerator {
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
  handSize(): Int;
  keepSize(): Int;
  denominator(): bigint;
  outcomes(): Outcome[];
  repr(): string;
}

/** Several hands: a `MultiDeal`, a tuple parameter of a multiset function, or a slice of one. */
export class MultisetTupleExpression extends Handle {
  hand(index: Int): MultisetExpression;
  slice(start?: Int | null, stop?: Int | null, step?: Int | null): MultisetTupleExpression;
}

/** Several hands dealt from one deck. */
export class MultiDeal extends MultisetTupleExpression {
  info(): [[Outcome, Int][], Int[], Int, Int, Int, Outcome[]];
  handSizes(): Int[];
  totalCardsDealt(): Int;
  denominator(): bigint;
  outcomes(): Outcome[];
  repr(): string;
}

// --- Wallenius ----------------------------------------------------------------------------------------------------

/** `icepool.Wallenius(data)`: `[outcome, weight]` pairs, or a Map of outcome -> weight or `[weight, quantity]`. */
export class Wallenius {
  constructor(data: readonly (readonly [Outcome, Int])[] | Map<Outcome, Int | readonly [Int, Int]>);
  deal(handSize: Int): MultisetExpression;
}

// --- Again --------------------------------------------------------------------------------------------------------

export interface AgainExpression extends Operators<AgainExpression> {}
/** `Again`, or operators over it. */
export class AgainExpression extends Handle {}
/** `icepool.Again`: `new Die([1, 2, Again.add(3)], { againDepth: 2 })`. */
export const Again: AgainExpression;

// --- Evaluators ---------------------------------------------------------------------------------------------------

/** A state the core can hash: values, arrays of them, or plain objects (keyed by their JSON). */
export type State = Outcome | { [key: string]: unknown } | readonly State[];

/** A multiset evaluator written in JS. `nextState` is required; the other methods are optional. Throw
 * `UnsupportedOrder` to ask for the other order. */
export class MultisetEvaluator {
  nextState?(state: any, order: OrderValue, outcome: any, ...counts: any[]): State | typeof Reroll;
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: any[]): State;
  finalOutcome?(finalState: any, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: any[]): MapResult | null;
  extraOutcomes?(outcomes: Outcome[]): Iterable<Outcome>;
  /** Evaluates pools, deals, expressions or arrays of outcomes; a trailing options object is Python's kwargs. */
  evaluate(...inputs: (MultisetArg | MultiDeal | { [kwarg: string]: unknown })[]): Die;
}

export function evaluator(methods: {
  nextState(state: any, order: OrderValue, outcome: any, ...counts: any[]): State | typeof Reroll;
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: any[]): State;
  finalOutcome?(finalState: any, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: any[]): MapResult | null;
  extraOutcomes?(outcomes: Outcome[]): Iterable<Outcome>;
}): MultisetEvaluator;

// --- Vector and Symbols -------------------------------------------------------------------------------------------

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
  lt(other: unknown): any;
  le(other: unknown): any;
  gt(other: unknown): any;
  ge(other: unknown): any;
  eq(other: unknown): any;
  ne(other: unknown): any;
  neg(): Self;
  pos(): Self;
  invert(): Self;
  abs(): Self;
  trunc(): Self;
  floor(): Self;
  ceil(): Self;
}

export interface Vector extends ValueOperators<Vector> {}
/** `icepool.Vector`. */
export class Vector implements Iterable<Outcome> {
  /** `null` for a `VectorWithTruthOnly` (a comparison between vectors of different lengths). */
  readonly items: Outcome[] | null;
  readonly hasTruth: boolean;
  readonly truthValue: Outcome;
  constructor(items: Iterable<Outcome>);
  readonly length: number;
  [Symbol.iterator](): Iterator<Outcome>;
  div(other: unknown): Vector;
  round(ndigits?: Int | null): Vector;
  append(x: Outcome): Vector;
  concatenate(other: Iterable<Outcome>): Vector;
  repr(): string;
}

export interface Symbols extends ValueOperators<Symbols> {}
/** `icepool.Symbols`: a multiset of characters (`new Symbols("aab-c")`). */
export class Symbols {
  /** Python's `str()`, `null` if that raises (non-int counts). */
  readonly str: string | null;
  constructor(str: string);
  /** `Symbols(x)` from anything Python takes (an iterable of characters, a Map of counts). */
  static from(x: string | readonly string[] | Map<string, Int>): Symbols;
  method(name: string, arg?: unknown): any;
  issubset(other: unknown): boolean;
  issuperset(other: unknown): boolean;
  isdisjoint(other: unknown): boolean;
  countSubset(divisor: unknown, options?: { emptyDivisor?: Int | null }): Int;
  hasNegativeCounts(): boolean;
  size(): Int;
  multiplyCounts(n: Int): Symbols;
  divideCounts(n: unknown): Symbols;
  moduloCounts(n: Int): Symbols;
  additiveUnion(...others: unknown[]): Symbols;
  difference(...others: unknown[]): Symbols;
  intersection(...others: unknown[]): Symbols;
  union(...others: unknown[]): Symbols;
  symmetricDifference(other: unknown): Symbols;
  get(c: string): Int;
  keys(): string[];
  values(): Int[];
  repr(): string;
}

// --- Other core objects -------------------------------------------------------------------------------------------

/** A callable made by the core (the harness's catalogue functions). */
export class CoreFunction extends Handle {}
/** A `@multiset_function`: from `multisetFunction(f)` or the program language's `mf_def`. A trailing options
 * object holds `star` and the function's keyword-only arguments. */
export class MultisetFunction extends Handle {
  call(...args: unknown[]): Die;
  evaluate(...args: unknown[]): Die;
}

/** A parameter of a multiset function: one input, or one hand of an unstarred MultiDeal input. */
export type MultisetParameter = MultisetExpression | MultisetTupleExpression;

/** `@multiset_function`: `f` is traced with parameter expressions and returns evaluations of them (an array for
 * a joint evaluation, or constants). Inside `f`, evaluation methods (typed as returning `Die`) only record the
 * evaluation. Keyword-only arguments (`kwonly`) arrive as a trailing options object. */
export function multisetFunction(
  f: (...args: any[]) => unknown,
  options?: { params?: number; rest?: boolean; kwonly?: readonly string[] },
): MultisetFunction;
