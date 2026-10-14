//! `Pool` (generator/pool.py) and its `PoolSource` on the core, with the keep-tuple work of generator/keep.py.
//!
//! The facade's `Pool.__new__` keeps the argument juggling (`Pool(pool, times)`, `itemize`,
//! `implicit_convert_to_die`) and holds the core pool made here as `_rs`; keeping, multiplying counts, merging
//! pools and popping run in the core. The evaluation driver is still the facade's Python (task S1e): it pops a
//! `PoolSource` wrapping the `icepool_rs.PoolSource` made here.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use dashu_int::IBig;
use icepool_core::keep_tuple::{KeepIndex, Slice};
use icepool_core::order::{Order, OrderReason};
use icepool_core::pool::PoolSource;
use icepool_core::{Pool, Value};
use pyo3::exceptions::{PyNotImplementedError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyEllipsis, PyInt, PyList, PySlice, PyTuple};

use crate::convert::{from_py, int_from_py, quantity_to_py, to_py, to_py_err};
use crate::RsDie;

/// A pool of the Rust core (`Pool<Value>`), held by the facade's `Pool` as `_rs`.
#[pyclass(frozen, name = "Pool", module = "icepool_rs")]
pub struct RsPool {
    pool: Pool<Value>,
}

/// What the evaluator pops outcomes from, held by the facade's `PoolSource` as `_rs`.
#[pyclass(frozen, name = "PoolSource", module = "icepool_rs")]
pub struct RsPoolSource {
    source: PoolSource<Value>,
}

/// One die count of `Pool(dice, times)`. Python adds the counts up and builds `(1, ) * sum(times)`, which raises
/// `TypeError` for a count that is not an `int` (a float, even `0.0`, or a Fraction).
fn die_count(t: &Bound<'_, PyAny>) -> PyResult<usize> {
    if !t.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "can't multiply sequence by non-int of type '{}'",
            t.get_type().name()?
        )));
    }
    // `itemize` has rejected negative counts. Python would build a keep tuple of this many elements.
    usize::try_from(int_from_py(t)?)
        .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("too many dice in the pool"))
}

/// `Pool(dice, times)` after `itemize` and `implicit_convert_to_die`: the dice and one count per die.
#[pyfunction]
pub fn pool_new(dice: Vec<Bound<'_, RsDie>>, times: Vec<Bound<'_, PyAny>>) -> PyResult<RsPool> {
    let counts = times.iter().map(die_count).collect::<PyResult<Vec<_>>>()?;
    let dice = dice
        .iter()
        .map(|d| d.get().die.clone())
        .zip(counts)
        .collect();
    Ok(RsPool {
        pool: Pool::new(dice).map_err(to_py_err)?,
    })
}

/// A Python int as an `i64` count; Python ints are unbounded, the core's keep tuples are not.
fn exact_count(x: &Bound<'_, PyAny>) -> PyResult<i64> {
    if !x.is_instance_of::<PyInt>() {
        return Err(PyNotImplementedError::new_err(format!(
            "icepool_rs: a keep count of type '{}'",
            x.get_type().name()?
        )));
    }
    i64::try_from(int_from_py(x)?)
        .map_err(|_| PyNotImplementedError::new_err("icepool_rs: keep counts beyond 64 bits"))
}

/// An int used as an index or a slice bound: past `i64` it saturates, which Python's list indexing and slicing
/// treat alike (out of range, clamped).
fn saturating_index(x: &Bound<'_, PyAny>) -> PyResult<i64> {
    let i: IBig = int_from_py(&x.py().import("operator")?.call_method1("index", (x,))?)?;
    Ok(i64::try_from(&i).unwrap_or(if i < IBig::ZERO { i64::MIN } else { i64::MAX }))
}

/// The argument of `KeepGenerator.keep` (`pool[index]`) as `make_keep_tuple` reads it: an `int`, a `slice`, or a
/// sequence of counts with at most one `...`.
fn keep_index(index: &Bound<'_, PyAny>) -> PyResult<KeepIndex> {
    if index.is_instance_of::<PyInt>() {
        return saturating_index(index).map(KeepIndex::Int);
    }
    if let Ok(slice) = index.cast::<PySlice>() {
        let step = !slice.getattr("step")?.is_none();
        // A step raises before the list is sliced, whatever the bounds are.
        let bound = |name| -> PyResult<Option<i64>> {
            let b = slice.getattr(name)?;
            if step || b.is_none() {
                return Ok(None);
            }
            saturating_index(&b).map(Some).map_err(|_| {
                PyTypeError::new_err(
                    "slice indices must be integers or None or have an __index__ method",
                )
            })
        };
        let slice = Slice::new(bound("start")?, bound("stop")?);
        return Ok(KeepIndex::Slice { slice, step });
    }
    let counts = index
        .try_iter()?
        .map(|x| {
            let x = x?;
            if x.is(PyEllipsis::get(x.py())) {
                Ok(None)
            } else {
                // Python keeps any element and sums them in `compose_keep_tuples`: only ints are ported.
                exact_count(&x).map(Some)
            }
        })
        .collect::<PyResult<_>>()?;
    Ok(KeepIndex::Seq(counts))
}

fn int_tuple<'py>(py: Python<'py>, xs: &[i64]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(py, xs)
}

fn outcome_tuple<'py>(py: Python<'py>, xs: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        xs.iter()
            .map(|o| to_py(py, o))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// `(die, count)` pairs as `(icepool_rs.Die, int)`, for the facade to wrap into its own dice.
fn dice_list<'py>(
    py: Python<'py>,
    dice: &[(icepool_core::Die<Value>, usize)],
) -> PyResult<Bound<'py, PyList>> {
    PyList::new(
        py,
        dice.iter()
            .map(|(d, c)| (RsDie { die: d.clone() }, *c))
            .collect::<Vec<_>>(),
    )
}

fn hash_of(x: &impl Hash) -> isize {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish() as isize
}

#[pymethods]
impl RsPool {
    /// The `(die, count)` pairs, sorted by the die's `hash_key` (dice put in zero times are not here).
    fn dice<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        dice_list(py, self.pool.dice())
    }

    /// The union of possible outcomes among all dice in this pool in ascending order.
    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.pool.outcomes())
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        int_tuple(py, self.pool.keep_tuple())
    }

    fn raw_size(&self) -> usize {
        self.pool.raw_size()
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.pool.denominator())
    }

    /// `KeepGenerator.keep` for a keep tuple without negative counts (the facade checks), short of evaluating
    /// the result for an `int` index (the facade does that).
    fn keep(&self, index: &Bound<'_, PyAny>) -> PyResult<RsPool> {
        let pool = self.pool.keep(&keep_index(index)?).map_err(to_py_err)?;
        Ok(RsPool { pool })
    }

    /// `KeepGenerator.multiply_counts`: every count multiplied by an int (`-pool` is `-1 * pool`).
    fn multiply_counts(&self, constant: &Bound<'_, PyAny>) -> PyResult<RsPool> {
        let pool = self
            .pool
            .multiply_counts(exact_count(constant)?)
            .map_err(to_py_err)?;
        Ok(RsPool { pool })
    }

    /// `_make_source`.
    fn source(&self) -> RsPoolSource {
        RsPoolSource {
            source: self.pool.source(),
        }
    }

    /// Python's `hash_key` `(Pool, _dice, _keep_tuple)`: the dice and the keep tuple.
    fn __hash__(&self) -> isize {
        hash_of(&(self.pool.dice(), self.pool.keep_tuple()))
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other.cast::<RsPool>().is_ok_and(|o| {
            let o = &o.get().pool;
            self.pool.dice() == o.dice() && self.pool.keep_tuple() == o.keep_tuple()
        })
    }
}

/// `Pool.additive_union` of pools: one merged pool if every sorted position counts the same, else `None` (the
/// facade then goes on to `KeepGenerator.additive_union`).
#[pyfunction]
pub fn pool_additive_union(pools: Vec<Bound<'_, RsPool>>) -> PyResult<Option<RsPool>> {
    let pools: Vec<_> = pools.iter().map(|p| p.get().pool.clone()).collect();
    Ok(Pool::additive_union(&pools)
        .map_err(to_py_err)?
        .map(|pool| RsPool { pool }))
}

#[pymethods]
impl RsPoolSource {
    fn dice<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        dice_list(py, self.source.dice())
    }

    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes())
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        int_tuple(py, self.source.keep_tuple())
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.source.denominator())
    }

    /// `KeepSource.size`: `None` with negative counts.
    fn size(&self) -> PyResult<Option<i64>> {
        self.source.size().map_err(to_py_err)
    }

    fn is_resolvable(&self) -> bool {
        self.source.is_resolvable()
    }

    /// `(order, reason)` as the values of the facade's `Order` and `OrderReason`.
    fn order_preference(&self) -> (i64, i64) {
        let (order, reason): (Order, OrderReason) = self.source.order_preference();
        (order.value(), reason as i64)
    }

    /// `PoolSource.pop(order, outcome)`: `(popped source, count, weight)` for each possibility. `order` is the
    /// value of an `Order` (`order > 0` pops the min outcome, else the max).
    fn pop<'py>(
        &self,
        py: Python<'py>,
        order: i64,
        outcome: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<(RsPoolSource, i64, Bound<'py, PyAny>)>> {
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
            .map(|(source, count, weight)| {
                Ok((RsPoolSource { source }, count, quantity_to_py(py, &weight)?))
            })
            .collect()
    }

    /// Python's `hash_key` `(PoolSource, dice, keep_tuple)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.source)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsPoolSource>()
            .is_ok_and(|o| self.source == o.get().source)
    }
}
