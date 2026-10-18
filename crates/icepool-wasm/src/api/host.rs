//! What the typed exports need from JS: the value classes of the TypeScript API (js/src/values.ts), which the
//! TypeScript side installs on the global `__icepool_wasm_host` (js/src/wasm.ts) before the first call, and two
//! plain JS globals. Declared directly, so wasm-bindgen stays the only binding dependency (no js-sys).

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// A JS object as `[tag, ...parts]`: `["array"]` (a tuple), `["list"]` (a `List`, Python's list),
    /// `["float", number]`, `["fraction", bigint, bigint]`, `["complex", number, number]`, `["vector", items | null,
    /// hasTruth, truth]`, `["symbols", items | null, str]` (items as `[char, count]` pairs), or `["other",
    /// typeName]` for anything else.
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

    /// `String(x)`: a BigInt's decimal digits.
    #[wasm_bindgen(js_name = String)]
    pub(super) fn js_string(x: &JsValue) -> String;

    /// `Array.from(x)`: the elements of an array.
    #[wasm_bindgen(js_namespace = Array, js_name = from)]
    pub(super) fn array_from(x: &JsValue) -> Vec<JsValue>;
}
