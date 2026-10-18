//! population/again.py on the core, for the TypeScript `Again` (js/src/again.ts): `evaluate_agains_using_depth`,
//! and count mode's step outcomes.
//!
//! As the Python facade does it (crates/icepool-py/src/again.rs): the TypeScript keeps `AgainExpression` itself (its
//! operator methods, `isAdditive`, `==`) and the argument juggling of the two functions (the checks on the depth and
//! count, the default `againEnd`, `compute_not_again_die`, count mode's dice). Here each `AgainExpression` among the
//! itemized outcomes becomes the core's `Again` tree, read through `__icepool_wasm_host.againParts`, over JS
//! objects: its functions (Python's `operator.add` & co., by name) are applied by JS with Python's operator dispatch
//! (`againApply`), and `Die(objects, times)` is the TypeScript constructor (`againDie`), whose mixing is the core's.
//! The core counts Python's frames as it goes, so a deep expression raises `RecursionError` where Python does.

use std::sync::Arc;

use icepool_core::again::{self, AgainExpression, AgainArg, StepOutcome};
use icepool_core::value::recursion::level;
use icepool_core::Error;
use wasm_bindgen::prelude::*;

use super::error::{to_js as to_js_error, JsResult};
use super::host;

/// An argument of an expression: the function's name (a JS string) and JS objects.
type Arg = AgainArg<JsValue, JsValue>;

/// A JS exception (thrown by a callback, or a core error made one) passing through the core's generic functions.
struct HostErr(JsValue);

impl From<Error> for HostErr {
    fn from(e: Error) -> Self {
        HostErr(to_js_error(e))
    }
}

/// A callback's `[true, result]` or `[false, exception]`.
fn envelope(parts: Vec<JsValue>) -> Result<JsValue, HostErr> {
    let result = parts.get(1).cloned().unwrap_or(JsValue::UNDEFINED);
    match parts.first().and_then(JsValue::as_bool) {
        Some(true) => Ok(result),
        _ => Err(HostErr(result)),
    }
}

/// An itemized outcome: an `AgainExpression` becomes the core tree, anything else is passed as-is. Expressions
/// nest, so a deep one raises `RecursionError` (`recursion.rs`) rather than overflowing the stack.
fn arg(x: JsValue) -> Result<Arg, HostErr> {
    let parts = host::again_parts(&x);
    let [function, args, is_additive] = parts.as_slice() else {
        return Ok(AgainArg::Other(x));
    };
    let _level = level()?;
    let is_additive = is_additive.as_bool().unwrap_or(false);
    // No function: the expression represents the again roll directly.
    if function.is_null() {
        return Ok(AgainArg::Again(Arc::new(AgainExpression::Again { is_additive })));
    }
    let args = host::array_from(args).into_iter().map(arg).collect::<Result<_, _>>()?;
    Ok(AgainArg::Again(Arc::new(AgainExpression::Apply { function: function.clone(), args, is_additive })))
}

fn args(outcomes: Vec<JsValue>) -> Result<Vec<Arg>, HostErr> {
    outcomes.into_iter().map(arg).collect()
}

/// `function(*args)`, args already evaluated.
fn apply(function: &JsValue, args: Vec<JsValue>) -> Result<JsValue, HostErr> {
    envelope(host::again_apply(function, args))
}

/// `evaluate_agains_using_depth(outcomes, times, again_depth, again_end)` after the TypeScript has checked
/// `again_depth`, replaced a `null` `again_end` with the zero outcome and rejected `Restart`: `end_is_reroll` for
/// `again_end=Reroll` (every expression at the innermost level becomes `reroll`, the JS `Reroll`). Returns the
/// TypeScript die.
#[wasm_bindgen(js_name = evaluateAgainsUsingDepth)]
pub fn evaluate_agains_using_depth(
    outcomes: Vec<JsValue>,
    times: Vec<JsValue>,
    again_depth: u32,
    again_end: JsValue,
    end_is_reroll: bool,
    reroll: JsValue,
) -> JsResult<JsValue> {
    let run = || {
        let outcomes = args(outcomes)?;
        let times = JsValue::from(times);
        again::evaluate_agains_using_depth(
            &outcomes,
            u64::from(again_depth),
            (!end_is_reroll).then_some(&again_end),
            &reroll,
            |objects| envelope(host::again_die(objects, &times)),
            apply,
        )
    };
    run().map_err(|e| e.0)
}

/// `[make_step_outcome(outcome) for outcome in outcomes]` of `evaluate_agains_using_count`, before tupleizing:
/// `[zero, 1, -1]` for an outcome that is not an expression, `[outcome._evaluate(zero), 0, outcome._again_count()
/// - 1]` for an additive one (a `ValueError` for any other). `Die(...)` then tupleizes these arrays as Python's
/// `make_step_outcome` does.
#[wasm_bindgen(js_name = againStepOutcomes)]
pub fn again_step_outcomes(outcomes: Vec<JsValue>, zero: JsValue) -> JsResult<Vec<JsValue>> {
    let int = |i: i32| JsValue::from_f64(f64::from(i));
    let run = || {
        let outcomes = args(outcomes)?;
        again::step_outcomes(&outcomes, &zero, &int(1), &int(0), apply, |step| {
            Ok(JsValue::from(match step {
                StepOutcome::NotAgain => vec![zero.clone(), int(1), int(-1)],
                StepOutcome::Again { flat, count } => {
                    vec![flat, int(0), apply(&JsValue::from_str("sub"), vec![count, int(1)])?]
                }
            }))
        })
    };
    run().map_err(|e| e.0)
}
