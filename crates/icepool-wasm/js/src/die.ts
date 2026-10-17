// `icepool.Die` (population/die.py, population/base.py): a die of the core (`WasmDie`), with Python's argument
// juggling done here. Arithmetic is methods (`d(6).add(1)`, `die.gt(10)`), keyword arguments are a trailing
// options object (`die.reroll([1], { depth: 2 })`), quantities are BigInts.

import type { WasmDie } from "../pkg-node/icepool_wasm.js";
import type { AgainExpression } from "./again.js";
import type { MultisetExpression, Pool } from "./multiset.js";
import { IcepoolError } from "./errors.js";
import { ENCODE, Handle, call, registerKind, type Encodable } from "./rpc.js";
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

/** What dice operators and constructors accept on the other side: an outcome, a die, or an Again expression. */
export type Operand = Outcome | Die | AgainExpression;

/** A JS function the core calls back. Its parameters are typed by the caller (`(x: number) => ...`); Python's
 * `star` is inferred from its `length`, as Python counts positional parameters. */
export type Callback<R = unknown> = (...args: never[]) => R;

/** A selection of outcomes (`reroll`, `filter`, `split`, `explode`, ...): a predicate or the outcomes themselves
 * (a Map: its keys, as Python's dict). */
export type Which = Callback | readonly Outcome[] | Map<unknown, unknown> | null;

/** A comparison, as Python spells it. */
export type Comparison = "==" | "!=" | "<=" | "<" | ">=" | ">";

/** What a `map` function may return. */
export type MapResult = Operand | typeof Reroll | typeof Restart | Break | readonly MapResult[];

/** What `map` takes: a function, or a Map (Python's dict: `mapping.get(outcome, outcome)`, keys compared with
 * Python's `==`, so `1`, `1.0` and `true` are one key). */
export type Repl = Callback<MapResult> | Map<unknown, MapResult>;

/** The keyword arguments of `Die(...)` for `Again` outcomes (population/again.py). */
export interface AgainOptions {
  againCount?: Int | null;
  againDepth?: Int | null;
  againEnd?: Operand | typeof Reroll | typeof Restart | null;
}

/** What `new Die(...)` takes as outcomes: a sequence of outcomes, dice, `Reroll` or Again expressions, a Map of
 * outcomes to quantities, or a die (whose quantities multiply the times). */
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

/** `icepool.Die`: `new Die([1, 2, 3])`, `new Die([d6, d8], [2, 1])`, `new Die(new Map([[1, 2n], [2, 1n]]))`;
 * options `{ againCount, againDepth, againEnd }` as in Python. Free it with `free()` or `using` when done with it
 * (else the garbage collector does, eventually). */
export class Die<T extends Outcome = Outcome> implements Iterable<T>, Encodable, Disposable {
  /** The core's die; `null` once freed. */
  protected core: WasmDie | null = null;

  constructor(outcomes: DieOutcomes, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions) {
    if (isOptions(times)) [times, options] = [undefined, times as AgainOptions];
    const made = makeDie(outcomes, times as Int | readonly Int[] | undefined, options ?? {});
    // `Die([die])` is that very die (a `DieWithTruth` keeps its type and truth value), as in Python.
    if (made !== undefined && !(made instanceof WasmDieBox)) return made as unknown as Die<T>;
    this.core = made === undefined ? null : made.die;
  }

  /** The core's die (throws once freed). */
  get w(): WasmDie {
    if (this.core === null) throw new TypeError("icepool: this die was freed");
    return this.core;
  }

  /** Wraps a core die. */
  static wrap(w: WasmDie): Die {
    const die = Object.create(Die.prototype) as Die;
    die.core = w;
    return die;
  }

  [ENCODE](temps: number[] | null): string {
    const w = this.w;
    const h = wasm((m) => m.dieHandle(w, undefined));
    if (temps === null) return `{"$take":${h}}`;
    temps.push(h);
    return `{"$h":${h}}`;
  }

  /** Frees the die's WASM memory now rather than when it is garbage-collected. Using it afterwards throws a
   * `TypeError`; freeing it again does nothing. Dice made from it (`die.add(1)`) are not affected. */
  free(): void {
    this.core?.free();
    this.core = null;
  }

  /** `free()`, for `using die = d(6);`. */
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

  add(other: Operand): Die { return this.binary("add", other); }
  sub(other: Operand): Die { return this.binary("sub", other); }
  mul(other: Operand): Die { return this.binary("mul", other); }
  truediv(other: Operand): Die { return this.binary("truediv", other); }
  /** `truediv`. */
  div(other: Operand): Die { return this.binary("truediv", other); }
  floordiv(other: Operand): Die { return this.binary("floordiv", other); }
  mod(other: Operand): Die { return this.binary("mod", other); }
  pow(other: Operand): Die { return this.binary("pow", other); }
  lshift(other: Operand): Die { return this.binary("lshift", other); }
  rshift(other: Operand): Die { return this.binary("rshift", other); }
  and(other: Operand): Die { return this.binary("and", other); }
  or(other: Operand): Die { return this.binary("or", other); }
  xor(other: Operand): Die { return this.binary("xor", other); }

  // The reflected operators: `3 - die` is `die.rsub(3)`, the other operand converted to a die on the left.
  radd(other: Operand): Die { return this.binary("add", other, true); }
  rsub(other: Operand): Die { return this.binary("sub", other, true); }
  rmul(other: Operand): Die { return this.binary("mul", other, true); }
  rtruediv(other: Operand): Die { return this.binary("truediv", other, true); }
  rfloordiv(other: Operand): Die { return this.binary("floordiv", other, true); }
  rmod(other: Operand): Die { return this.binary("mod", other, true); }
  rpow(other: Operand): Die { return this.binary("pow", other, true); }
  rlshift(other: Operand): Die { return this.binary("lshift", other, true); }
  rrshift(other: Operand): Die { return this.binary("rshift", other, true); }
  rand(other: Operand): Die { return this.binary("and", other, true); }
  ror(other: Operand): Die { return this.binary("or", other, true); }
  rxor(other: Operand): Die { return this.binary("xor", other, true); }

  // Comparisons: dice of bools.
  lt(other: Operand): Die { return this.binary("lt", other); }
  le(other: Operand): Die { return this.binary("le", other); }
  gt(other: Operand): Die { return this.binary("gt", other); }
  ge(other: Operand): Die { return this.binary("ge", other); }
  /** `self == other`: a die of bools with a truth value, `truth()`, which is whether the dice are the same
   * (`equals`; a die is never the same as a plain outcome). */
  eq(other: Operand): DieWithTruth { return this.equality(other, false); }
  /** `self != other`: as `eq`, with the opposite truth value. */
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

  neg(): Die { return this.unary("neg"); }
  pos(): Die { return this.unary("pos"); }
  invert(): Die { return this.unary("invert"); }
  abs(): Die { return this.unary("abs"); }
  trunc(): Die { return this.unary("trunc"); }
  floor(): Die { return this.unary("floor"); }
  ceil(): Die { return this.unary("ceil"); }
  /** `round(die, ndigits)`. */
  round(ndigits: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.round(ndigits)));
  }

  /** `self @ other`: roll this die, then roll `other` that many times and sum. */
  matmul(other: Operand): Die {
    const o = nativeDie(other);
    if (o === undefined) return call("matmul", { n: this, die: other }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => w.matmul(o)));
  }
  /** `n @ self`: the sum of `n` rolls of this die (`n` an int or a die). */
  rmatmul(n: Int | Die): Die {
    const o = nativeDie(n);
    if (o === undefined) return call("matmul", { n, die: this }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => o.matmul(w)));
  }
  /** `n @ die`: the sum of `n` rolls (n an int or a die). */
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

  /** `die.reroll(which, star=, depth=)`. `which` a function, an array of outcomes, or null (the lowest outcome);
   * `depth` is required, as in Python: an int, or Infinity (Python's 'inf'). */
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
  /** `die.filter(which, star=, depth=)`: rerolls the outcomes `which` does not select. */
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
  truncate(min: Outcome = null, max: Outcome = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.truncate(min, max)));
  }
  clip(min: Outcome = null, max: Outcome = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.clip(min, max)));
  }
  /** `[selected, rest]`. */
  split(which: Which = null, { star = null }: { star?: boolean | null } = {}): [Die, Die] {
    const mask = which === null ? undefined : this.selection(which, star);
    if (mask !== undefined) {
      const w = this.w;
      const [selected, rest] = wasm(() => w.split(mask));
      return [Die.wrap(selected), Die.wrap(rest)];
    }
    return call("split", { die: this, which: list(which), star }) as [Die, Die];
  }
  explode(
    which: Which = null,
    { star = null, depth = 9, end = null }: { star?: boolean | null; depth?: Int; end?: Operand | typeof Reroll | null } = {},
  ): Die {
    return call("explode", { die: this, which: list(which), star, depth, end }) as Die;
  }

  // --- map and friends: `die.map(f, ...extraArgs, { star, repeat })` is `map(f, die, ...extraArgs, ...)` --------

  map(f: Repl, ...rest: unknown[]): Die {
    return map(f, this, ...rest);
  }
  mapAndTime(f: Callback<MapResult>, ...rest: unknown[]): Die {
    return mapAndTime(f, this, ...rest);
  }
  meanTimeToAbsorb(f: Callback<MapResult>, ...rest: unknown[]): Fraction {
    return meanTimeToAbsorb(f, this, ...rest);
  }
  /** The expected number of rolls to reach a sum of at least `target` (a Fraction). */
  meanTimeToSum(target: Int): Fraction {
    return (call("mean_time_to_sum", { die: this, targets: List.of(target) }) as Fraction[])[0];
  }
  timeToSum(target: Int, maxTime: Int | null = null, dnf: Int | typeof Reroll | null = null): Die {
    return call("time_to_sum", { die: this, target, max_time: maxTime, dnf }) as Die;
  }
  /** The dice of the outcomes with the same key, by key (in order of first appearance; keys equal by Python's
   * `==`). `key` is called on each outcome (unpacked if `star`), or a Map looked up with `key.get(o, o)`. */
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
  groupByIndex(index: Int): Map<Outcome, Die> {
    return new Map(call("group_by_index", { die: this, index }) as [Outcome, Die][]);
  }
  /** `die.is_in(outcomes)`: a die of whether each outcome is `in` the outcomes. */
  isIn(outcomes: readonly Outcome[]): Die {
    const mask = Array.isArray(outcomes) ? this.selection(outcomes, false) : undefined;
    if (mask === undefined) return call("is_in", { die: this, outcomes: list(outcomes) }) as Die;
    // `self.map(lambda x: x in outcomes)`.
    return new Die(Array.from(mask, (m) => m === 1), this.quantities());
  }
  /** `die.count(rolls, outcomes)`: how many of `rolls` rolls are `in` the outcomes. */
  count(rolls: Int, outcomes: readonly Outcome[]): Die {
    if (!Array.isArray(outcomes)) return call("die_count", { die: this, rolls, outcomes: list(outcomes) }) as Die;
    return this.isIn(outcomes).rmatmul(rolls);
  }
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

  highest(rolls: Int, keep: Int | null = null, drop: Int | null = null): Die {
    return call("die_highest", { die: this, rolls, keep, drop }) as Die;
  }
  lowest(rolls: Int, keep: Int | null = null, drop: Int | null = null): Die {
    return call("die_lowest", { die: this, rolls, keep, drop }) as Die;
  }
  middle(rolls: Int, keep: Int = 1, { tie = "error" }: { tie?: "error" | "high" | "low" } = {}): Die {
    return call("die_middle", { die: this, rolls, keep, tie }) as Die;
  }
  /** `die.keep(rolls, index)`: roll `rolls` dice, sort them and sum the selected ones. `rolls` an array of counts
   * (one per sorted roll) and no index; or an int and an index: an int (the roll at that sorted position) or an
   * array of counts. */
  keep(rolls: Int | readonly Int[], index: Int | readonly (Int | typeof Ellipsis)[] | null = null): Die {
    return call("die_keep", { die: this, rolls, index }) as Die;
  }
  /** `die.keep(rolls, slice(start, stop, step))`: the sorted rolls in the slice, summed. */
  keepSlice(rolls: Int, start: Int | null = null, stop: Int | null = null, step: Int | null = null): Die {
    return call("die_keep_slice", { die: this, rolls, start, stop, step }) as Die;
  }
  /** Every ordered sequence of `rolls` rolls, as a die of arrays (Python's tuples); `[]` itself for 0 rolls. */
  sequence(rolls: Int): Die {
    return call("die_sequence", { die: this, rolls }) as Die;
  }
  /** `rolls` copies of this die as a pool (a list of counts: a keep tuple). */
  pool(rolls: Int | readonly Int[] = 1): Pool {
    return call("die_pool", { die: this, rolls: Array.isArray(rolls) ? list(rolls) : rolls }) as Pool;
  }

  // --- Pools from a die (EXPERIMENTAL in icepool) ---------------------------------------------------------------

  /** `die.map_to_pool(f, ...extraArgs, { star })`: `mapToPool(f, die, ...extraArgs)`; `f` null uses the outcomes
   * themselves as the pools' contents. */
  mapToPool(f: Repl | null = null, ...rest: unknown[]): MultisetExpression {
    const [args, { star = null }] = opts<{ star: boolean | null }>(rest);
    return call("die_map_to_pool", { die: this, f, args: List.from(args), star }) as MultisetExpression;
  }
  /** Each explosion is a separate die of the pool; `outcomes` null explodes the max outcome. */
  explodeToPool(rolls: Int = 1, outcomes: Which = null, { star = null, depth = 9 }: { star?: boolean | null; depth?: Int } = {}): MultisetExpression {
    return call("explode_to_pool", { die: this, rolls, outcomes: list(outcomes), star, depth }) as MultisetExpression;
  }
  /** At most `maxRerolls` rerolls (an int or Infinity) shared across a pool of `rolls` dice; `mode` is 'random',
   * 'low', 'high' or 'drop'. */
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

  simplify(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.simplify()));
  }
  zero(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.zero()));
  }
  zeroOutcome(): Outcome {
    const w = this.w;
    return wasm(() => w.zeroOutcome()) as Outcome;
  }
  sign(): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.sign()));
  }
  /** `die.cmp(other)`: 1, -1 or 0 per pair of outcomes. */
  cmp(other: Operand): Die {
    const o = nativeDie(other);
    if (o === undefined) return call("die_cmp", { die: this, other }) as Die;
    const w = this.w;
    return Die.wrap(wasm(() => w.cmp(o)));
  }
  /** This die with `quantity` more of `outcome` (a negative quantity removes some, but not below zero). */
  append(outcome: Outcome, quantity: Int = 1): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.append(outcome, quantity)));
  }
  /** This die with `quantity` less of `outcome` (`null`: all of it; a negative quantity adds). */
  remove(outcome: Outcome, quantity: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.remove(outcome, quantity)));
  }
  /** Each outcome as a one-hot Vector over `outcomes` (default: this die's outcomes). */
  toOneHot(outcomes: readonly Outcome[] | null = null): Die {
    return call("to_one_hot", { population: this, outcomes: list(outcomes) }) as Die;
  }
  /** `die.unary_operator(op)`: `op` applied to each outcome (the results with their quantities, as `Die`
   * merges them). */
  unaryOperator(op: Callback<Outcome>): Die {
    const f = op as (outcome: unknown) => unknown;
    return new Die(this.outcomes().map((o) => f(o) as Outcome), this.quantities());
  }
  /** `die.binary_operator(other, op)`: `op` applied to each pair of outcomes, with the product of their
   * quantities. */
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
  padToDenominator(denominator: Int, outcome: Outcome): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.padToDenominator(denominator, outcome)));
  }
  multiplyToDenominator(denominator: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.multiplyToDenominator(denominator)));
  }
  multiplyQuantities(scale: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.multiplyQuantities(scale)));
  }
  divideQuantities(divisor: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.divideQuantities(divisor)));
  }
  moduloQuantities(divisor: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.moduloQuantities(divisor)));
  }
  /** Each outcome rounded up with probability `x - floor(x)`, else down. */
  stochasticRound({ maxDenominator = null }: { maxDenominator?: Int | null } = {}): Die {
    return call("die_stochastic_round", { die: this, max_denominator: maxDenominator }) as Die;
  }
  /** Whether both are dice with the same outcomes and quantities (`false` for anything not a die). */
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

  /** `[outcome, quantity]` pairs in sorted order. */
  items(): [T, bigint][] {
    const w = this.w;
    return wasm(() => w.items()) as [T, bigint][];
  }
  /** The outcomes in sorted order. */
  outcomes(): T[] {
    const w = this.w;
    return wasm(() => w.outcomes()) as T[];
  }
  keys(): T[] {
    return this.outcomes();
  }
  values(): bigint[] {
    return this.quantities();
  }
  /** The quantity of an outcome (a KeyError if it is not one). */
  get(outcome: Outcome): bigint {
    const w = this.w;
    return wasm(() => w.get(outcome)) as bigint;
  }
  has(outcome: Outcome): boolean {
    const w = this.w;
    return wasm(() => w.has(outcome));
  }
  /** The number of outcomes. */
  get size(): number {
    return this.w.size;
  }
  [Symbol.iterator](): Iterator<T> {
    return this.outcomes()[Symbol.iterator]();
  }

  // --- Statistics: exact ones are Fractions (or ints), the rest floats (numbers) ---------------------------------

  mean(): Fraction | number { return this.stat((w) => w.mean()) as Fraction | number; }
  variance(): Fraction | number { return this.stat((w) => w.variance()) as Fraction | number; }
  standardDeviation(): number { return this.stat((w) => w.standardDeviation()) as number; }
  /** `standardDeviation`. */
  sd(): number { return this.standardDeviation(); }
  skewness(): number { return this.stat((w) => w.skewness()) as number; }
  excessKurtosis(): number { return this.stat((w) => w.excessKurtosis()) as number; }
  /** The outcomes with the highest quantity. */
  mode(): T[] { return this.stat((w) => w.mode()) as T[]; }
  modalQuantity(): bigint { return this.stat((w) => w.modalQuantity()) as bigint; }
  median(): Outcome { return this.quantile(1, 2); }
  medianLow(): Outcome { return this.quantileLow(1, 2); }
  medianHigh(): Outcome { return this.quantileHigh(1, 2); }
  minOutcome(): T { return this.stat((w) => w.minOutcome()) as T; }
  maxOutcome(): T { return this.stat((w) => w.maxOutcome()) as T; }
  denominator(): bigint { return this.stat((w) => w.denominator()) as bigint; }
  isEmpty(): boolean { return this.size === 0; }
  quantile(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantile(n, d)) as Outcome; }
  quantileLow(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantileLow(n, d)) as Outcome; }
  quantileHigh(n: Int, d: Int = 100): Outcome { return this.stat((w) => w.quantileHigh(n, d)) as Outcome; }
  standardizedMoment(k: Int): number { return this.stat((w) => w.standardizedMoment(k)) as number; }
  entropy(base: number | Float = new Float(2)): number { return this.stat((w) => w.entropy(base)) as number; }
  kolmogorovSmirnov(other: Die): Fraction | number {
    const o = other.w;
    return this.stat((w) => w.kolmogorovSmirnov(o)) as Fraction | number;
  }
  cramerVonMises(other: Die): Fraction | number {
    const o = other.w;
    return this.stat((w) => w.cramerVonMises(o)) as Fraction | number;
  }
  /** The covariance of two elements of tuple outcomes. */
  covariance(i: Int, j: Int): Fraction | number { return this.stat((w) => w.covariance(i, j)) as Fraction | number; }
  correlation(i: Int, j: Int): number { return this.stat((w) => w.correlation(i, j)) as number; }
  /** The length common to every outcome, or `null` if they differ or are not sequences. */
  commonOutcomeLength(): number | null { return this.stat((w) => w.commonOutcomeLength() ?? null) as number | null; }
  /** The outcome nearest to `outcome` in the direction of the comparison, `null` if there is none. */
  nearest(comparison: Comparison, outcome: Outcome): T | null {
    return this.stat((w) => w.nearest(comparison, outcome)) as T | null;
  }

  /** `quantity(outcome)` or `quantity(comparison, outcome)`. */
  quantity(...args: [Outcome] | [Comparison, Outcome]): bigint {
    const [c, outcome] = comparisonArgs(args);
    return this.stat((w) => w.quantity(c, outcome)) as bigint;
  }
  /** All quantities, or cumulative ones for a comparison ('<=', '>=', ...). */
  quantities(comparison: Comparison | null = null): bigint[] {
    return this.stat((w) => w.quantities(comparison ?? undefined)) as bigint[];
  }
  /** `probability(outcome)` or `probability(comparison, outcome)`, with a trailing `{ percent }` option. */
  probability(
    ...args: [Outcome] | [Comparison, Outcome] | [Outcome, { percent?: boolean }] | [Comparison, Outcome, { percent?: boolean }]
  ): Fraction | number {
    const [a, { percent = false }] = opts<{ percent: boolean }>(args);
    const [c, outcome] = comparisonArgs(a as [Outcome] | [Comparison, Outcome]);
    return this.stat((w) => w.probability(c, outcome, percent)) as Fraction | number;
  }
  probabilities(comparison: Comparison | null = null, { percent = false }: { percent?: boolean } = {}): (Fraction | number)[] {
    return this.stat((w) => w.probabilities(comparison ?? undefined, percent)) as (Fraction | number)[];
  }
  /** The total quantity of the outcomes `which` (a function) selects. */
  quantityWhere(which: Callback, { star = null }: { star?: boolean | null } = {}): bigint {
    const mask = typeof which === "function" ? this.selection(which, star) : undefined;
    if (mask === undefined) return BigInt(call("quantity_where", { die: this, which, star }) as Int);
    const w = this.w;
    return wasm(() => w.quantityWhere(mask)) as bigint;
  }
  /** The probability of the outcomes `which` (a function) selects. */
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

  /** One die per element of the (tuple) outcomes. */
  marginals(): Die[] {
    const w = this.w;
    const n = wasm(() => w.marginalsLength());
    return Array.from({ length: n }, (_, i) => this.marginal(i));
  }
  /** `die.marginals[index]`. */
  marginal(index: Int): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.marginal(index)));
  }
  /** `die.marginals[start:stop:step]`. */
  marginalSlice(start: Int | null = null, stop: Int | null = null, step: Int | null = null): Die {
    const w = this.w;
    return Die.wrap(wasm(() => w.marginalSlice(start, stop, step)));
  }

  /** One outcome at random (Python's Mersenne Twister seeded with `seed`). */
  sample(options?: { seed?: Int }): T;
  /** `n` outcomes at random. */
  sample(options: { seed?: Int; n: Int }): T[];
  sample({ seed = Math.floor(Math.random() * 2 ** 32), n }: { seed?: Int; n?: Int } = {}): T | T[] {
    const rolls = call("sample", { die: this, seed, n: n ?? 1 }) as T[];
    return n === undefined ? rolls[0] : rolls;
  }

  // --- Formatting -----------------------------------------------------------------------------------------------

  /** Python's `type(die).__name__`. */
  protected get typeName(): string {
    return "Die";
  }
  /** `format(die, spec)`: a Markdown / BBCode / CSV / HTML table (`spec` as Python's, e.g. "md:q==|p"). */
  format(spec: string | null = null): string {
    const w = this.w;
    const name = this.typeName;
    return wasm(() => w.format(spec ?? "", name));
  }
  /** Python's `str(die)`. */
  toString(): string {
    return this.format(null);
  }
  /** Python's `repr(die)`. */
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

/** The result of `die.eq(x)` / `die.ne(x)`: a die of bools that also has a truth value (`equals`). */
export class DieWithTruth extends Die<boolean> {
  private truthValue = false;
  /** The handle of a result whose data raised (kept so the error comes back when the data is used). */
  private errorHandle = -1;

  /** Wraps a core die with its truth value. */
  static wrapTruth(w: WasmDie, truth: boolean): DieWithTruth {
    const die = Object.create(DieWithTruth.prototype) as DieWithTruth;
    die.core = w;
    die.truthValue = truth;
    die.errorHandle = -1;
    return die;
  }

  /** A result whose data raised: the data's error comes back from every use but `truth()` and `equals`. */
  static failed(handle: number, truth: boolean): DieWithTruth {
    const die = Object.create(DieWithTruth.prototype) as DieWithTruth;
    die.core = null;
    die.truthValue = truth;
    die.errorHandle = handle;
    failedRegistry.register(die, handle);
    return die;
  }

  override get w(): WasmDie {
    if (this.core === null && this.errorHandle >= 0) {
      const h = this.errorHandle;
      // Throws the data's error.
      return wasm((m) => m.WasmDie.fromHandle(h));
    }
    return super.w;
  }

  override [ENCODE](temps: number[] | null): string {
    if (this.core === null && this.errorHandle >= 0) return `{"$h":${this.errorHandle}}`;
    const w = this.w;
    const h = wasm((m) => m.dieHandle(w, this.truthValue));
    if (temps === null) return `{"$take":${h}}`;
    temps.push(h);
    return `{"$h":${h}}`;
  }

  override equals(other: unknown, options: { simplify?: boolean } = {}): boolean {
    // `isinstance(other, Die)` comes first: a result whose data raised is never compared with a non-die.
    if (!(other instanceof Die)) return false;
    return super.equals(other, options);
  }

  /** Python's `bool(die == x)`. */
  truth(): boolean {
    return this.truthValue;
  }

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

/** `icepool.Die(outcomes, times, options)` without `new`. */
export function die(outcomes: DieOutcomes, times?: Int | readonly Int[] | AgainOptions, options?: AgainOptions): Die {
  return new Die(outcomes, times, options);
}

/** A fair die numbered 1..sides. */
export function d(sides: Int): Die<number> {
  return Die.wrap(wasm((w) => w.WasmDie.d(sides))) as Die<number>;
}
/** A fair die numbered 0..sides-1. */
export function z(sides: Int): Die<number> {
  return Die.wrap(wasm((w) => w.WasmDie.z(sides))) as Die<number>;
}
