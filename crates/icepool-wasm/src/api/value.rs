//! Python's operators on plain values (no dice): `ops.add(1, 2.5)`, `ops.lt("a", "b")`, `truth(vector)`, as the
//! core's `Value` does them (value/ops.rs).

use wasm_bindgen::prelude::*;

use super::convert::{value_from_js, value_to_js};
use super::die::{apply, BinaryOp, UnaryOp};
use super::error::{JsResult, OrThrow};
use icepool_core::value::UnOp;

/// `a <op> b` for two plain values.
#[wasm_bindgen(js_name = valueBinary)]
pub fn value_binary(op: BinaryOp, a: JsValue, b: JsValue) -> JsResult<JsValue> {
    let (a, b) = (value_from_js(&a).or_throw()?, value_from_js(&b).or_throw()?);
    value_to_js(&apply(op, &a, &b).or_throw()?).or_throw()
}

/// `<op> a` for a plain value.
#[wasm_bindgen(js_name = valueUnary)]
pub fn value_unary(op: UnaryOp, a: JsValue) -> JsResult<JsValue> {
    let a = value_from_js(&a).or_throw()?;
    value_to_js(&a.unary(UnOp::from(op)).or_throw()?).or_throw()
}

/// `round(a, ndigits)` for a plain value.
#[wasm_bindgen(js_name = valueRound)]
pub fn value_round(a: JsValue, ndigits: JsValue) -> JsResult<JsValue> {
    let (a, ndigits) = (value_from_js(&a).or_throw()?, value_from_js(&ndigits).or_throw()?);
    value_to_js(&a.round(&ndigits).or_throw()?).or_throw()
}

/// Python's `bool(a)` for a plain value (a Vector from a comparison has its truth value; Symbols theirs).
#[wasm_bindgen(js_name = valueTruth)]
pub fn value_truth(a: JsValue) -> JsResult<bool> {
    value_from_js(&a).or_throw()?.truthy().or_throw()
}
