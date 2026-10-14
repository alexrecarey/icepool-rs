//! `map` without `repeat` (map_tools/core_impl.py `map_simple`) and `tupleize` (expand.py `cartesian_product`) on
//! the core.
//!
//! The facade keeps the argument juggling (`transition_and_star`, `infer_star`, expanding multiset expressions,
//! `_items_for_cartesian_product`) and the final `icepool.Die(...)`, whose mixing (`weighted_lcm`, `Reroll`,
//! tupleized tuples) runs in the core like every other die construction. Here: the Cartesian product, the calls
//! to the Python callable (a host callback per joint outcome, in `itertools.product` order), and unwrapping
//! `Break`.

use icepool_core::expand::{iter_cartesian_product, tupleize_dice};
use icepool_core::{Quantity, Value};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyList, PyTuple, PyType};

use crate::convert::{from_py, quantity_to_py};
use crate::{quantity, RsDie};

static BREAK: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// One argument's `(outcome, quantity)` items, as `iter_cartesian_product`'s `arg_items` gives them.
fn items<'py>(arg: &Bound<'py, PyAny>) -> PyResult<Vec<(Bound<'py, PyAny>, Quantity)>> {
    arg.try_iter()?
        .map(|item| {
            let (o, q): (Bound<'py, PyAny>, Bound<'py, PyAny>) = item?.extract()?;
            Ok((o, quantity(&q)?))
        })
        .collect()
}

/// `map_simple`'s loop: `transition` called with every joint outcome of `args` (each given as its items; the
/// first is unpacked if `star`), returning `(final_states, final_quantities)` for `icepool.Die`.
#[pyfunction]
#[pyo3(signature = (transition, star, args, kwargs=None))]
pub fn map_simple<'py>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyList>)> {
    let parts = args.iter().map(items).collect::<PyResult<Vec<_>>>()?;
    let break_type = BREAK.import(py, "icepool.map_tools.common", "Break")?;
    let final_states = PyList::empty(py);
    let final_quantities = PyList::empty(py);
    for (outcomes, q) in iter_cartesian_product(&parts) {
        let (first, extra) = outcomes
            .split_first()
            .expect("map has at least one argument");
        // `transition(*first_outcome, *extra_outcomes, **kwargs)` or `transition(first_outcome, ...)`.
        let mut call_args: Vec<Bound<'py, PyAny>> = if star {
            first.try_iter()?.collect::<PyResult<_>>()?
        } else {
            vec![first.clone()]
        };
        call_args.extend(extra.iter().cloned());
        let mut state = transition.call(PyTuple::new(py, call_args)?, kwargs)?;
        if state.is_instance(break_type)? {
            // `Break()` stands for the first outcome (before unpacking), `Break(x)` for `x`.
            let outcome = state.getattr("outcome")?;
            state = if outcome.is_none() {
                first.clone()
            } else {
                outcome
            };
        }
        final_states.append(state)?;
        final_quantities.append(quantity_to_py(py, &q)?)?;
    }
    Ok((final_states, final_quantities))
}

/// `cartesian_product(*args, outcome_type=tuple)` once the facade has found the population type to be `Die`:
/// each arg is `(1, rs_die)` or `(0, outcome)` (an argument that is not a population, used as-is).
#[pyfunction]
pub fn tupleize(py: Python<'_>, args: Vec<(u8, Bound<'_, PyAny>)>) -> PyResult<RsDie> {
    let parts = args
        .iter()
        .map(|(kind, x)| -> PyResult<Vec<(Value, Quantity)>> {
            Ok(if *kind == 1 {
                x.cast::<RsDie>()?.get().die.items().to_vec()
            } else {
                vec![(from_py(x)?, Quantity::ONE)]
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    RsDie::wrap(py, tupleize_dice(&parts))
}
