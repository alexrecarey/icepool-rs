// Multiset evaluators written in JS (evaluator/multiset_evaluator.py) and `@multiset_function`
// (evaluator/multiset_function.py). INTERIM (task C6): through the bridge (rpc.ts).

import { list, opts, type Die, type MapResult } from "./die.js";
import type { MultiDeal, MultisetTupleExpression } from "./deck.js";
import type { MultisetArg, MultisetExpression } from "./multiset.js";
import { HANDLE, Handle, call, callWith, enc, kwargsText, registerEvaluator, registerKind, type EvaluatorMethods } from "./rpc.js";
import { List, type OrderValue, type Outcome, type Reroll } from "./values.js";
import { wasm } from "./wasm.js";

/** A state the core can hash: values, arrays of them, or plain objects (keyed by their JSON).
 *
 * @example
 * ```ts
 * import { d, evaluator, type State } from "icepool-wasm";
 *
 * // The state is [count of ones, count of sixes].
 * const onesAndSixes = evaluator({
 *   initialState: (): State => [0, 0],
 *   nextState: ([ones, sixes]: number[], order: number, outcome: number, count: number): State =>
 *     [ones + (outcome === 1 ? count : 0), sixes + (outcome === 6 ? count : 0)],
 * });
 * onesAndSixes.evaluate(d(6).pool(2)).probability([1, 1]).toString(); //=> "1/18"
 * ```
 */
export type State = Outcome | { [key: string]: unknown } | readonly State[];

/** What an evaluation takes: pools, deals, expressions or arrays of outcomes, and a trailing options object of
 * Python's keyword arguments (which reach `initialState` and `finalOutcome`).
 *
 * @example
 * ```ts
 * import { d, evaluator, type EvaluateArg } from "icepool-wasm";
 *
 * const atLeast = evaluator({
 *   initialState: () => 0,
 *   nextState: (n: number, order: number, outcome: number, count: number) => n + count,
 *   finalOutcome: (n: number, order: number, outcomes: number[], size: unknown, { target }: { target: number }) =>
 *     n >= target,
 * });
 * const args: EvaluateArg[] = [d(6).pool(3).keepOutcomes([5, 6]), { target: 2 }];
 * atLeast.evaluate(...args).probability(true).toString(); //=> "7/27"
 * ```
 */
export type EvaluateArg = MultisetArg | MultiDeal | { [kwarg: string]: unknown };

/** `icepool.MultisetEvaluator`: evaluates multisets with a state transition function. Write one as a subclass:
 *
 *   nextState(state, order, outcome, ...counts)      required; return the next state, or Reroll
 *   initialState(order, outcomes, ...sizes)          optional; default null
 *   finalOutcome(finalState, order, outcomes, ...sizes)  optional; default the final state; may return a die
 *   extraOutcomes(outcomes)                          optional; extra outcomes for nextState to see
 *
 * (or call {@link evaluator} with an object of these methods). `evaluate(...inputs)` then calls `initialState`
 * once, `nextState` once per outcome, in the order (`Order.Ascending` or `Order.Descending`) the core chose, with
 * how many of that outcome each input rolled, and `finalOutcome` on each final state: the result is the die of
 * final outcomes, weighted by how likely each final state is. The core computes it with dynamic programming over
 * the states, so `nextState` must depend only on its arguments, and fewer distinct states are faster.
 *
 * States must be values the core can hash: numbers, BigInts, strings, booleans, null, arrays of those, or plain
 * objects, which are keyed by their JSON (so two objects with the same JSON are the same state). Arrays come back
 * as arrays, objects as fresh objects. Throw `UnsupportedOrder` to ask for the other order. A subclass may narrow
 * the parameter types (`nextState(state: number | null, order, outcome: number, count: number)`).
 *
 * @example
 * ```ts
 * // How many sixes? The initial state is null.
 * class CountSixes extends MultisetEvaluator {
 *   nextState(state: number | null, order: number, outcome: number, count: number): number {
 *     return (state ?? 0) + (outcome === 6 ? count : 0);
 *   }
 * }
 * new CountSixes().evaluate(d(6).pool(3)).items(); //=> [[0, 125n], [1, 75n], [2, 15n], [3, 1n]]
 *
 * // The sum of a pool, starting from 0.
 * class SumPool extends MultisetEvaluator {
 *   initialState(): number {
 *     return 0;
 *   }
 *   nextState(state: number, order: number, outcome: number, count: number): number {
 *     return state + outcome * count;
 *   }
 * }
 * new SumPool().evaluate(d(6).pool(3)).equals(d(6).pool(3).sum()); //=> true
 * new SumPool().evaluate([1, 2, 2]).items(); //=> [[5, 1n]]
 * ```
 */
export abstract class MultisetEvaluator {
  /** The next state after seeing `outcome` with these counts (one per input). */
  abstract nextState(state: unknown, order: OrderValue, outcome: Outcome, ...counts: unknown[]): State | typeof Reroll;

  /** `evaluate(...inputs, { ...kwargs })`: the die of final outcomes. Each input is a multiset (a pool, a deal, an
   * expression, or an array of outcomes; the union of their outcomes must be totally orderable); a trailing plain
   * object passes its entries as keyword arguments to `initialState` and `finalOutcome`. Inside a
   * {@link multisetFunction}, it records the evaluation instead.
   *
   * @example
   * ```ts
   * const pairs = evaluator({
   *   nextState: (n: number | null, order: number, outcome: number, count: number) => (n ?? 0) + (count >= 2 ? 1 : 0),
   * });
   * pairs.evaluate(d(6).pool(3)).items(); //=> [[0, 120n], [1, 96n]]
   * pairs.evaluate([1, 1, 2, 2, 3]).items(); //=> [[2, 1n]]
   * ```
   */
  evaluate(...rest: EvaluateArg[]): Die {
    return evaluateWith(this as unknown as EvaluatorMethods, rest);
  }
}

/** The optional methods a `MultisetEvaluator` subclass may define. */
export interface MultisetEvaluator {
  /** `initial_state(order, outcomes, *sizes, **kwargs)`: the state before any outcome is seen (default null).
   * `outcomes` are all the outcomes `nextState` will see; `sizes` one per input, the size of the multiset if it is
   * known with non-negative counts, otherwise null; then the options object of `evaluate`'s keyword arguments, if
   * any. Throw `UnsupportedOrder` if `order` is not supported. */
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): State;
  /** `final_outcome(final_state, order, outcomes, *sizes, **kwargs)`: the outcome of the result die for a final
   * state (default the state itself). It may be a die, or `Reroll`. With no outcomes at all, it is called at once
   * with the initial state; null is not a valid outcome, so an evaluator whose initial state is null should handle
   * that. */
  finalOutcome?(finalState: unknown, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): MapResult | null;
  /** `extra_outcomes(outcomes)`: outcomes that `nextState` should see (with zero counts) even though no input has
   * them, given the inputs' outcomes in ascending order; e.g. every int between the lowest and the highest, to
   * see gaps. Default none. */
  extraOutcomes?(outcomes: Outcome[]): Iterable<Outcome>;
}

function evaluateWith(ev: EvaluatorMethods, rest: readonly unknown[]): Die {
  const [inputs, kwargs] = opts<Record<string, unknown>>(rest);
  // The core releases the id (rpc.ts HOST.release) once it no longer needs the evaluator: when this call
  // returns, or, inside a multiset function's body, when the function holding the recorded evaluation is dropped.
  const id = registerEvaluator(ev);
  return callWith((temps) => {
    const args = `[${inputs.map((x) => enc(list(x), temps)).join(",")}]`;
    const kw = kwargsText(kwargs, temps);
    return wasm((w) => w.evaluate(id, args, kw));
  }) as Die;
}

/** The methods of an evaluator made by `evaluator(...)`: those of a {@link MultisetEvaluator} subclass, as an
 * object.
 *
 * @example
 * ```ts
 * import { d, evaluator, type EvaluatorDefinition } from "icepool-wasm";
 *
 * const definition: EvaluatorDefinition = {
 *   nextState: (high: number | null, order: number, outcome: number, count: number) => (count > 0 ? outcome : high),
 * };
 * evaluator(definition).evaluate(d(6).pool(2)).equals(d(6).pool(2).at(-1)); //=> true
 * ```
 */
export interface EvaluatorDefinition {
  /** Required: the next state after seeing `outcome` with these counts, one per input (see
   * {@link MultisetEvaluator}); `Reroll` drops the roll altogether. */
  nextState(state: unknown, order: OrderValue, outcome: Outcome, ...counts: unknown[]): State | typeof Reroll;
  /** The state before any outcome is seen (default null), as {@link MultisetEvaluator.initialState}. */
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): State;
  /** The outcome for a final state (default the state), as {@link MultisetEvaluator.finalOutcome}. */
  finalOutcome?(finalState: unknown, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): MapResult | null;
  /** Extra outcomes for `nextState` to see, as {@link MultisetEvaluator.extraOutcomes}. */
  extraOutcomes?(outcomes: Outcome[]): Iterable<Outcome>;
}

/** A `MultisetEvaluator` from an object of methods: `evaluator({ nextState: (s, order, o, c) => ... })`, without
 * writing a subclass.
 *
 * @example
 * ```ts
 * // How many outcomes are missing between the lowest and the highest roll?
 * const gaps = evaluator({
 *   nextState: (n: number | null, order: number, outcome: number, count: number) => (n ?? 0) + (count === 0 ? 1 : 0),
 *   extraOutcomes: (outcomes: number[]) =>
 *     Array.from({ length: outcomes.at(-1)! - outcomes[0] + 1 }, (_, i) => outcomes[0] + i),
 * });
 * gaps.evaluate([1, 4]).items(); //=> [[2, 1n]]
 * // Reroll: 2d6, rerolling both whenever either shows a 6.
 * const noSixes = evaluator({
 *   nextState: (sum: number | null, order: number, outcome: number, count: number) =>
 *     outcome === 6 && count > 0 ? Reroll : (sum ?? 0) + outcome * count,
 * });
 * noSixes.evaluate(d(6).pool(2)).equals(d(5).add(d(5))); //=> true
 * ```
 */
export function evaluator(methods: EvaluatorDefinition): MultisetEvaluator {
  class Made extends MultisetEvaluator {
    nextState(...args: unknown[]): State | typeof Reroll {
      return (methods.nextState as (...a: unknown[]) => State | typeof Reroll)(...args);
    }
  }
  return Object.assign(new Made(), {
    ...(methods.initialState && { initialState: methods.initialState }),
    ...(methods.finalOutcome && { finalOutcome: methods.finalOutcome }),
    ...(methods.extraOutcomes && { extraOutcomes: methods.extraOutcomes }),
  }) as MultisetEvaluator;
}

// --- Other core objects -------------------------------------------------------------------------------------------

/** A callable the core made (the harness's catalogue functions, in the tests); user code does not make these.
 *
 * @example
 * ```ts
 * // A multiset function is a different kind of core object.
 * multisetFunction((a: MultisetExpression) => a.size()) instanceof CoreFunction; //=> false
 * ```
 */
export class CoreFunction extends Handle {}

/** A `@multiset_function`: from `multisetFunction(f)`. `f.call(...inputs, { star, ...kwargs })` evaluates it
 * (Python's `f(*inputs, star=star, **kwargs)`); `star` may be left out (then it is inferred), the other options are
 * the function's keyword-only arguments.
 *
 * @example
 * ```ts
 * const diff = multisetFunction((a: MultisetExpression, b: MultisetExpression) => a.sub(b).size());
 * diff.call([1, 2, 3], [3]).items(); //=> [[2, 1n]]
 * diff.call(d(6).pool(3), d(6).pool(2)).mean().toString(); //=> "319/144"
 * ```
 */
export class MultisetFunction extends Handle {
  /** Evaluates the function on these inputs: Python's `f(*inputs, star=star, **kwargs)`. The inputs are multisets
   * (pools, deals, expressions, arrays of outcomes); a trailing options object gives `star` (whether to unpack the
   * first input, a `MultiDeal`, into its hands; inferred if left out) and the function's keyword-only arguments.
   *
   * @example
   * ```ts
   * const both = multisetFunction((a: MultisetExpression, b: MultisetExpression) => [
   *   a.sub(b).size(),
   *   b.sub(a).size(),
   * ]);
   * both.call([1, 2, 3], [3, 4]).items(); //=> [[[2, 1], 1n]]
   * // Two hands of 2 from the cards 1 to 4: `star` unpacks them into `a` and `b`.
   * both.call(new Deck([1, 2, 3, 4]).deal([2, 2]), { star: true }).items(); //=> [[[2, 2], 6n]]
   * ```
   */
  call(...rest: unknown[]): Die {
    const [args, { star, ...kwargs }] = opts<{ star: boolean | null; [kwarg: string]: unknown }>(rest);
    // `star` left out (undefined) is inferred; any other value is passed, as Python's `star=...` (even null).
    const items: [string, unknown][] = Object.entries(kwargs);
    if (star !== undefined) items.unshift(["star", star]);
    return call("mf_call_kw", { f: this, args: List.from(args), kwargs: List.from(items) }) as Die;
  }
  /** The same as `call` (Python's `evaluator.evaluate(...)`).
   *
   * @example
   * ```ts
   * const sixes = multisetFunction((a: MultisetExpression) => a.keepOutcomes([6]).size());
   * sixes.evaluate(d(6).pool(2)).equals(sixes.call(d(6).pool(2))); //=> true
   * ```
   */
  evaluate(...rest: unknown[]): Die {
    return this.call(...rest);
  }
}

/** A parameter of a multiset function: one input, or one hand of an unstarred MultiDeal input.
 *
 * @example
 * ```ts
 * import { Deck, multisetFunction, MultisetTupleExpression, type MultisetParameter } from "icepool-wasm";
 *
 * // An unstarred MultiDeal input arrives as a MultisetTupleExpression, its hands as `hand(i)`.
 * const firstMinusSecond = multisetFunction((hands: MultisetParameter) =>
 *   hands instanceof MultisetTupleExpression ? hands.hand(0).sub(hands.hand(1)).size() : hands.size(),
 * );
 * firstMinusSecond.call(new Deck([1, 2, 3, 4]).deal([2, 2])).items(); //=> [[2, 6n]]
 * firstMinusSecond.call([1, 2, 3]).items(); //=> [[3, 1n]]
 * ```
 */
export type MultisetParameter = MultisetExpression | MultisetTupleExpression;

/** `@multiset_function` (experimental): a multiset evaluator from a JS function of multisets, as Python's decorator
 * makes one.
 *
 * Calling the result traces `f`: it is called with one parameter expression per input (a `MultisetExpression`,
 * or a `MultisetTupleExpression` for a `MultiDeal` input, unless `star` unpacks the first input into its hands),
 * builds expressions with their methods, and returns an evaluation of them (`a.sub(b).size()`, a JS evaluator's
 * `evaluate(a)`, another multiset function's `call(a)`), or an array of evaluations (a joint evaluation, whose
 * outcomes are arrays; it may also hold constants). Evaluations inside `f` only record what to evaluate: the core
 * evaluates the whole function at once. `f` may be traced several times, so it should be a pure function, and it
 * cannot branch on the value of an expression or evaluation (`if (a.equalSet(b))` is not allowed).
 *
 * Python reads the signature from the function; here `params` (default `f.length`) is the number of required
 * positional parameters, `rest` says `f` takes any number more (`(...xs) => ...`), and `kwonly` names its
 * keyword-only parameters, which `f` receives as a trailing options object: `(a, { target } = {}) => ...` with
 * `{ kwonly: ["target"] }` (give it a default, as for other callbacks, or `f.length` counts it).
 *
 * @example
 * ```ts
 * // How many more elements does `a` have than `b`, one for one?
 * const diff = multisetFunction((a: MultisetExpression, b: MultisetExpression) => a.sub(b).size());
 * diff.call(d(6).pool(3), d(6).pool(2)).mean().toString(); //=> "319/144"
 *
 * // A keyword-only argument.
 * const countOutcomes = multisetFunction(
 *   (a: MultisetExpression, { target }: { target?: number[] } = {}) => a.keepOutcomes(target!).size(),
 *   { kwonly: ["target"] },
 * );
 * countOutcomes.call(d(6).pool(2), { target: [5, 6] }).items(); //=> [[0, 16n], [1, 16n], [2, 4n]]
 * ```
 */
export function multisetFunction(
  f: (...args: never[]) => unknown,
  { params = f.length, rest = false, kwonly = [] }: { params?: number; rest?: boolean; kwonly?: readonly string[] } = {},
): MultisetFunction {
  if (typeof f !== "function") throw new TypeError("icepool: multisetFunction takes a function");
  return callWith((temps) => {
    const text = `{"f":${enc(f, temps)},"params":${Number(params)},"var":${rest ? "true" : "false"},"kwonly":${JSON.stringify(Array.from(kwonly, String))}}`;
    return wasm((w) => w.multiset_function(text));
  }) as MultisetFunction;
}

/** A multiset function's result in the program language (the fixture tests'). */
class RawResult extends Handle {}

registerKind("fn", (h) => new CoreFunction(HANDLE, h));
registerKind("mfunc", (h) => new MultisetFunction(HANDLE, h));
registerKind("raw", (h) => new RawResult(HANDLE, h));
