//! map_tools/function.py `map_to_pool` and the arithmetic of population/die.py `explode_to_pool` /
//! `reroll_to_pool` on the core.
//!
//! The facade keeps the argument juggling (`transition_and_star`, `_select_outcomes`, `split`, the early
//! `self.pool(rolls)` returns, `depth == 'inf'`), turning each result into a `Pool` (`Pool.__new__`, the core's)
//! and the `MultisetMixture` (a facade expression, whose evaluations run on the core). Here: `map_to_pool`'s
//! Cartesian product with the calls to the Python callable (a host callback per joint outcome, in
//! `itertools.product` order) and the `REROLL_TYPES` check, and the core's count / composition dice (with `n @
//! die` as the facade's `Die.__rmatmul__` computes it).

use icepool_core::expand::iter_cartesian_product;
use icepool_core::{to_pool, Die as CoreDie, DieArg, Quantity, Value};
use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::again::HostErr;
use crate::convert::{from_py, quantity_to_py};
use crate::map::items;
use crate::{matmul_core, quantity, RsDie};

/// `map_to_pool`'s loop: `transition` called with every joint outcome of `args` (each given as its items; the
/// first is unpacked if `star`), returning the `(result, quantity)` pairs whose result is not in
/// `icepool.REROLL_TYPES`.
#[pyfunction]
#[pyo3(signature = (transition, star, args, kwargs=None))]
pub fn map_to_pool<'py>(
    py: Python<'py>,
    transition: &Bound<'py, PyAny>,
    star: bool,
    args: Vec<Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyList>> {
    let parts = args.iter().map(items).collect::<PyResult<Vec<_>>>()?;
    let reroll_types = py.import("icepool")?.getattr("REROLL_TYPES")?;
    let results = PyList::empty(py);
    for (outcomes, q) in iter_cartesian_product(&parts) {
        // `transition_function(*outcomes[0], *outcomes[1:], **kwargs)` or `transition_function(*outcomes, ...)`.
        let call_args: Vec<Bound<'py, PyAny>> = if star {
            let (first, extra) = outcomes
                .split_first()
                .ok_or_else(|| PyIndexError::new_err("tuple index out of range"))?;
            let mut a: Vec<_> = first.try_iter()?.collect::<PyResult<_>>()?;
            a.extend(extra.iter().cloned());
            a
        } else {
            outcomes
        };
        let pool = transition.call(PyTuple::new(py, call_args)?, kwargs)?;
        // `pool in icepool.REROLL_TYPES`: Python's `==` (identity first).
        if reroll_types.contains(&pool)? {
            continue;
        }
        results.append((pool, quantity_to_py(py, &q)?))?;
    }
    Ok(results)
}

/// `n @ die` with a Python `n`: `Die([n]) @ die` (`Die.__rmatmul__`).
fn rmatmul(py: Python<'_>) -> impl Fn(&Value, &CoreDie<Value>) -> Result<CoreDie<Value>, HostErr> + '_ {
    move |n, die| {
        let n = CoreDie::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
        Ok(matmul_core(py, n, die)?)
    }
}

/// `explode_to_pool`'s `count_die` (`rolls @ Die(single_data)`) from the denominators of the die and its two
/// halves. A non-int `depth` raises TypeError where `range(depth + 1)` would.
#[pyfunction]
pub fn explode_counts(
    py: Python<'_>,
    die_den: &Bound<'_, PyAny>,
    explode_den: &Bound<'_, PyAny>,
    not_explode_den: &Bound<'_, PyAny>,
    rolls: &Bound<'_, PyAny>,
    depth: i64,
) -> PyResult<RsDie> {
    let die = to_pool::explode_counts(
        &quantity(die_den)?,
        &quantity(explode_den)?,
        &quantity(not_explode_den)?,
        &from_py(rolls)?,
        depth,
        rmatmul(py),
    )
    .map_err(|e| e.0)?;
    RsDie::wrap(py, Ok(die))
}

/// `reroll_to_pool`'s `pool_composition` (already multiplied to its denominator) from the denominators of the
/// die and of its rerollable half; `depth` is an int here (`'inf'` was replaced by `max_rerolls`).
#[pyfunction]
pub fn reroll_composition(
    py: Python<'_>,
    die_den: &Bound<'_, PyAny>,
    rerollable_den: &Bound<'_, PyAny>,
    rolls: &Bound<'_, PyAny>,
    max_rerolls: &Bound<'_, PyAny>,
    depth: &Bound<'_, PyAny>,
) -> PyResult<RsDie> {
    let die = to_pool::reroll_composition(
        &quantity(die_den)?,
        &quantity(rerollable_den)?,
        &from_py(rolls)?,
        &from_py(max_rerolls)?,
        &from_py(depth)?,
        rmatmul(py),
    )
    .map_err(|e| e.0)?;
    RsDie::wrap(py, Ok(die))
}
