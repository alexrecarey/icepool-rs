//! `MultisetEvaluator` subclasses written in JS (evaluator/multiset_evaluator.py), run by the core's driver as the
//! Python facade runs Python ones (crates/icepool-py/src/evaluate.rs `HostEvaluator`): the driver pops the sources
//! and memoises the rooms; `initial_state` / `next_state` / `extra_outcomes` / `final_outcome` are JS callbacks.
//!
//! The TypeScript wraps the evaluator for the evaluation (js/src/evaluator.ts `HostEvaluator`): it adds
//! `evaluate()`'s keyword arguments, the defaults of the methods a subclass leaves out, and the answer envelope
//! (`host::JsEvaluator`). States are values (`convert`), hashed and compared the Python way, so equal states merge
//! as in Python's `defaultdict`; a plain JS object state is a [`StateKey`], keyed by its canonical JSON (the
//! TypeScript makes it). Final outcomes stay JS values: the TypeScript makes the result die from them with
//! `new Die(final_outcomes, final_weights)`, as Python's `Die(...)` (a final outcome may be a die).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use icepool_core::evaluator::{Count, Final, MultisetEvaluator, Order};
use icepool_core::expression::count_value;
use icepool_core::host::{HostObject, HostRef};
use icepool_core::value::{BinOp, UnOp};
use icepool_core::{Error, Int, Value};
use wasm_bindgen::prelude::*;

use super::convert::{quantity_to_js, value_from_js, value_to_js, values_to_js};
use super::error::{JsResult, OrThrow};
use super::evaluate::{callback_error, evaluations, Evaluations, ExpressionBuilder};
use super::host::{self, JsEvaluator};

/// An evaluator state that was a plain JS object, keyed by its canonical JSON (sorted keys, made by the
/// TypeScript): equal keys are equal states. It has no order or arithmetic, like a Python dict, and its truth value
/// is a dict's. It goes back to JS as a fresh object.
#[derive(Debug)]
pub(super) struct StateKey(Arc<str>);

impl StateKey {
    pub(super) fn value(key: &str) -> Value {
        Value::Host(HostRef::new(StateKey(key.into())))
    }

    pub(super) fn of(v: &Value) -> Option<&StateKey> {
        match v {
            Value::Host(h) => h.as_any().downcast_ref::<StateKey>(),
            _ => None,
        }
    }

    pub(super) fn key(&self) -> &str {
        &self.0
    }
}

fn no_operators() -> Error {
    Error::WrongType("JS object states have no operators".into())
}

impl HostObject for StateKey {
    fn hash_value(&self) -> i64 {
        // Any fixed hash of the key: equal keys hash alike, and no other value equals a key.
        let mut h = std::hash::DefaultHasher::new();
        self.0.hash(&mut h);
        h.finish() as i64
    }
    fn eq(&self, a: &Value, b: &Value) -> bool {
        matches!((StateKey::of(a), StateKey::of(b)), (Some(x), Some(y)) if x.0 == y.0)
    }
    fn compare(&self, _: BinOp, _: &Value, _: &Value) -> icepool_core::Result<bool> {
        Err(Error::WrongType("JS object states have no order".into()))
    }
    fn binary(&self, _: BinOp, _: &Value, _: &Value) -> icepool_core::Result<Value> {
        Err(no_operators())
    }
    fn unary(&self, _: UnOp) -> icepool_core::Result<Value> {
        Err(no_operators())
    }
    fn truthy(&self) -> icepool_core::Result<bool> {
        Ok(&*self.0 != "{}")
    }
    fn repr(&self) -> String {
        self.0.to_string()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What a JS evaluator's method answered (see `host::JsEvaluator`): `Some(value)`, or `None` for a reroll type.
/// `UnsupportedOrder` is the core's signal to retry in the other order (Python's `Dungeon.evaluate` catches it
/// around `initial_state`, `next_state` and `final_outcome`; the TypeScript never answers so for
/// `extra_outcomes`, whose `UnsupportedOrder` propagates as itself); anything else it threw is rethrown as itself.
fn answer(parts: Vec<JsValue>) -> icepool_core::Result<Option<JsValue>> {
    let mut parts = parts.into_iter();
    let tag = parts.next().and_then(|t| t.as_string()).unwrap_or_default();
    let payload = parts.next().unwrap_or(JsValue::UNDEFINED);
    match tag.as_str() {
        "ok" => Ok(Some(payload)),
        "reroll" => Ok(None),
        "unsupported" => Err(Error::UnsupportedOrder(payload.as_string().unwrap_or_default())),
        _ => Err(callback_error()),
    }
}

/// A count or size as the JS methods get it: an int, or an array of ints for a tuple count.
fn count_js(c: &Count) -> icepool_core::Result<JsValue> {
    value_to_js(&count_value(c))
}

fn sizes_js(sizes: &[Option<Count>]) -> icepool_core::Result<Vec<JsValue>> {
    sizes.iter().map(|s| s.as_ref().map_or(Ok(JsValue::NULL), count_js)).collect()
}

fn int_counts(counts: &[Int]) -> Vec<Count> {
    counts.iter().cloned().map(Count::Int).collect()
}

fn int_sizes(sizes: &[Option<Int>]) -> Vec<Option<Count>> {
    sizes.iter().map(|s| s.clone().map(Count::Int)).collect()
}

fn order_js(order: Order) -> i32 {
    order.value() as i32
}

/// The driver's view of a JS evaluator.
pub(super) struct HostEvaluator(pub(super) JsEvaluator);

impl MultisetEvaluator<Value> for HostEvaluator {
    /// Any value the core can hash (see the top of this file).
    type State = Value;
    /// The final outcome as JS returned it (it may be a die).
    type Final = JsValue;

    fn initial_state(&self, order: Order, outcomes: &[Value], sizes: &[Option<Int>]) -> icepool_core::Result<Value> {
        self.initial_state_counts(order, outcomes, &int_sizes(sizes))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> icepool_core::Result<Option<Value>> {
        self.next_state_counts(state, order, outcome, &int_counts(counts))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Int>],
    ) -> icepool_core::Result<Final<JsValue>> {
        self.final_outcome_counts(final_state, order, outcomes, &int_sizes(sizes))
    }

    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<Value> {
        // `quest.initial_state_main(order, outcomes, *arg_sizes, **kwargs)`
        let state = self.0.initial_state(order_js(order), values_to_js(outcomes)?, sizes_js(sizes)?);
        value_from_js(&answer(state)?.unwrap_or(JsValue::NULL))
    }

    fn next_state_counts(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> icepool_core::Result<Option<Value>> {
        // `self.next_state_main(prev_state_main, eval_order, outcome, *count_tree)`
        let counts = counts.iter().map(count_js).collect::<icepool_core::Result<_>>()?;
        let next = self.0.next_state(value_to_js(state)?, order_js(order), value_to_js(outcome)?, counts);
        // `if state_main not in icepool.REROLL_TYPES`: a reroll drops this branch.
        answer(next)?.map(|next| value_from_js(&next)).transpose()
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> icepool_core::Result<Vec<Value>> {
        let extra = answer(self.0.extra_outcomes(values_to_js(outcomes)?))?.unwrap_or(JsValue::UNDEFINED);
        host::array_from(&extra).iter().map(value_from_js).collect()
    }

    fn final_outcome_counts(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<Final<JsValue>> {
        // `self.final_outcome(state, order, outcomes, *arg_sizes, **kwargs)`
        let outcome =
            self.0.final_outcome(value_to_js(final_state)?, order_js(order), values_to_js(outcomes)?, sizes_js(sizes)?);
        Ok(match answer(outcome)? {
            None => Final::Reroll,
            Some(o) if o.is_null_or_undefined() => Final::None,
            Some(o) => Final::Outcome(o),
        })
    }
}

/// The evaluations as JS: an array with, per way to prepare the inputs, `[final_outcomes, final_weights, weight]`
/// (weights as BigInts), for the TypeScript to make `Die(final_outcomes, final_weights)` of each and mix them by
/// weight (`final_data[result] += weight`, then `Die(final_data)`).
pub(super) fn evaluations_to_js(evaluations: Evaluations<JsValue>) -> JsValue {
    let parts = evaluations
        .into_iter()
        .map(|(finals, weight)| {
            let (outcomes, weights): (Vec<JsValue>, Vec<JsValue>) =
                finals.into_iter().map(|(o, w)| (o, quantity_to_js(&w))).unzip();
            JsValue::from(vec![JsValue::from(outcomes), JsValue::from(weights), quantity_to_js(&weight)])
        })
        .collect::<Vec<_>>();
    JsValue::from(parts)
}

#[wasm_bindgen]
impl ExpressionBuilder {
    /// `evaluator.evaluate(*inputs)` of a JS evaluator over the last `inputs` nodes: its evaluations (see
    /// [`evaluations_to_js`]).
    #[wasm_bindgen(js_name = evaluateHost)]
    pub fn evaluate_host(&mut self, evaluator: JsEvaluator, inputs: usize) -> JsResult<JsValue> {
        let inputs = self.pop(inputs)?;
        evaluations(&HostEvaluator(evaluator), &inputs).map(evaluations_to_js).or_throw()
    }
}
