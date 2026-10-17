//! `Pool` (generator/pool.py) and its `PoolSource` on the core, with the keep-tuple work of generator/keep.py.
//!
//! The facade's `Pool.__new__` keeps the argument juggling (`Pool(pool, times)`, `itemize`,
//! `implicit_convert_to_die`) and holds the core pool made here as `_rs`; keeping, multiplying counts, merging
//! pools and popping run in the core, and so does the evaluation driver over these sources (evaluate.rs).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;
use icepool_core::evaluator::PoolSource;
use icepool_core::evaluator::{Order, OrderReason};
use icepool_core::keep_tuple::{self, KeepIndex, Slice};
use icepool_core::{Int, Pool, Value};
use pyo3::exceptions::{PyNotImplementedError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyEllipsis, PyInt, PyList, PySlice, PyTuple};

use crate::convert::{
    count_from_py, count_to_py, counts_to_py, from_py, int_from_py, quantity_to_py, to_py,
    to_py_err,
};
use crate::RsDie;

/// A pool of the Rust core (`Pool<Value>`), held by the facade's `Pool` as `_rs`.
///
/// It holds its dice's Python objects (the facade dice's `_rs`) rather than clones of their core values, and makes
/// the core pool when asked (`Pool::from_parts`), as `RsDeal` does with its deck: the facade's dice are often alive
/// too, and Python's cycle collector can only be told about Python objects held by one object
/// (`convert::traverse`), so the dice's outcomes stay the dice's alone.
#[pyclass(frozen, name = "Pool", module = "icepool_rs")]
pub struct RsPool {
    /// Each die put in and how many times, in order, dice put in zero times included (their outcomes are the
    /// pool's too); a merged pool has those of all its pools in turn.
    dice: Vec<(Py<RsDie>, usize)>,
    keep_tuple: Arc<[Int]>,
}

/// What the evaluator pops outcomes from, held by the facade's `PoolSource` as `_rs`.
#[pyclass(frozen, name = "PoolSource", module = "icepool_rs")]
pub struct RsPoolSource {
    pub(crate) source: PoolSource<Value>,
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
    let dice: Vec<_> = dice.into_iter().map(Bound::unbind).zip(counts).collect();
    // Made once here for its errors (too many dice, outcomes that do not sort) and its keep tuple.
    let pool = Pool::new(core_dice(&dice)).map_err(to_py_err)?;
    Ok(RsPool {
        dice,
        keep_tuple: pool.keep_tuple().into(),
    })
}

fn core_dice(dice: &[(Py<RsDie>, usize)]) -> Vec<(icepool_core::Die<Value>, usize)> {
    dice.iter()
        .map(|(d, c)| (d.get().die.clone(), *c))
        .collect()
}

impl RsPool {
    /// The core pool.
    pub(crate) fn pool(&self) -> PyResult<Pool<Value>> {
        Pool::from_parts(&core_dice(&self.dice), self.keep_tuple.clone()).map_err(to_py_err)
    }

    /// A pool of the same dice with another keep tuple.
    fn with_keep_tuple(&self, py: Python<'_>, keep_tuple: Vec<Int>) -> RsPool {
        RsPool {
            dice: self
                .dice
                .iter()
                .map(|(d, c)| (d.clone_ref(py), *c))
                .collect(),
            keep_tuple: keep_tuple.into(),
        }
    }
}

/// One possibility of a source's `pop`: `(popped source, count, weight)`, the count and weight as Python ints.
pub(crate) type IntPopped<'py, S> = (S, Bound<'py, PyAny>, Bound<'py, PyAny>);

/// A Python int as a count (unbounded, as Python's).
pub(crate) fn exact_count(x: &Bound<'_, PyAny>) -> PyResult<Int> {
    if !x.is_instance_of::<PyInt>() {
        return Err(PyNotImplementedError::new_err(format!(
            "icepool_rs: a keep count of type '{}'",
            x.get_type().name()?
        )));
    }
    count_from_py(x)
}

/// An int used as an index or a slice bound: past `i64` it saturates, which Python's list indexing and slicing
/// treat alike (out of range, clamped).
fn saturating_index(x: &Bound<'_, PyAny>) -> PyResult<i64> {
    let i: IBig = int_from_py(&x.py().import("operator")?.call_method1("index", (x,))?)?;
    Ok(i64::try_from(&i).unwrap_or(if i < IBig::ZERO { i64::MIN } else { i64::MAX }))
}

/// The argument of `KeepGenerator.keep` (`pool[index]`) as `make_keep_tuple` reads it: an `int`, a `slice`, or a
/// sequence of counts with at most one `...`.
pub(crate) fn keep_index(index: &Bound<'_, PyAny>) -> PyResult<KeepIndex> {
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

pub(crate) fn outcome_tuple<'py>(py: Python<'py>, xs: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
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

pub fn hash_of(x: &impl Hash) -> isize {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish() as isize
}

#[pymethods]
impl RsPool {
    /// Its dice's Python objects, for the cycle collector.
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for (die, _) in &self.dice {
            visit.call(die)?;
        }
        Ok(())
    }

    /// The `(die, count)` pairs, sorted by the die's `hash_key` (dice put in zero times are not here). Each die is
    /// the Python object the pool holds (the first put in of those equal to it), not a new one, so the facade's
    /// `_dice` share nothing with the pool but Python references.
    fn dice<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let pool = self.pool()?;
        let items = pool
            .dice()
            .iter()
            .map(|(die, count)| {
                let held = self
                    .dice
                    .iter()
                    .find(|(d, _)| d.get().die.ptr_eq(die))
                    .map(|(d, _)| d.clone_ref(py))
                    .expect("the pool's dice are clones of the dice put in");
                (held, *count)
            })
            .collect::<Vec<_>>();
        PyList::new(py, items)
    }

    /// The union of possible outcomes among all dice in this pool in ascending order.
    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.pool()?.outcomes())
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        counts_to_py(py, &self.keep_tuple)
    }

    /// The number of dice in this pool before the keep_tuple is applied.
    fn raw_size(&self) -> usize {
        self.dice.iter().map(|(_, c)| c).sum()
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.pool()?.denominator().map_err(to_py_err)?)
    }

    /// `KeepGenerator.keep` for a keep tuple without negative counts (the facade checks), short of evaluating
    /// the result for an `int` index (the facade does that).
    fn keep(&self, index: &Bound<'_, PyAny>) -> PyResult<RsPool> {
        let keep_tuple =
            keep_tuple::keep(&self.keep_tuple, &keep_index(index)?).map_err(to_py_err)?;
        Ok(self.with_keep_tuple(index.py(), keep_tuple))
    }

    /// `KeepGenerator.multiply_counts`: every count multiplied by an int (`-pool` is `-1 * pool`).
    fn multiply_counts(&self, constant: &Bound<'_, PyAny>) -> PyResult<RsPool> {
        let keep_tuple = keep_tuple::multiply_counts(&self.keep_tuple, &exact_count(constant)?);
        Ok(self.with_keep_tuple(constant.py(), keep_tuple))
    }

    /// `_make_source`.
    fn source(&self) -> PyResult<RsPoolSource> {
        Ok(RsPoolSource {
            source: self.pool()?.source(),
        })
    }

    /// Python's `hash_key` `(Pool, _dice, _keep_tuple)`: the dice and the keep tuple.
    fn __hash__(&self) -> PyResult<isize> {
        let pool = self.pool()?;
        Ok(hash_of(&(pool.dice(), pool.keep_tuple())))
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(other) = other.cast::<RsPool>() else {
            return Ok(false);
        };
        let (a, b) = (self.pool()?, other.get().pool()?);
        Ok(a.dice() == b.dice() && a.keep_tuple() == b.keep_tuple())
    }
}

/// `Pool.additive_union` of pools: one merged pool if every sorted position counts the same, else `None` (the
/// facade then goes on to `KeepGenerator.additive_union`).
#[pyfunction]
pub fn pool_additive_union(
    py: Python<'_>,
    pools: Vec<Bound<'_, RsPool>>,
) -> PyResult<Option<RsPool>> {
    let cores = pools
        .iter()
        .map(|p| p.get().pool())
        .collect::<PyResult<Vec<_>>>()?;
    let Some(merged) = Pool::additive_union(&cores).map_err(to_py_err)? else {
        return Ok(None);
    };
    // The merged pool is the pool of all the pools' dice in turn (`Pool::from_parts`).
    let dice = pools
        .iter()
        .flat_map(|p| p.get().dice.iter().map(|(d, c)| (d.clone_ref(py), *c)))
        .collect();
    Ok(Some(RsPool {
        dice,
        keep_tuple: merged.keep_tuple().into(),
    }))
}

#[pymethods]
impl RsPoolSource {
    /// The Python objects it holds, for the cycle collector (`convert::traverse`).
    fn __traverse__(&self, visit: pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        crate::convert::traverse(&self.source, &visit)
    }

    fn dice<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        dice_list(py, self.source.dice())
    }

    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes())
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        counts_to_py(py, self.source.keep_tuple())
    }

    fn denominator<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        quantity_to_py(py, &self.source.denominator().map_err(to_py_err)?)
    }

    /// `KeepSource.size`: `None` with negative counts.
    fn size<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.source.size().map(|s| count_to_py(py, &s)).transpose()
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
    ) -> PyResult<Vec<IntPopped<'py, RsPoolSource>>> {
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
                Ok((
                    RsPoolSource { source },
                    count_to_py(py, &count)?,
                    quantity_to_py(py, &weight)?,
                ))
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
