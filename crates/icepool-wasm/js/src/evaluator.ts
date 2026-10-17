// Multiset evaluators written in JS (evaluator/multiset_evaluator.py) and `@multiset_function`
// (evaluator/multiset_function.py). INTERIM (task C6): through the bridge (rpc.ts).

import { list, opts, type Die, type MapResult } from "./die.js";
import type { MultiDeal, MultisetTupleExpression } from "./deck.js";
import type { MultisetArg, MultisetExpression } from "./multiset.js";
import { HANDLE, Handle, call, callWith, enc, kwargsText, registerEvaluator, registerKind, type EvaluatorMethods } from "./rpc.js";
import { List, type OrderValue, type Outcome, type Reroll } from "./values.js";
import { wasm } from "./wasm.js";

/** A state the core can hash: values, arrays of them, or plain objects (keyed by their JSON). */
export type State = Outcome | { [key: string]: unknown } | readonly State[];

/** What an evaluation takes: pools, deals, expressions or arrays of outcomes, and a trailing options object of
 * Python's keyword arguments (which reach `initialState` and `finalOutcome`). */
export type EvaluateArg = MultisetArg | MultiDeal | { [kwarg: string]: unknown };

/** A multiset evaluator written in JS, as a subclass of icepool's `MultisetEvaluator`:
 *
 *   nextState(state, order, outcome, ...counts)      required; return the next state, or Reroll
 *   initialState(order, outcomes, ...sizes)          optional; default null
 *   finalOutcome(finalState, order, outcomes, ...sizes)  optional; default the final state; may return a die
 *   extraOutcomes(outcomes)                          optional; extra outcomes for nextState to see
 *
 * States must be values the core can hash: numbers, BigInts, strings, booleans, null, arrays of those, or plain
 * objects, which are keyed by their JSON (so two objects with the same JSON are the same state). Arrays come back
 * as arrays, objects as fresh objects. Throw `UnsupportedOrder` to ask for the other order. A subclass may narrow
 * the parameter types (`nextState(state: number | null, order, outcome: number, count: number)`). */
export abstract class MultisetEvaluator {
  /** The next state after seeing `outcome` with these counts (one per input). */
  abstract nextState(state: unknown, order: OrderValue, outcome: Outcome, ...counts: unknown[]): State | typeof Reroll;

  /** `evaluate(...inputs, { ...kwargs })`: the die of final outcomes. */
  evaluate(...rest: EvaluateArg[]): Die {
    return evaluateWith(this as unknown as EvaluatorMethods, rest);
  }
}

/** The optional methods a `MultisetEvaluator` subclass may define. */
export interface MultisetEvaluator {
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): State;
  finalOutcome?(finalState: unknown, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): MapResult | null;
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

/** The methods of an evaluator made by `evaluator(...)`. */
export interface EvaluatorDefinition {
  nextState(state: unknown, order: OrderValue, outcome: Outcome, ...counts: unknown[]): State | typeof Reroll;
  initialState?(order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): State;
  finalOutcome?(finalState: unknown, order: OrderValue, outcomes: Outcome[], ...sizesAndKwargs: unknown[]): MapResult | null;
  extraOutcomes?(outcomes: Outcome[]): Iterable<Outcome>;
}

/** A `MultisetEvaluator` from an object of methods: `evaluator({ nextState: (s, order, o, c) => ... })`. */
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

/** A callable the core made (the harness's catalogue functions, in the tests). */
export class CoreFunction extends Handle {}

/** A `@multiset_function`: from `multisetFunction(f)`. `f.call(...inputs, { star, ...kwargs })` evaluates it
 * (Python's `f(*inputs, star=star, **kwargs)`); `star` may be left out (then it is inferred), the other options are
 * the function's keyword-only arguments. */
export class MultisetFunction extends Handle {
  call(...rest: unknown[]): Die {
    const [args, { star, ...kwargs }] = opts<{ star: boolean | null; [kwarg: string]: unknown }>(rest);
    // `star` left out (undefined) is inferred; any other value is passed, as Python's `star=...` (even null).
    const items: [string, unknown][] = Object.entries(kwargs);
    if (star !== undefined) items.unshift(["star", star]);
    return call("mf_call_kw", { f: this, args: List.from(args), kwargs: List.from(items) }) as Die;
  }
  /** The same as `call` (Python's `evaluator.evaluate(...)`). */
  evaluate(...rest: unknown[]): Die {
    return this.call(...rest);
  }
}

/** A parameter of a multiset function: one input, or one hand of an unstarred MultiDeal input. */
export type MultisetParameter = MultisetExpression | MultisetTupleExpression;

/** `@multiset_function`: a multiset evaluator from a JS function of multisets, as Python's decorator makes one.
 *
 * Calling the result traces `f`: it is called with one parameter expression per input (a `MultisetExpression`,
 * or a `MultisetTupleExpression` for a `MultiDeal` input, unless `star` unpacks the first input into its hands),
 * builds expressions with their methods, and returns an evaluation of them (`a.sub(b).size()`, a JS evaluator's
 * `evaluate(a)`, another multiset function's `call(a)`), an array of evaluations (a joint evaluation, whose
 * outcomes are arrays), or a constant. Evaluations inside `f` only record what to evaluate: the core evaluates
 * the whole function at once. `f` may be traced several times, so it should be a pure function.
 *
 * Python reads the signature from the function; here `params` (default `f.length`) is the number of required
 * positional parameters, `rest` says `f` takes any number more (`(...xs) => ...`), and `kwonly` names its
 * keyword-only parameters, which `f` receives as a trailing options object: `(a, { target } = {}) => ...` with
 * `{ kwonly: ["target"] }` (give it a default, as for other callbacks, or `f.length` counts it). */
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
