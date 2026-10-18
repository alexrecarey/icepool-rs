// The keep family (population/keep.py and the `Die` methods built on it, population/die.py): `Die.highest` /
// `lowest` / `middle` / `keep`, and the free `highest` / `lowest` / `middle`. As in Python, they are pools kept and
// summed (multiset.ts), with shortcuts for the single highest or lowest die on the core's `keepSingle` (the Python
// facade's `die_keep_single` / `keep_single`). Argument checks follow the runner's nodes
// (crates/icepool-harness/src/pool.rs), which copy Python's.

import { Die, dictItems, pyLen, unpack } from "./die.js";
import { IcepoolError } from "./errors.js";
import { isDeck, pool } from "./multiset.js";
import { binary } from "./operators.js";
import { AgainExpression } from "./again.js";
import { type Ellipsis, type Int, type Outcome } from "./values.js";
import { wasm } from "./wasm.js";

/** Python's `isinstance(x, int)` (a bool is one). */
const isPyInt = (x: unknown): x is Int | boolean =>
  typeof x === "bigint" || typeof x === "boolean" || (typeof x === "number" && Number.isInteger(x));


const valueError = (message: string) => new IcepoolError("InvalidArgument", "ValueError", message);
const unported = (what: string) => new IcepoolError("Unported", "NotImplementedError", what);

/** `[start, stop]` of a slice; `null` is an omitted bound. */
type Bounds = [Int | null, Int | null];

/** keep.py `highest_slice(keep, drop)` (`highest`) or `lowest_slice(keep, drop)`, on the core's (which raises
 * Python's `ValueError` for a negative count). `keep` and `drop` are ints or null, as in the runner. */
function keepDropSlice(highest: boolean, keep: unknown, drop: unknown): Bounds {
  const bound = (x: unknown, what: string): Int | null => {
    if (x === null || x === undefined) return null;
    if (!isPyInt(x)) throw unported(`a non-int ${what}`);
    return typeof x === "boolean" ? Number(x) : x;
  };
  const [k, d] = [bound(keep, "keep"), bound(drop, "drop")];
  return wasm((w) => w.keepDropSlice(highest, k, d)) as Bounds;
}

/** keep.py `canonical_slice(original, length)`: `[start, stop]` with `0 <= start <= stop <= length` (for a step that
 * is positive). */
function canonicalSlice([start, stop]: Bounds, length: bigint): [bigint, bigint] {
  const clamp = (x: Int | null, omitted: bigint): bigint => {
    if (x === null) return omitted;
    const i = BigInt(x);
    if (i < 0n) return length + i > 0n ? length + i : 0n;
    return i < length ? i : length;
  };
  const s = clamp(start, 0n);
  const t = clamp(stop, length);
  return [s, s > t ? s : t];
}

/** A number of rolls: an int, as the runner takes it (Python would go on with a float until `pool(rolls)`). */
function rollsCount(rolls: unknown): bigint {
  if (!isPyInt(rolls)) throw unported("non-int rolls");
  return BigInt(rolls);
}

/** `die._highest_single(rolls)` (`highest`) / `die._lowest_single(rolls)`: roll this die several times and keep the
 * highest (lowest). */
function keepSingle(die: Die, rolls: bigint, highest: boolean): Die {
  if (rolls === 0n) return die.zero().simplify();
  const w = die.w;
  return Die.wrap(wasm(() => w.keepSingle(rolls, highest)));
}

/** `Die.highest(rolls, keep, drop)` (`highest`) / `Die.lowest(rolls, keep, drop)`. @internal */
export function dieHighestLowest(die: Die, highest: boolean, rolls: Int, keep: unknown, drop: unknown): Die {
  const index = keepDropSlice(highest, keep, drop);
  const n = rollsCount(rolls);
  const [start, stop] = canonicalSlice(index, n);
  // The single highest / lowest. (With `rolls == 0` the canonical slice is empty, so `_highest_single(0)` is never
  // reached from here.)
  if (highest ? start === n - 1n && stop === n : start === 0n && stop === 1n) return keepSingle(die, n, highest);
  // Expression evaluators are difficult to type.
  return die.pool(rolls).slice(...index).sum();
}

/** `Die.middle(rolls, keep, tie=tie)`: `self.pool(rolls).middle(keep, tie=tie).sum()`. @internal */
export function dieMiddle(die: Die, rolls: Int, keep: Int, tie: "error" | "high" | "low"): Die {
  return die.pool(rolls).middle(keep, { tie }).sum();
}

/** `Die.keep(rolls, index)`: an int `rolls` needs an index (an int index is `self.pool(rolls).keep(index)`, a Die;
 * anything else `self.pool(rolls).keep(index).sum()`); without an index `rolls` is itself the sequence of counts,
 * `self.pool(len(rolls)).keep(rolls).sum()`. @internal */
export function dieKeep(die: Die, rolls: Int | readonly Int[], index: Int | readonly unknown[] | null): Die {
  if (isPyInt(rolls)) {
    if (index === null || index === undefined) {
      throw valueError("If the number of rolls is an integer, an index argument must be provided.");
    }
    const rolled = die.pool(rolls);
    if (isPyInt(index)) return rolled.keep(index as Int);
    return rolled.keep(index as readonly (Int | typeof Ellipsis)[]).sum();
  }
  if (index !== null && index !== undefined) throw valueError("Only one index sequence can be given.");
  return die.pool(pyLen(rolls)).keep(rolls as readonly Int[]).sum();
}

/** `Die.keep(rolls, slice(start, stop, step))`: `self.pool(rolls)[start:stop:step].sum()`; a non-int `rolls` with an
 * index raises. @internal */
export function dieKeepSlice(die: Die, rolls: Int, start: Int | null, stop: Int | null, step: Int | null): Die {
  if (!isPyInt(rolls)) throw valueError("Only one index sequence can be given.");
  return die.pool(rolls).slice(start, stop, step).sum();
}

// --- The free functions -------------------------------------------------------------------------------------------

/** Python's `tuple(x)` of the iterable a free keep function was given as its only argument, after its `len(x)`
 * (`TypeError` for an object without one): a die or deck iterates its outcomes, a Map (Python's dict) its keys, a
 * sequence its elements. */
function elements(x: unknown): unknown[] {
  pyLen(x);
  if (x instanceof Die || isDeck(x)) return x.outcomes();
  if (x instanceof Map) return dictItems(x)[0];
  return unpack(x);
}

/** The arguments of `highest(*args)` & co.: `args = arg0` if it is the only argument (an iterable of them), else
 * `(arg0,) + more_args`. */
function keepArgs(op: string, args: readonly unknown[]): unknown[] {
  // Python's `arg0` is positional-only and required (one argument is an iterable of them, which may be empty).
  if (args.length === 0) {
    throw new IcepoolError("WrongType", "TypeError", `${op}() missing 1 required positional argument: 'arg0'`);
  }
  return args.length === 1 ? elements(args[0]) : [...args];
}

/** `Die([default])` for no dice, or Python's `ValueError` without a default. */
function emptyDefault(op: string, fallback: unknown): Die {
  if (fallback === null || fallback === undefined) {
    throw valueError(`${op}() arg is an empty sequence and no default was provided.`);
  }
  return new Die([fallback as Outcome]);
}

/** `icepool.highest(*args, keep=, drop=, default=)` (`highest`) / `icepool.lowest(...)`. @internal */
export function highestLowest(highest: boolean, args: readonly unknown[], keep: unknown, drop: unknown, fallback: unknown): Die {
  const op = highest ? "highest" : "lowest";
  const all = keepArgs(op, args);
  if (all.length === 0) return emptyDefault(op, fallback);
  return sumSlice(all, keepDropSlice(highest, keep, drop));
}

/** `icepool.middle(*args, keep=, tie=, default=)`: `icepool.Pool(args).middle(keep, tie=tie).sum()`, where `args`
 * is the only argument itself (not its elements) if there is one. @internal */
export function middleOf(args: readonly unknown[], keep: Int, tie: "error" | "high" | "low", fallback: unknown): Die {
  if (keepArgs("middle", args).length === 0) return emptyDefault("middle", fallback);
  const dice = (args.length === 1 ? args[0] : [...args]) as Parameters<typeof pool>[0];
  // Expression evaluators are difficult to type.
  return pool(dice).middle(keep, { tie }).sum();
}

/** keep.py `_sum_slice(*args, index_slice)`: common code for `lowest` and `highest`. Keeps `index` of the sorted
 * rolls of the dice (not converted to `Die` yet) and sums. */
function sumSlice(args: readonly unknown[], index: Bounds): Die {
  const dice = args.map(toDie);
  if (dice.some((die) => die.isEmpty())) return new Die([]);
  // Faster special cases.
  const n = BigInt(dice.length);
  const [start, stop] = canonicalSlice(index, n);
  // `sum()` starts from the int 0.
  const sum = (xs: Iterable<Die>): Die => {
    let total: unknown = 0;
    for (const x of xs) total = binary("add", total, x);
    return total as Die;
  };
  // Zero-size slice. The inputs should support addition.
  if (start === stop) return sum((function* () { for (const die of dice) yield die.zero(); })());
  // All dice selected.
  if (start === 0n && stop === n) return sum(dice);
  if (start === 0n && stop === 1n) return keepSingleOf(dice, false);
  if (start === n - 1n && stop === n) return keepSingleOf(dice, true);
  // Use pool.
  return pool(dice).slice(...index).sum();
}

/** keep.py `_highest_single(*dice)` (`highest`) / `_lowest_single(*dice)`: roll all the dice and take the highest
 * (lowest) single one. The core takes the dice over, so it gets copies. */
function keepSingleOf(dice: readonly Die[], highest: boolean): Die {
  return Die.wrap(wasm((w) => w.keepSingleOf(dice.map((die) => die.w.copy()), highest)));
}

/** `implicit_convert_to_die(x)`: a die as it is, anything else `Die([x])`; an Again expression cannot be. */
function toDie(x: unknown): Die {
  if (x instanceof Die) return x;
  if (x instanceof AgainExpression) {
    throw new IcepoolError("WrongType", "ImplicitConversionError", "Again expression cannot be implicitly converted to a Die.");
  }
  return new Die([x as Outcome]);
}
