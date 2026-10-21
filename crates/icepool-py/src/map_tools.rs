//! The repeated forms of `map` on the core: `map(repeat=n)`, the absorbing Markov chain (`repeat='inf'`,
//! `mean_time_to_absorb`) and `map_and_time` (map_tools/function.py, markov_chain.py).
//!
//! The facade keeps the argument juggling: `TransitionCache.__init__` (`transition_and_star`, so `infer_star` and
//! the mapping lambda stay Python's), expanding multiset expressions, `Die([initial_state])`, the checks on
//! `repeat`, and `NotImplementedError` for `mean_time_to_absorb` with `Restart`. Here the core's
//! `TransitionCache` calls the Python callable as a host callback, lazily, where Python's would: once per state
//! and joint outcome of the extra arguments (each given as its `arg_items`, objects untouched), the state
//! unpacked if `star`, `**kwargs` passed as they are. New states are discovered incrementally, as the callable
//! returns them.

use icepool_core::value::{self, iter_cartesian_product, TransitionCache};
use icepool_core::{Mapped, Target};
use icepool_core::{Quantity, Value};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyInt, PyTuple, PyType};

use crate::again::HostErr;
use crate::convert::{from_py, to_py};
use crate::map::items;
use crate::RsDie;

/// A transition function as the core's `TransitionCache` calls it: `transition(state, i)`, `i` the joint
/// outcome of the extra arguments.
type TransitionFn<'a> = dyn FnMut(&Value, usize) -> Result<Mapped<Value>, HostErr> + 'a;

static BREAK: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// What the callable returned, as `TransitionCache` reads it.
fn next(py: Python<'_>, result: &Bound<'_, PyAny>) -> PyResult<Mapped<Value>> {
    let icepool = py.import("icepool")?;
    let (reroll, restart) = (icepool.getattr("Reroll")?, icepool.getattr("Restart")?);
    let die_type = icepool.getattr("Die")?;
    let target = |x: &Bound<'_, PyAny>| -> PyResult<Target<Value>> {
        Ok(if x.is(&reroll) {
            Target::Reroll
        } else if x.is(&restart) {
            Target::Restart
        } else if x.is_instance(&die_type)? {
            let rs = x.call_method0("_rust_die")?;
            Target::Die(rs.cast::<RsDie>()?.get().die.clone())
        } else {
            // Unhashable: `curr_state in self._self_loop_cache` raises TypeError, and so does this.
            Target::Outcome(from_py(x)?)
        })
    };
    if result.is(&reroll) {
        return Ok(Mapped::Reroll);
    }
    if result.is(&restart) {
        return Ok(Mapped::Restart);
    }
    if result.is_instance(BREAK.import(py, "icepool.map_tools.common", "Break")?)? {
        let outcome = result.getattr("outcome")?;
        return Ok(Mapped::Break(if outcome.is_none() {
            None
        } else {
            Some(target(&outcome)?)
        }));
    }
    Ok(target(result)?.into())
}

/// Runs `body` with a `TransitionCache` over `transition` (`TransitionCache._transition`, `._star`, the
/// `arg_items` of `._extra_args`, `._kwargs`).
fn with_cache<'py, R>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    extra_args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
    body: impl FnOnce(
        &mut TransitionCache<Value, &mut TransitionFn<'_>>,
    ) -> Result<R, HostErr>,
) -> PyResult<R> {
    let parts = extra_args.iter().map(items).collect::<PyResult<Vec<_>>>()?;
    let joint: Vec<(Vec<Bound<'py, PyAny>>, Quantity)> = iter_cartesian_product(&parts).collect();
    let quantities = joint.iter().map(|(_, q)| q.clone()).collect();
    let mut call = |state: &Value, i: usize| -> Result<Mapped<Value>, HostErr> {
        let state = to_py(py, state)?;
        // `self._transition(*curr_state, *extra_outcomes, **self._kwargs)` or `(curr_state, ...)`.
        let mut args: Vec<Bound<'py, PyAny>> = if star {
            state.try_iter()?.collect::<PyResult<_>>()?
        } else {
            vec![state]
        };
        args.extend(joint[i].0.iter().cloned());
        let result = transition.call(PyTuple::new(py, args)?, kwargs)?;
        Ok(next(py, &result)?)
    };
    let call: &mut TransitionFn<'_> = &mut call;
    body(&mut TransitionCache::new(call, quantities)).map_err(|e| e.0)
}

/// `range(repeat)`'s view of `repeat`: `None` where it raises TypeError (no `__index__`), a negative count is no
/// steps. A count beyond `u64` would run until the process converges either way.
fn range_count(repeat: &Bound<'_, PyAny>) -> PyResult<Option<u64>> {
    let index = if repeat.is_instance_of::<PyInt>() {
        repeat.clone()
    } else if repeat.hasattr("__index__")? {
        repeat.call_method0("__index__")?
    } else {
        return Ok(None);
    };
    let i = crate::convert::int_from_py(&index)?;
    Ok(Some(u64::try_from(&i).unwrap_or(
        if i < dashu_int::IBig::ZERO {
            0
        } else {
            u64::MAX
        },
    )))
}

/// The `else` branch of `map(repeat=repeat)` (a `repeat` that is not `'inf'`, negative or zero), from
/// `initial_die = Die([first_arg])`.
#[pyfunction]
#[pyo3(signature = (transition, star, extra_args, kwargs, initial_die, repeat))]
pub fn map_repeat<'py>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    extra_args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
    initial_die: &Bound<'py, RsDie>,
    repeat: &Bound<'py, PyAny>,
) -> PyResult<RsDie> {
    let repeat = range_count(repeat)?;
    let die = with_cache(py, transition, star, extra_args, kwargs, |cache| {
        value::map_repeat(cache, &initial_die.get().die, repeat)
    })?;
    RsDie::wrap(py, Ok(die))
}

/// `map_and_time(...)` from `initial_die = Die([initial_state])`: the die of `(state, time)`.
#[pyfunction]
#[pyo3(signature = (transition, star, extra_args, kwargs, initial_die, repeat))]
pub fn map_and_time<'py>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    extra_args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
    initial_die: &Bound<'py, RsDie>,
    repeat: &Bound<'py, PyAny>,
) -> PyResult<RsDie> {
    let repeat = range_count(repeat)?;
    let die = with_cache(py, transition, star, extra_args, kwargs, |cache| {
        value::map_and_time(cache, &initial_die.get().die, repeat)
    })?;
    RsDie::wrap(py, Ok(die))
}

/// `absorbing_markov_chain_impl(transition_cache, initial_state)` once the facade has made `initial_die =
/// Die([initial_state])`: the absorption die and the mean absorption time (a `Fraction`, or `None` if a `Restart`
/// was possible).
#[pyfunction]
#[pyo3(signature = (transition, star, extra_args, kwargs, initial_die))]
pub fn absorbing_markov_chain<'py>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    extra_args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
    initial_die: &Bound<'py, RsDie>,
) -> PyResult<(RsDie, Bound<'py, PyAny>)> {
    let (die, time) = with_cache(py, transition, star, extra_args, kwargs, |cache| {
        value::absorbing_markov_chain(cache, &initial_die.get().die)
    })?;
    let time = match time {
        Some(t) => to_py(py, &Value::Fraction(t))?,
        None => py.None().into_bound(py),
    };
    Ok((RsDie::wrap(py, Ok(die))?, time))
}
