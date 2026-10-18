//! The repeated forms of `map` on the core: `map(repeat=n)`, the absorbing Markov chain (`repeat=Infinity`,
//! `mean_time_to_absorb`) and `map_and_time` (map_tools/function.py, markov_chain.py).
//!
//! As the Python facade does them (crates/icepool-py/src/map_tools.rs): the TypeScript keeps the argument juggling
//! (`TransitionCache.__init__`: `transition_and_star`, so the star inference and the mapping lookup stay its own;
//! `Die([initial_state])`; the checks on `repeat`; `NotImplementedError` for `mean_time_to_absorb` with `Restart`)
//! and makes the transition object (js/src/functions.ts `transition`). Here the core's `TransitionCache` calls it
//! lazily, where Python's would: once per state and joint outcome of the extra arguments (given by index; the
//! TypeScript holds the outcomes, the core their joint quantities). New states are discovered incrementally, as
//! the transition function returns them.

use icepool_core::map_tools::{self, Next, Target, TransitionCache};
use icepool_core::{Error, Quantity, Value};
use wasm_bindgen::convert::TryFromJsValue;
use wasm_bindgen::prelude::*;

use super::convert::{value_from_js, value_to_js};
use super::die::WasmDie;
use super::error::{to_js as to_js_error, JsResult, OrThrow};
use super::host::Transition;

/// A JS exception (thrown by the transition function, or a core error made one) passing through the core.
struct HostErr(JsValue);

impl From<Error> for HostErr {
    fn from(e: Error) -> Self {
        HostErr(to_js_error(e))
    }
}

/// Where a transition goes, as `[kind, payload]`: `"outcome"` and a value, `"die"` and a die the core takes over
/// (a fresh `WasmDie`: taking it empties the JS object), `"reroll"` for `Reroll` or `Restart`.
fn target(kind: &JsValue, payload: JsValue) -> Result<Target, HostErr> {
    match kind.as_string().as_deref() {
        Some("reroll") => Ok(Target::Reroll),
        Some("die") => match WasmDie::try_from_js_value(payload) {
            Ok(die) => Ok(Target::Die(die.die.clone())),
            Err(_) => Err(Error::WrongType("a transition's die is not a WasmDie".into()).into()),
        },
        // Unhashable: `curr_state in self._self_loop_cache` raises TypeError, and so does this.
        _ => Ok(Target::Outcome(value_from_js(&payload)?)),
    }
}

/// What the transition function returned: `[true, "reroll" | "restart"]`, `[true, "break"]` (`Break()`, the current
/// state), `[true, "break" | "go", kind, payload]` with a target, or `[false, exception]` for what it threw.
fn next(parts: Vec<JsValue>) -> Result<Next, HostErr> {
    let mut parts = parts.into_iter();
    let mut part = || parts.next().unwrap_or(JsValue::UNDEFINED);
    let (ok, what) = (part(), part());
    if ok.as_bool() != Some(true) {
        return Err(HostErr(what));
    }
    let (kind, payload) = (part(), part());
    Ok(match what.as_string().as_deref() {
        Some("reroll") => Next::Reroll,
        Some("restart") => Next::Restart,
        Some("break") if kind.is_undefined() => Next::Break(None),
        Some("break") => Next::Break(Some(target(&kind, payload)?)),
        _ => Next::Go(target(&kind, payload)?),
    })
}

/// Runs `body` with a `TransitionCache` over `transition`, whose extra arguments' joint outcomes have the
/// quantities `extra_quantities`.
fn with_cache<R>(
    transition: &Transition,
    extra_quantities: Vec<JsValue>,
    body: impl FnOnce(
        &mut TransitionCache<&mut dyn FnMut(&Value, usize) -> Result<Next, HostErr>>,
    ) -> Result<R, HostErr>,
) -> JsResult<R> {
    let quantities = extra_quantities
        .iter()
        .map(|q| match value_from_js(q)? {
            Value::Int(i) => Quantity::try_from(i).map_err(|_| Error::WrongType("a negative quantity".into())),
            _ => Err(Error::WrongType("a quantity must be an int".into())),
        })
        .collect::<icepool_core::Result<Vec<_>>>()
        .or_throw()?;
    let mut call = |state: &Value, i: usize| -> Result<Next, HostErr> {
        next(transition.step(value_to_js(state)?, i as u32))
    };
    let call: &mut dyn FnMut(&Value, usize) -> Result<Next, HostErr> = &mut call;
    body(&mut TransitionCache::new(call, quantities)).map_err(|e| e.0)
}

/// The `else` branch of `map(repeat=repeat)` (a `repeat` that is not infinite, negative or zero), from
/// `initial_die = Die([first_arg])`; `repeat` is `undefined` where `range(repeat)` raises (a non-int).
#[wasm_bindgen(js_name = mapRepeat)]
pub fn map_repeat(
    transition: &Transition,
    extra_quantities: Vec<JsValue>,
    initial_die: &WasmDie,
    repeat: Option<u32>,
) -> JsResult<WasmDie> {
    let die = with_cache(transition, extra_quantities, |cache| {
        map_tools::map_repeat(cache, &initial_die.die, repeat.map(u64::from))
    })?;
    Ok(WasmDie::new(die))
}

/// `map_and_time(...)` from `initial_die = Die([initial_state])`: the die of `(state, time)`; `repeat` as for
/// `mapRepeat` (a negative one is 0: `range` is then empty).
#[wasm_bindgen(js_name = mapAndTime)]
pub fn map_and_time(
    transition: &Transition,
    extra_quantities: Vec<JsValue>,
    initial_die: &WasmDie,
    repeat: Option<u32>,
) -> JsResult<WasmDie> {
    let die = with_cache(transition, extra_quantities, |cache| {
        map_tools::map_and_time(cache, &initial_die.die, repeat.map(u64::from))
    })?;
    Ok(WasmDie::new(die))
}

/// `absorbing_markov_chain_impl(transition_cache, initial_state)` once the TypeScript has made `initial_die =
/// Die([initial_state])`: `[absorption die, mean absorption time]`, the time a `Fraction`, or `null` if a `Restart`
/// was possible.
#[wasm_bindgen(js_name = absorbingMarkovChain)]
pub fn absorbing_markov_chain(
    transition: &Transition,
    extra_quantities: Vec<JsValue>,
    initial_die: &WasmDie,
) -> JsResult<Vec<JsValue>> {
    let (die, time) = with_cache(transition, extra_quantities, |cache| {
        map_tools::absorbing_markov_chain(cache, &initial_die.die)
    })?;
    let time = match time {
        Some(t) => value_to_js(&Value::Fraction(t)).or_throw()?,
        None => JsValue::NULL,
    };
    Ok(vec![WasmDie::new(die).into(), time])
}
