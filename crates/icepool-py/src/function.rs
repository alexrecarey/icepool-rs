//! function.py's free functions on the core (`icepool_core::function`): the facade keeps each one's argument
//! juggling (`_iter_outcomes`, `Fraction(n)` and `limit_denominator`, `implicit_convert_to_die`, the
//! `multiply_quantities` calls) and calls in here for the work.

use dashu_int::UBig;
use icepool_core::value::builtins::py_min_max;
use icepool_core::value;
use icepool_core::{Quantity, Value};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

use crate::convert::{from_py, int_from_py, to_py, to_py_err};
use crate::RsDie;

fn values(xs: &[Bound<'_, PyAny>]) -> PyResult<Vec<Value>> {
    xs.iter().map(from_py).collect()
}

/// math.py `weighted_lcm(denominators, weights)`: the scale factor for each die (`harmonize_denominators`), given
/// what each of Python's two `zip(denominators, weights)` passes takes from the weights.
#[pyfunction]
pub fn weighted_lcm<'py>(
    py: Python<'py>,
    denominators: Vec<Bound<'py, PyAny>>,
    weights: Vec<Bound<'py, PyAny>>,
    weights_again: Vec<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let denominators = denominators
        .iter()
        .map(|d| Ok(UBig::try_from(int_from_py(d)?).expect("a denominator is not negative")))
        .collect::<PyResult<Vec<Quantity>>>()?;
    let scales =
        value::weighted_lcm(&denominators, &values(&weights)?, &values(&weights_again)?)
            .map_err(to_py_err)?;
    PyList::new(
        py,
        scales
            .iter()
            .map(|v| to_py(py, v))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// `pointwise_max` (`max`) / `pointwise_min` of dice whose denominators are already harmonized.
#[pyfunction]
pub fn pointwise(py: Python<'_>, dice: Vec<Bound<'_, RsDie>>, max: bool) -> PyResult<RsDie> {
    let dice: Vec<_> = dice.iter().map(|d| d.get().die.clone()).collect();
    RsDie::wrap(py, value::pointwise(&dice, max))
}

/// The `dict` items of `coin(n, d)` once a non-int numerator has become a Fraction's numerator and denominator.
#[pyfunction]
pub fn coin_data<'py>(
    py: Python<'py>,
    n: &Bound<'py, PyAny>,
    d: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let data = value::coin_data(&from_py(n)?, &from_py(d)?).map_err(to_py_err)?;
    PyList::new(
        py,
        data.iter()
            .map(|(k, q)| PyTuple::new(py, [to_py(py, k)?, to_py(py, q)?]))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// Python's `min(items)` (`max` false) or `max(items)` (`min_outcome` / `max_outcome`).
#[pyfunction]
pub fn min_max<'py>(
    py: Python<'py>,
    items: Vec<Bound<'py, PyAny>>,
    max: bool,
) -> PyResult<Bound<'py, PyAny>> {
    to_py(
        py,
        &py_min_max(values(&items)?, max).map_err(to_py_err)?,
    )
}

/// `consecutive(*args)` given what each of its two chains over the arguments yields.
#[pyfunction]
pub fn consecutive<'py>(
    py: Python<'py>,
    first: Vec<Bound<'py, PyAny>>,
    second: Vec<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    to_py(
        py,
        &value::consecutive(values(&first)?, values(&second)?).map_err(to_py_err)?,
    )
}

/// `sorted_union(*args)` given the elements of each argument.
#[pyfunction]
pub fn sorted_union<'py>(
    py: Python<'py>,
    args: Vec<Vec<Bound<'py, PyAny>>>,
) -> PyResult<Bound<'py, PyTuple>> {
    let sets = args
        .iter()
        .map(|a| values(a))
        .collect::<PyResult<Vec<_>>>()?;
    let union = value::sorted_union(sets.iter().map(|s| s.iter())).map_err(to_py_err)?;
    PyTuple::new(
        py,
        union
            .iter()
            .map(|v| to_py(py, v))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// `one_hot(sides)`.
#[pyfunction]
pub fn one_hot(py: Python<'_>, sides: &Bound<'_, PyAny>) -> PyResult<RsDie> {
    RsDie::wrap(py, value::one_hot(&from_py(sides)?))
}
