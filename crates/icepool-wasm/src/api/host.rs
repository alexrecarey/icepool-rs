//! What the typed exports need from JS: the value classes of the TypeScript API (js/src/values.ts), which the
//! TypeScript side installs on the global `__icepool_wasm_host` (js/src/wasm.ts) before the first call, and two
//! plain JS globals. Declared directly, so wasm-bindgen stays the only binding dependency (no js-sys).

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// A JS object as `[tag, ...parts]`: `["array"]` (a tuple), `["list"]` (a `List`, Python's list),
    /// `["float", number]`, `["fraction", bigint, bigint]`, `["complex", number, number]`, `["vector", items | null,
    /// hasTruth, truth]`, `["symbols", items | null, str]` (items as `[char, count]` pairs), `["state", key]` (an
    /// evaluator state that is a plain object, by its canonical JSON), or `["other", typeName]` for anything else.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = describe)]
    pub(super) fn describe(x: &JsValue) -> Vec<JsValue>;

    /// A Python float that is integral (`2.0`), as the `Float` class, so it stays a float when passed back.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeFloat)]
    pub(super) fn make_float(x: f64) -> JsValue;

    /// A `Fraction` from its numerator and denominator (BigInts).
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeFraction)]
    pub(super) fn make_fraction(numerator: JsValue, denominator: JsValue) -> JsValue;

    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeComplex)]
    pub(super) fn make_complex(re: f64, im: f64) -> JsValue;

    /// A `Vector`: `items` an array, or null for a `VectorWithTruthOnly`; `truth` only if `has_truth`.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeVector)]
    pub(super) fn make_vector(items: JsValue, has_truth: bool, truth: JsValue) -> JsValue;

    /// A `Symbols` from its dict's `[char, count]` items in order, and Python's `str()` of it (null if that
    /// raises).
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeSymbols)]
    pub(super) fn make_symbols(items: Vec<JsValue>, str: Option<String>) -> JsValue;

    /// A Python object JS has no type for, as its repr.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeOpaque)]
    pub(super) fn make_opaque(repr: &str) -> JsValue;

    /// The `IcepoolError` (or subclass) for a core error: `kind` is the `icepool_core::Error` variant,
    /// `python_type` the exception Python raises.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeError)]
    pub(super) fn make_error(kind: &str, python_type: &str, message: &str) -> JsValue;

    /// An `AgainExpression` (js/src/again.ts) as `[func, args, isAdditive]` (`func` the name of its function, or
    /// null for `Again` itself); `[]` for anything else.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = againParts)]
    pub(super) fn again_parts(x: &JsValue) -> Vec<JsValue>;

    /// An Again expression's function (by name) applied to evaluated arguments with Python's operator dispatch:
    /// `[true, result]`, or `[false, exception]` for what it threw.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = againApply)]
    pub(super) fn again_apply(function: &JsValue, args: Vec<JsValue>) -> Vec<JsValue>;

    /// `new Die(objects, times)`, the TypeScript constructor: `[true, die]` or `[false, exception]`.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = againDie)]
    pub(super) fn again_die(objects: Vec<JsValue>, times: &JsValue) -> Vec<JsValue>;

    /// The transition function of a `TransitionCache` (js/src/functions.ts `transition`): `step(state, i)` calls
    /// it on a state and the `i`th joint outcome of the extra arguments (see `map_tools::next` for what it
    /// returns).
    pub type Transition;

    #[wasm_bindgen(method, structural)]
    pub(super) fn step(this: &Transition, state: JsValue, i: u32) -> Vec<JsValue>;

    /// `map_counts`' JS function `f` (js/src/multiset.ts), called on an outcome and its counts: `[true, count]`, or
    /// `[false]` when it threw (the TypeScript keeps the exception and rethrows it).
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = countFunction)]
    pub(super) fn count_function(f: &JsValue, outcome: JsValue, counts: Vec<JsValue>) -> Vec<JsValue>;

    /// `keep_outcomes`' JS callable `f`, called on an outcome: `[true, bool(result)]`, or `[false]` when it threw.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = outcomePredicate)]
    pub(super) fn outcome_predicate(f: &JsValue, outcome: JsValue) -> Vec<JsValue>;

    /// A JS `MultisetEvaluator` as the TypeScript wraps it for one evaluation (js/src/evaluator.ts `HostEvaluator`:
    /// the evaluator, `evaluate()`'s keyword arguments, and the defaults of the methods it leaves out). Each method
    /// answers `["ok", value]`, `["reroll"]` (`Reroll` or `Restart`), `["unsupported", message]` (it threw
    /// `UnsupportedOrder`) or `["throw"]` (it threw something else, which the TypeScript keeps and rethrows).
    #[derive(Clone)]
    pub type JsEvaluator;

    /// `initial_state(order, outcomes, *sizes, **kwargs)`: sizes are ints, tuples of ints or null.
    #[wasm_bindgen(method, structural, js_name = initialState)]
    pub(super) fn initial_state(this: &JsEvaluator, order: i32, outcomes: JsValue, sizes: Vec<JsValue>) -> Vec<JsValue>;

    /// `next_state(state, order, outcome, *counts)`.
    #[wasm_bindgen(method, structural, js_name = nextState)]
    pub(super) fn next_state(
        this: &JsEvaluator,
        state: JsValue,
        order: i32,
        outcome: JsValue,
        counts: Vec<JsValue>,
    ) -> Vec<JsValue>;

    /// `extra_outcomes(outcomes)`, as an array.
    #[wasm_bindgen(method, structural, js_name = extraOutcomes)]
    pub(super) fn extra_outcomes(this: &JsEvaluator, outcomes: JsValue) -> Vec<JsValue>;

    /// `final_outcome(final_state, order, outcomes, *sizes, **kwargs)`.
    #[wasm_bindgen(method, structural, js_name = finalOutcome)]
    pub(super) fn final_outcome(
        this: &JsEvaluator,
        state: JsValue,
        order: i32,
        outcomes: JsValue,
        sizes: Vec<JsValue>,
    ) -> Vec<JsValue>;

    /// Traces the multiset function `f` (js/src/evaluator.ts `MultisetFunction`) on `inputs` inputs with these
    /// keyword arguments (`MultisetFunctionEvaluator._prepare` up to the call of the wrapped function): `[true,
    /// builder, joint]`, the `ExpressionBuilder` holding what the body returned (its raw results, one unless
    /// `joint`), or `[false]` when the body (or binding its arguments) threw.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = traceFunction)]
    pub(super) fn trace_function(f: &JsValue, inputs: u32, kwargs: &JsValue) -> Vec<JsValue>;

    /// `icepool.tupleize(*final_outcomes)` of a joint multiset function: `[true, result]` or `[false]`.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = tupleize)]
    pub(super) fn tupleize(items: Vec<JsValue>) -> Vec<JsValue>;

    /// An evaluator state that was a plain JS object, from its canonical JSON: a fresh object.
    #[wasm_bindgen(js_namespace = __icepool_wasm_host, js_name = makeState)]
    pub(super) fn make_state(key: &str) -> JsValue;

    /// `String(x)`: a BigInt's decimal digits.
    #[wasm_bindgen(js_name = String)]
    pub(super) fn js_string(x: &JsValue) -> String;

    /// `Array.from(x)`: the elements of an array.
    #[wasm_bindgen(js_namespace = Array, js_name = from)]
    pub(super) fn array_from(x: &JsValue) -> Vec<JsValue>;
}
