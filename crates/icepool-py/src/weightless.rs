//! `WeightlessSource` (generator/weightless.py) on the core: the facade's `WeightlessSource` holds one as `_rs`, so
//! an evaluation over `g.weightless()` runs on the core driver. The facade keeps `WeightlessGenerator`, which only
//! wraps a generator and makes this source from the wrapped one's.

use icepool_core::evaluator::{Count, MultisetSource};
use icepool_core::evaluator::{Order, OrderReason};
use icepool_core::weightless::WeightlessSource;
use icepool_core::Value;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::convert::{count_to_py, counts_to_py, from_py, to_py_err};
use crate::expression::{core_source, source_object};
use crate::pool::{hash_of, outcome_tuple};

/// A `WeightlessSource` of the core, held by the facade's as `_rs`.
#[pyclass(frozen, name = "WeightlessSource", module = "icepool_rs")]
pub struct RsWeightlessSource {
    pub(crate) source: WeightlessSource<Value>,
}

/// `WeightlessSource(wrapped)`, given the wrapped source's `_rs`.
#[pyfunction]
pub fn weightless_source(wrapped: &Bound<'_, PyAny>) -> PyResult<RsWeightlessSource> {
    Ok(RsWeightlessSource {
        source: WeightlessSource::new(core_source(wrapped)?),
    })
}

/// A count or size as Python has it: an `int`, or a tuple of `int`s.
fn count_py<'py>(py: Python<'py>, c: &Count) -> PyResult<Bound<'py, PyAny>> {
    match c {
        Count::Int(c) => count_to_py(py, c),
        Count::Tuple(cs) => counts_to_py(py, cs).map(Bound::into_any),
    }
}

#[pymethods]
impl RsWeightlessSource {
    /// The Python objects it holds, for the cycle collector (`convert::traverse`).
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        crate::convert::traverse(&self.source, &visit)
    }

    /// The wrapped source as an `icepool_rs` object, for the facade to wrap.
    fn wrapped<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        source_object(py, self.source.wrapped())
    }

    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes().map_err(to_py_err)?)
    }

    fn size<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.source
            .size()
            .map_err(to_py_err)?
            .map(|c| count_py(py, &c))
            .transpose()
    }

    fn is_resolvable(&self) -> PyResult<bool> {
        self.source.is_resolvable().map_err(to_py_err)
    }

    /// `(order, reason)` as the values of the facade's `Order` and `OrderReason`.
    fn order_preference(&self) -> PyResult<(i64, i64)> {
        let (order, reason): (Order, OrderReason) =
            self.source.order_preference().map_err(to_py_err)?;
        Ok((order.value(), reason as i64))
    }

    /// `WeightlessSource.pop(order, outcome)`: `(popped source, count, 1)` for each possibility of the wrapped
    /// source; `UnsupportedOrder` if two have the same count.
    fn pop<'py>(
        &self,
        py: Python<'py>,
        order: i64,
        outcome: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<(RsWeightlessSource, Bound<'py, PyAny>, i64)>> {
        let order = if order > 0 {
            Order::Ascending
        } else {
            Order::Descending
        };
        let popped = self
            .source
            .pop(order, &from_py(outcome)?)
            .map_err(to_py_err)?;
        popped
            .into_iter()
            .map(|(source, count, _)| {
                let MultisetSource::Weightless(source) = source else {
                    unreachable!("a weightless source pops weightless sources")
                };
                Ok((RsWeightlessSource { source }, count_py(py, &count)?, 1))
            })
            .collect()
    }

    /// Python's `hash_key` `(WeightlessSource, wrapped.hash_key)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.source)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsWeightlessSource>()
            .is_ok_and(|o| self.source == o.get().source)
    }
}
