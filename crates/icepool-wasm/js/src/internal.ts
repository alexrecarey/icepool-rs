// Test-only: the low level the JS tests reach (test/fixtures.mjs translates fixture programs into calls of the
// public API, and uses these for what has no public counterpart: the bridge to the program runner, rpc.ts, and the
// evaluator objects of the built-in classes below). Neither this file nor rpc.ts is in the npm package.

export { BridgeObject, RawJSON, bridgeCalls, call, fromWire, liveHandles, setStandIn, wireResult } from "./rpc.js";
export { run } from "./wasm.js";

import type { Die } from "./die.js";
import { IcepoolError } from "./errors.js";
import { builtinEvaluation, selectFunction } from "./multiset.js";
import { liveHandles } from "./rpc.js";
import { isPlainValue } from "./values.js";
import { wasm } from "./wasm.js";

/** How many core objects JS holds: handles of the bridge and typed dice (for leak checks). */
export function liveObjects(): number {
  return liveHandles() + wasm((w) => w.liveDice());
}

// --- The evaluator objects of the built-in classes ----------------------------------------------------------------
//
// Python's `icepool.evaluator` exports the evaluator classes behind the built-in evaluations, and the fixtures call
// them directly (`builtin_evaluate`, `keep_evaluate`, `straight_and_outcome_evaluate`). The TypeScript API has no
// counterpart: every built-in is a method of `MultisetExpression` (`sum()`, `keep(i)`, `largestStraightAndOutcome`,
// `issubset`, ...), and JS evaluators subclass `MultisetEvaluator`. So the fixtures reach the core's built-ins
// through this test-only route, which maps a class and its arguments onto them as the Python facade does
// (python/icepool/evaluator/evaluator_rust.py `builtin_spec`).

/** `[name, value]` pairs of keyword arguments, as Python's `dict(pairs)`: the last value of a name wins. */
type Kwargs = readonly (readonly [string, unknown])[];

const isInt = (x: unknown): boolean => typeof x === "bigint" || Number.isInteger(x);

/** The core's built-in for a class: its name, how many inputs it takes (`null`: any number but none), and its
 * parameters from the constructor's (`params`, `ctor`) and `evaluate`'s keyword arguments. `undefined` for what
 * `builtin_spec` leaves to the evaluator's own methods (e.g. two inputs to `SumEvaluator`, an unknown keyword):
 * Python's errors there come from those methods, which the bridge runs (in the fixture tests). */
type Spec = { name: string; inputs: number | null; params: unknown[] };

/** The classes whose constructor takes no arguments and whose `evaluate` takes none, and their core names. */
const SIMPLE: Record<string, [string, number]> = {
  SumEvaluator: ["sum", 1],
  SizeEvaluator: ["size", 1],
  ProductOfCountsEvaluator: ["product_of_counts", 1],
  HighestOutcomeAndCountEvaluator: ["highest_outcome_and_count", 1],
  LargestCountEvaluator: ["largest_count", 1],
  LargestCountAndOutcomeEvaluator: ["largest_count_and_outcome", 1],
  LargestStraightEvaluator: ["largest_straight", 1],
  AllStraightsEvaluator: ["all_straights", 1],
  // `reducer=operator.mul`: the core's default.
  AllStraightsReduceCountsEvaluator: ["all_straights_reduce_counts", 1],
  IsProperSubsetEvaluator: ["is_proper_subset", 2],
  IsSubsetEvaluator: ["is_subset", 2],
  IsProperSupersetEvaluator: ["is_proper_superset", 2],
  IsSupersetEvaluator: ["is_superset", 2],
  IsEqualSetEvaluator: ["is_equal_set", 2],
  IsNotEqualSetEvaluator: ["is_not_equal_set", 2],
  IsDisjointSetEvaluator: ["is_disjoint_set", 2],
};

const ARGSORT_TIES: Record<string, string> = {
  ArgsortEvaluatorDrop: "drop",
  ArgsortEvaluatorLeft: "left",
  ArgsortEvaluatorRight: "right",
};

/** `getattr(icepool.evaluator, cls)(*params, **ctor)`, then the `evaluate` keyword arguments `kwargs`, as a core
 * built-in. The constructor's own errors are raised here, as Python raises them before `evaluate`. */
function builtinSpec(cls: string, params: readonly unknown[], ctorPairs: Kwargs, kwargPairs: Kwargs): Spec | undefined {
  const ctor = new Map(ctorPairs);
  const kwargs = new Map(kwargPairs);
  // Only these keyword arguments (each optional) and this many positional ones.
  const only = (m: Map<string, unknown>, ...names: string[]) => [...m.keys()].every((k) => names.includes(k));
  const plain = (positional: number) => params.length === positional && ctor.size === 0 && kwargs.size === 0;
  if (cls in SIMPLE) {
    const [name, inputs] = SIMPLE[cls];
    return plain(0) ? { name, inputs, params: [] } : undefined;
  }
  switch (cls) {
    case "EmptyEvaluator": {
      // `any` is a keyword argument of `initial_state` / `final_outcome`, read as a bool.
      const any = kwargs.get("any") ?? false;
      return params.length === 0 && ctor.size === 0 && only(kwargs, "any") && typeof any === "boolean"
        ? { name: "empty", inputs: 1, params: [any] }
        : undefined;
    }
    case "ExpandEvaluator": {
      // `reverse = order != Order.Ascending` in `final_outcome`.
      const order = kwargs.get("order") ?? 1;
      return params.length === 0 && ctor.size === 0 && only(kwargs, "order") && (isInt(order) || typeof order === "boolean")
        ? { name: "expand", inputs: 1, params: [Number(order) === 1] }
        : undefined;
    }
    case "KeepEvaluator": {
      const index = kwargs.get("index");
      return params.length === 0 && ctor.size === 0 && kwargs.size === 1 && kwargs.has("index") && (index === null || isInt(index))
        ? { name: "keep", inputs: 1, params: [index] }
        : undefined;
    }
    case "AllCountsEvaluator":
      return params.length === 0 && kwargs.size === 0 && only(ctor, "filter")
        ? { name: "all_counts", inputs: 1, params: [ctor.has("filter") ? ctor.get("filter") : 1] }
        : undefined;
    case "CountSubsetEvaluator":
      return params.length === 0 && kwargs.size === 0 && only(ctor, "empty_divisor")
        ? { name: "count_subset", inputs: 2, params: [ctor.get("empty_divisor") ?? null] }
        : undefined;
    case "LargestStraightAndOutcomeEvaluator": {
      if (params.length !== 1 || ctor.size !== 0 || kwargs.size !== 0) return undefined;
      const [priority] = params;
      if (priority !== "low" && priority !== "high") {
        throw new IcepoolError("InvalidArgument", "ValueError", "priority must be 'low' or 'high'.");
      }
      return { name: "largest_straight_and_outcome", inputs: 1, params: [priority === "high"] };
    }
    case "LargestCountWithWildEvaluator":
      // `make_select_function` of each, in the constructor.
      return params.length === 3 && ctor.size === 0 && kwargs.size === 0
        ? { name: "largest_count_wild", inputs: 1, params: params.map(selectFunction) }
        : undefined;
    case "LexiComparisonEvaluator": {
      // What `leximin` / `leximax` pass to `evaluate`: the sort order and the five results of the comparison.
      const [order, tuple] = [kwargs.get("sort_order"), kwargs.get("lexi_tuple")];
      const fiveInts = Array.isArray(tuple) && tuple.length === 5 && tuple.every(isInt);
      return params.length === 0 && ctor.size === 0 && kwargs.size === 2 && isInt(order) && fiveInts
        ? { name: "lexi_tuple", inputs: 2, params: [order, tuple] }
        : undefined;
    }
  }
  if (cls in ARGSORT_TIES || cls === "ArgsortGroupedEvaluator") {
    // Keyword-only constructor arguments, defaults `order=Order.Descending, limit=None, limit_pad=None`; the
    // evaluation's `initial_state` compares `order != self._order`.
    const grouped = cls === "ArgsortGroupedEvaluator";
    const [order, limit, pad] = [ctor.get("order") ?? -1, ctor.get("limit") ?? null, ctor.get("limit_pad") ?? null];
    const fits = params.length === 0 && kwargs.size === 0 && (grouped ? only(ctor, "order", "limit") : only(ctor, "order", "limit", "limit_pad"));
    // Any order (the comparison is the core's Python `!=`), an int or `None` limit.
    if (!fits || !isPlainValue(order) || !(limit === null || isInt(limit))) return undefined;
    return grouped
      ? { name: "argsort_grouped", inputs: null, params: [order, limit] }
      : { name: "argsort", inputs: null, params: [ARGSORT_TIES[cls], order, limit, pad] };
  }
  return undefined;
}

/** `getattr(icepool.evaluator, cls)(*params, **dict(ctor)).evaluate(*args, **dict(kwargs))` on the core's built-in
 * (fixture node `builtin_evaluate`; `keep_evaluate` is `KeepEvaluator` with `index`, `straight_and_outcome_evaluate`
 * `LargestStraightAndOutcomeEvaluator(priority)`). `undefined` where the core has no such built-in: the caller
 * runs it through the bridge. */
export function evaluateBuiltin(cls: string, params: readonly unknown[], ctor: Kwargs, kwargs: Kwargs, args: readonly unknown[]): Die | undefined {
  const spec = builtinSpec(cls, params, ctor, kwargs);
  if (spec === undefined || (spec.inputs === null ? args.length === 0 : args.length !== spec.inputs)) return undefined;
  return builtinEvaluation(spec.name, spec.params, args);
}
