// Pools from dice (EXPERIMENTAL in icepool): `icepool.map_to_pool` (map_tools/function.py) and the `Die` methods
// built on it (population/die.py `map_to_pool`, `explode_to_pool`, `reroll_to_pool`). Each makes a
// `MultisetMixture` of pools, one per joint outcome of the arguments, weighted by its quantity (equal pools counted
// together). The arithmetic of the pool compositions is the core's (src/api/to_pool.rs); here are Python's calling
// conventions and the pools each composition stands for, as in the runner (crates/icepool-harness/src/to_pool.rs).

import { Die, list, opts, pyBool, unpack, type PoolRepl, type Which } from "./die.js";
import { IcepoolError } from "./errors.js";
import { cartesianItems, jointItems, mappingGet, notARepl, unexpectedKeyword, MAPPING_LAMBDA } from "./functions.js";
import { MultisetExpression, Pool, mixtureOf, native } from "./multiset.js";
import { binary } from "./operators.js";
import { List, Reroll, Restart, Vector, isPlainValue, pyTypeName, type Int } from "./values.js";
import { call } from "./rpc.js";
import { wasm } from "./wasm.js";

const unported = (what: string) => new IcepoolError("Unported", "NotImplementedError", what);

/** `icepool.map_to_pool(repl, *args, star=star, **kwargs)`: `repl` (a function, a Map, or `Die.map_to_pool`'s
 * identity lambda) called with every joint outcome of the arguments. `bridge` is the bridge call the result stands
 * for. @internal */
export function mapToPoolOf(
  repl: PoolRepl | typeof IDENTITY,
  args: readonly unknown[],
  star: unknown,
  kwargs: Record<string, unknown>,
  bridge: () => unknown,
): MultisetExpression {
  const hasKwargs = Object.keys(kwargs).length > 0;
  // common.py `transition_and_star`: a mapping becomes `lambda o: mapping.get(o, o)`, never starred.
  let transition: (xs: unknown[]) => unknown;
  let starred: boolean;
  if (repl === IDENTITY) {
    starred = star === null || star === undefined ? false : pyBool(star);
    transition = (xs) => identity(xs, kwargs);
  } else if (typeof repl === "function") {
    const fn = repl as (...args: unknown[]) => unknown;
    starred = star === null || star === undefined ? fn.length > args.length : pyBool(star);
    // Keyword arguments reach `repl` as a trailing options object, when there are any.
    transition = (xs) => (hasKwargs ? fn(...xs, kwargs) : fn(...xs));
  } else if (repl instanceof Map) {
    if (args.length !== 1) throw new IcepoolError("InvalidArgument", "ValueError", "If a mapping is provided for repl, len(args) must be 1.");
    starred = false;
    transition = ([x]) => {
      if (hasKwargs) throw unexpectedKeyword(MAPPING_LAMBDA, Object.keys(kwargs)[0]);
      return mappingGet(repl, [x]).get(x);
    };
  } else {
    throw notARepl();
  }
  return native(null, () => {
    const weighted: [MultisetExpression, bigint][] = [];
    for (const [outcomes, quantity] of jointItems(args.map(cartesianItems))) {
      if (starred && outcomes.length === 0) {
        // `transition_function(*outcomes[0], ...)` with no args at all.
        throw new IcepoolError("IndexOutOfRange", "IndexError", "tuple index out of range");
      }
      const pool = transition(starred ? [...unpack(outcomes[0]), ...outcomes.slice(1)] : outcomes);
      const inner = asPool(pool);
      if (inner !== undefined) weighted.push([inner, quantity]);
    }
    // I couldn't get the covariance / contravariance to work.
    return mixtureOf(weighted);
  }, bridge);
}

/** One result of `repl`: `undefined` for `Reroll` / `Restart` (`pool in icepool.REROLL_TYPES`), else the expression,
 * or `icepool.Pool(result)`. */
function asPool(result: unknown): MultisetExpression | undefined {
  if (result === Reroll || result === Restart) return undefined;
  // `Vector in REROLL_TYPES` compares element by element: not modelled (nor is it by the runner).
  if (result instanceof Vector) throw unported("a Vector as a map_to_pool result (Vector == Reroll)");
  if (result instanceof MultisetExpression) return result;
  return new Pool(result as ConstructorParameters<typeof Pool>[0]);
}

/** `Die.map_to_pool`'s `repl` when none is given: `lambda x: x`. */
const IDENTITY = Symbol("lambda x: x");

/** Python's `TypeError`s for calling `lambda x: x` (`Die.map_to_pool.<locals>.<lambda>`) with anything but one
 * positional argument; else that argument. */
function identity(xs: readonly unknown[], kwargs: Record<string, unknown>): unknown {
  const qualname = "Die.map_to_pool.<locals>.<lambda>";
  const [name] = Object.keys(kwargs);
  if (xs.length === 0) throw typeError(`${qualname}() missing 1 required positional argument: 'x'`);
  if (xs.length > 1) throw typeError(`${qualname}() takes 1 positional argument but ${xs.length} were given`);
  if (name !== undefined) throw unexpectedKeyword(qualname, name);
  return xs[0];
}

const typeError = (message: string) => new IcepoolError("WrongType", "TypeError", message);

/** `die.map_to_pool(repl, *extra_args, star=star, **kwargs)`: `icepool.map_to_pool(repl, die, ...)`, where a missing
 * `repl` is `lambda x: x` (the outcomes are the pools' contents). @internal */
export function dieMapToPool(die: Die, repl: PoolRepl | null, rest: readonly unknown[]): MultisetExpression {
  const [args, { star = null, ...kwargs }] = opts<{ star: boolean | null; [kwarg: string]: unknown }>(rest);
  const bridge = () => {
    const [bound, boundStar] = bindForBridge(repl, args.length + 1, star, kwargs, "Die.map_to_pool.<locals>.<lambda>");
    return call("die_map_to_pool", { die, f: bound, args: List.from(args), star: boundStar });
  };
  return mapToPoolOf(repl ?? IDENTITY, [die, ...args], star, kwargs, bridge);
}

/** `icepool.map_to_pool(repl, *args, star=, **kwargs)` of the API's arguments. @internal */
export function mapToPool(repl: PoolRepl, rest: readonly unknown[]): MultisetExpression {
  const [args, { star = null, ...kwargs }] = opts<{ star: boolean | null; [kwarg: string]: unknown }>(rest);
  const bridge = () => {
    const [bound, boundStar] = bindForBridge(repl, args.length, star, kwargs);
    return call("map_to_pool", { f: bound, args: List.from(args), star: boundStar });
  };
  return mapToPoolOf(repl, args, star, kwargs, bridge);
}

/** `repl` with Python's `**kwargs` bound, for the bridge's copy of a result (which calls `repl` with none): a
 * function gets them as a trailing options object, its `star` inferred from the unbound function first. A Map or
 * the identity lambda (`null`) raises Python's `TypeError` when called. Unchanged without keyword arguments. */
function bindForBridge(
  repl: PoolRepl | null,
  nArgs: number,
  star: unknown,
  kwargs: Record<string, unknown>,
  qualname = MAPPING_LAMBDA,
): [unknown, unknown] {
  const [name] = Object.keys(kwargs);
  if (name === undefined) return [repl, star];
  if (typeof repl === "function") {
    const fn = repl as (...args: unknown[]) => unknown;
    const starred = star === null || star === undefined ? fn.length > nArgs : pyBool(star);
    return [(...xs: unknown[]) => fn(...xs, kwargs), starred];
  }
  return [() => { throw unexpectedKeyword(qualname, name); }, false];
}

// --- explode_to_pool, reroll_to_pool ------------------------------------------------------------------------------

/** A count argument (`rolls`, `max_rerolls`, `depth`): any plain value, used with Python's arithmetic (`@`, `==`,
 * `-`, `**`), which raises where Python does. */
function countArg(x: unknown, what: string): unknown {
  if (!isPlainValue(x)) throw unported(`a ${what} that is not a value`);
  return x;
}

/** The outcomes `which` selects (`_select_outcomes`, then `split` on that set, which selects the same ones), or the
 * single extreme one (`{self.min_outcome()}` / `{self.max_outcome()}`, raising for an empty die) for `null`. */
function selection(die: Die, which: Which | undefined, star: unknown, max: boolean): Uint8Array {
  if (which !== null && which !== undefined) return die.selection(which, star);
  if (max) die.maxOutcome();
  else die.minOutcome();
  const mask = new Uint8Array(die.size);
  mask[max ? die.size - 1 : 0] = 1;
  return mask;
}

/** `die.split(selected)`: the selected outcomes and the rest. */
function split(die: Die, mask: Uint8Array): [Die, Die] {
  const w = die.w;
  const [selected, rest] = wasm(() => w.split(mask));
  return [Die.wrap(selected), Die.wrap(rest)];
}

/** `Pool(list_of_dice)` of `[die] * count + ...`: each die put in `count` times (a die put in zero times would
 * still add its outcomes to the pool's: `[die] * 0` leaves it out). */
function poolOf(dice: readonly (readonly [Die, unknown])[]): Pool {
  const kept = dice.filter(([, count]) => {
    if (typeof count !== "bigint" && typeof count !== "number") throw typeError("can't multiply sequence by non-int");
    return count > 0;
  });
  return new Pool(kept.map(([die]) => die), kept.map(([, count]) => count as Int));
}

/** `die.explode_to_pool(rolls, outcomes, star=star, depth=depth)`: each of `rolls` dice explodes up to `depth`
 * times, each roll a separate die in the pool. @internal */
export function explodeToPool(die: Die, rolls: Int, outcomes: Which, star: boolean | null, depth: Int): MultisetExpression {
  if (pyBool(binary("eq", depth, 0))) return die.pool(rolls);
  const mask = selection(die, outcomes, star, true);
  if (!mask.includes(1)) return die.pool(rolls);
  const [explode, notExplode] = split(die, mask);
  // `range(depth + 1)`.
  if (!(typeof depth === "boolean" || typeof depth === "bigint" || Number.isInteger(depth))) {
    throw typeError(`'${pyTypeName(depth)}' object cannot be interpreted as an integer`);
  }
  const n = countArg(rolls, "explode_to_pool rolls");
  const bridge = () => call("explode_to_pool", { die, rolls, outcomes: list(outcomes), star, depth });
  return native(null, () => {
    const countDie = Die.wrap(wasm((w) => w.explodeCounts(die.w, explode.w, notExplode.w, n, typeof depth === "boolean" ? Number(depth) : depth)));
    // `count_die.map_to_pool(lambda x, nx: [explode] * x + [not_explode] * nx)`.
    const weighted = countDie.items().map(([counts, quantity]): [MultisetExpression, bigint] => {
      const [x, nx] = [...(counts as Vector)];
      return [poolOf([[explode, x], [notExplode, nx]]), quantity];
    });
    return mixtureOf(weighted);
  }, bridge);
}

/** The options of `Die.reroll_to_pool`. */
interface RerollToPoolOptions {
  maxRerolls: Int;
  star: boolean | null;
  depth: Int;
  mode: unknown;
}

/** `die.reroll_to_pool(rolls, outcomes, max_rerolls=, star=, depth=, mode=)`: a limited number of rerolls shared
 * across a pool. @internal */
export function rerollToPool(die: Die, rolls: Int, outcomes: Which | undefined, o: RerollToPoolOptions): MultisetExpression {
  const inf = (x: unknown) => x === Infinity || x === "inf";
  if (inf(o.maxRerolls)) return die.reroll(outcomes ?? null, { star: o.star, depth: o.depth }).pool(rolls);
  const mask = selection(die, outcomes, o.star, false);
  if (!mask.includes(1)) return die.pool(rolls);
  const [rerollableDie, notRerollableDie] = split(die, mask);
  const n = countArg(rolls, "reroll_to_pool rolls");
  const maxRerolls = countArg(o.maxRerolls, "max_rerolls");
  const depth = inf(o.depth) ? maxRerolls : countArg(o.depth, "reroll_to_pool depth");
  // `match mode: case 'random': ...`: a value that is not a str matches none of the cases, and the error shows its
  // `str`.
  const mode = typeof o.mode === "string" ? o.mode : pyStr(o.mode);
  const bridge = () => call("reroll_to_pool", {
    die,
    rolls,
    outcomes: list(outcomes ?? null),
    max_rerolls: o.maxRerolls,
    star: o.star,
    depth: inf(o.depth) ? "inf" : o.depth,
    mode: o.mode,
  });
  return native(null, () => {
    const composition = Die.wrap(wasm((w) => w.rerollComposition(die.w, rerollableDie.w, n, maxRerolls, depth)));
    // `pool_composition.map_to_pool(make_pool, star=True)`.
    const weighted = composition.items().map(([parts, quantity]): [MultisetExpression, bigint] => {
      const [rerollable, rerolled, rerolledToRerollable] = parts as Int[];
      return [makePool(rerollableDie, notRerollableDie, n, mode, rerollable, rerolled, rerolledToRerollable), quantity];
    });
    return mixtureOf(weighted);
  }, bridge);
}

/** `reroll_to_pool`'s `make_pool(rerollable, rerolled, rerolled_to_rerollable)`. */
function makePool(
  rerollableDie: Die,
  notRerollableDie: Die,
  rolls: unknown,
  mode: string,
  rerollable: Int,
  rerolled: Int,
  rerolledToRerollable: Int,
): MultisetExpression {
  const sub = (a: unknown, b: unknown) => binary("sub", a, b) as Int;
  const rerollsRanOut = sub(rerollable, rerolled);
  const notRerollable = sub(sub(rolls, rerollsRanOut), rerolledToRerollable);
  const common = rerollableDie.pool(rerolledToRerollable).add(notRerollableDie.pool(notRerollable));
  switch (mode) {
    case "random":
      return common.add(rerollableDie.pool(rerollsRanOut));
    case "low":
      return common.add(rerollableDie.pool(rerollable).highest(rerollsRanOut));
    case "high":
      return common.add(rerollableDie.pool(rerollable).lowest(rerollsRanOut));
    case "drop":
      return notRerollableDie.pool(notRerollable);
    default:
      throw new IcepoolError(
        "InvalidArgument",
        "ValueError",
        `Invalid reroll_priority '${mode}'. Allowed values are 'random', 'low', 'high', 'drop'.`,
      );
  }
}

/** Python's `str()` of a plain value that is not a str, for an error message. */
function pyStr(x: unknown): string {
  if (x === null || x === undefined) return "None";
  if (typeof x === "boolean") return x ? "True" : "False";
  return String(x);
}
