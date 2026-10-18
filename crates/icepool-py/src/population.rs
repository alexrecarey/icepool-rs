//! What population/base.py, die.py, deck.py and expand.py had left in Python (S5c): `append` / `remove` /
//! `to_one_hot`, the deck forms of `zero` / `split`, `Deck.map`'s calls, `Die.mean_time_to_sum`, and the
//! Cartesian products of `iter_cartesian_product` / `cartesian_product` for any argument kind.
//!
//! The facade keeps the argument juggling (`_items_for_cartesian_product`, the population type, `star` and
//! mappings in `Deck.map`, `_select_outcomes`) and builds the result object; the work is the core's, and Python
//! callables (`outcome_type`, `Deck.map`'s function, a sequence's `in` / `index`) are called back from here.

use icepool_core::value::{iter_cartesian_product as core_product, PyDie};
use icepool_core::{Deck, Die, Value};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::convert::{from_py, quantity_to_py, to_py, to_py_err};
use crate::deck::RsDeck;
use crate::map::items;
use crate::stats::data;
use crate::RsDie;

/// `Population.append(outcome, quantity)`: the items of the `Counter` Python passes to `self._new_type(...)`.
#[pyfunction]
pub fn population_append<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    outcome: &Bound<'py, PyAny>,
    quantity: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let d = die
        .get()
        .die
        .append(&from_py(outcome)?, &from_py(quantity)?);
    Ok(data(py, d.map(Some))?.expect("append always makes data"))
}

/// `Population.remove(outcome, quantity)` (`None`: all of it), as `population_append`.
#[pyfunction]
#[pyo3(signature = (die, outcome, quantity=None))]
pub fn population_remove<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    outcome: &Bound<'py, PyAny>,
    quantity: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let quantity = quantity.map(from_py).transpose()?;
    let d = die.get().die.remove(&from_py(outcome)?, quantity.as_ref());
    Ok(data(py, d.map(Some))?.expect("remove always makes data"))
}

/// `Population.to_one_hot(outcomes)` of `die` (a deck's cards when `deck`), whose facade outcomes (the objects
/// Python's loop sees) are `own_outcomes`. `outcomes=None` means the population's own; otherwise its `len`,
/// `in` and `index` are Python's. Returns a core die, or a core deck when `deck`.
#[pyfunction]
pub fn to_one_hot<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    own_outcomes: &Bound<'py, PyAny>,
    outcomes: Option<&Bound<'py, PyAny>>,
    deck: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let die = &die.get().die;
    let (len, positions) = match outcomes {
        None => (die.len(), (0..die.len() as i64).map(Some).collect()),
        Some(seq) => {
            // `[False] * len(outcomes)` is made for each outcome, so an empty population never asks.
            let len = if die.is_empty() { 0 } else { seq.len()? };
            let mut positions = Vec::with_capacity(die.len());
            for outcome in own_outcomes.try_iter()? {
                let outcome = outcome?;
                positions.push(if seq.contains(&outcome)? {
                    Some(seq.call_method1("index", (outcome,))?.extract::<i64>()?)
                } else {
                    None
                });
            }
            (len, positions)
        }
    };
    let items = die.one_hot_items(len, &positions).map_err(to_py_err)?;
    if deck {
        let deck = Deck::from_dict(items).map_err(to_py_err)?;
        Ok(Bound::new(py, RsDeck { deck })?.into_any())
    } else {
        Ok(Bound::new(py, RsDie::wrap(py, Die::from_dict(items))?)?.into_any())
    }
}

/// `Population.zero` of a deck.
#[pyfunction]
pub fn deck_zero(deck: &Bound<'_, RsDeck>) -> PyResult<RsDeck> {
    Ok(RsDeck {
        deck: deck.get().deck.zero().map_err(to_py_err)?,
    })
}

/// `Population.split` of a deck, given one flag per outcome (the facade's `_select_outcomes`).
#[pyfunction]
pub fn deck_split(deck: &Bound<'_, RsDeck>, mask: Vec<bool>) -> PyResult<(RsDeck, RsDeck)> {
    let (a, b) = deck.get().deck.split(&mask).map_err(to_py_err)?;
    Ok((RsDeck { deck: a }, RsDeck { deck: b }))
}

/// `[f(outcome) for outcome in self.outcomes()]`: `Deck.map`'s calls of its transition function, whose results
/// the facade passes to `Deck(..., times=self.quantities())`.
#[pyfunction]
pub fn call_per_outcome<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    f: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let results = PyList::empty(py);
    for outcome in die.get().die.outcomes() {
        results.append(f.call1((to_py(py, outcome)?,))?)?;
    }
    Ok(results)
}

/// `Die.mean_time_to_sum(target)` with the die's `_mean_time_to_sum_cache` (a Python list), which the core
/// extends: the entries it computed are appended to `cache`, as Python's loop appends them.
#[pyfunction]
pub fn mean_time_to_sum<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    cache: &Bound<'py, PyList>,
    target: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let mut values = cache
        .iter()
        .map(|v| from_py(&v))
        .collect::<PyResult<Vec<Value>>>()?;
    let known = values.len();
    let result = die
        .get()
        .die
        .mean_time_to_sum(&mut values, &from_py(target)?);
    for v in &values[known..] {
        cache.append(to_py(py, v)?)?;
    }
    to_py(py, &result.map_err(to_py_err)?)
}

/// `iter_cartesian_product(*args)` given each argument's items (`arg_items`): `(outcomes, quantity)` per joint
/// outcome in `itertools.product` order. The outcomes are the facade's own objects.
#[pyfunction]
pub fn iter_cartesian_product<'py>(
    py: Python<'py>,
    args: Vec<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let parts = args.iter().map(items).collect::<PyResult<Vec<_>>>()?;
    let result = PyList::empty(py);
    for (outcomes, q) in core_product(&parts) {
        result.append((PyTuple::new(py, outcomes)?, quantity_to_py(py, &q)?))?;
    }
    Ok(result)
}

/// `cartesian_product`'s `data` once the population type is known: `data[outcome_type(outcomes)] =
/// final_quantity` for each joint outcome (assignment: equal keys keep the first spelling and the last quantity).
/// `outcome_type` is called back unless it is `tuple`.
#[pyfunction]
pub fn cartesian_product_data<'py>(
    py: Python<'py>,
    outcome_type: &Bound<'py, PyAny>,
    args: Vec<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyDict>> {
    let parts = args.iter().map(items).collect::<PyResult<Vec<_>>>()?;
    let is_tuple = outcome_type.is(py.get_type::<PyTuple>().as_any());
    let data = PyDict::new(py);
    for (outcomes, q) in core_product(&parts) {
        let outcomes = PyTuple::new(py, outcomes)?;
        let key = if is_tuple {
            outcomes.into_any()
        } else {
            outcome_type.call1((outcomes,))?
        };
        data.set_item(key, quantity_to_py(py, &q)?)?;
    }
    Ok(data)
}
