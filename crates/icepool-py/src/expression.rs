//! Multiset expressions on the core: `CompoundKeepGenerator` / `CompoundKeepSource` (generator/compound_keep.py)
//! and the operators of operator/*.py as the core's `Operator`s, for the evaluation driver (evaluate.rs).
//!
//! The facade keeps its expression classes and their argument juggling: an expression is still built and
//! `_prepare`d in Python. For an evaluation, evaluator_rust.py describes each prepared input (its flat of
//! dungeonlets) as a list of node specs, read here into the core's `expression::Dungeonlet`s: `None` for a free
//! variable, `(spec, child_indexes)` for an operator. The spec tuple names the operator class and carries the
//! attributes its `_next_state` / `_initial_state` read. `map_counts` functions and `keep_outcomes` callables are
//! host callbacks.

use std::sync::Arc;

use icepool_core::compound_keep::{CompoundKeep, CompoundKeepSource};
use icepool_core::expression::{Dungeonlet, Flat};
use icepool_core::operator::{
    BinaryOperator, CountFunction, KeepCountsComparison, MultisetKeepIndex, Operator,
    OutcomeFilter, OutcomePredicate,
};
use icepool_core::order::{Order, OrderReason};
use icepool_core::pool::MultisetSource;
use icepool_core::value::builtins::Index;
use icepool_core::{Error, Value};
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyEllipsis, PyInt, PyList, PySlice, PyTuple, PyType};
use pyo3::IntoPyObjectExt;

use crate::convert::{from_py, host_error, int_from_py, quantity_to_py, to_py, to_py_err};
use crate::deck::RsDealSource;
use crate::multi_deal::RsMultiDealSource;
use crate::pool::{exact_count, hash_of, keep_index, outcome_tuple, RsPoolSource};

/// A `CompoundKeepGenerator` of the core, held by the facade's as `_rs`.
#[pyclass(frozen, name = "CompoundKeep", module = "icepool_rs")]
pub struct RsCompoundKeep {
    compound: CompoundKeep<Value>,
}

/// A `CompoundKeepSource` of the core, held by the facade's as `_rs`.
#[pyclass(frozen, name = "CompoundKeepSource", module = "icepool_rs")]
pub struct RsCompoundKeepSource {
    pub(crate) source: CompoundKeepSource<Value>,
}

/// The core source of a facade source's `_rs`: a pool's, deal's, multi-deal's or compound keep's.
pub(crate) fn core_source(s: &Bound<'_, PyAny>) -> PyResult<MultisetSource<Value>> {
    if let Ok(s) = s.cast::<RsPoolSource>() {
        return Ok(MultisetSource::Pool(s.get().source.clone()));
    }
    if let Ok(s) = s.cast::<RsDealSource>() {
        return Ok(MultisetSource::Deal(s.get().source.clone()));
    }
    if let Ok(s) = s.cast::<RsMultiDealSource>() {
        return Ok(MultisetSource::MultiDeal(s.get().source.clone()));
    }
    Ok(MultisetSource::CompoundKeep(
        s.cast::<RsCompoundKeepSource>()?.get().source.clone(),
    ))
}

/// A core source as the `icepool_rs` object the facade wraps (`PoolSource._new_rust(...)` and so on).
fn source_object<'py>(py: Python<'py>, s: &MultisetSource<Value>) -> PyResult<Bound<'py, PyAny>> {
    Ok(match s {
        MultisetSource::Pool(s) => Bound::new(py, RsPoolSource { source: s.clone() })?.into_any(),
        MultisetSource::Deal(s) => Bound::new(py, RsDealSource { source: s.clone() })?.into_any(),
        MultisetSource::MultiDeal(s) => {
            Bound::new(py, RsMultiDealSource { source: s.clone() })?.into_any()
        }
        MultisetSource::CompoundKeep(s) => {
            Bound::new(py, RsCompoundKeepSource { source: s.clone() })?.into_any()
        }
    })
}

fn keep_counts(keep_tuple: &[Bound<'_, PyAny>]) -> PyResult<Vec<i64>> {
    keep_tuple.iter().map(exact_count).collect()
}

/// `CompoundKeepGenerator(inners, keep_tuple)`, given the inners' sources (`_make_source()._rs`).
#[pyfunction]
pub fn compound_keep_new(
    inner_sources: Vec<Bound<'_, PyAny>>,
    keep_tuple: Vec<Bound<'_, PyAny>>,
) -> PyResult<RsCompoundKeep> {
    let sources = inner_sources
        .iter()
        .map(core_source)
        .collect::<PyResult<Vec<_>>>()?;
    Ok(RsCompoundKeep {
        compound: CompoundKeep::new(sources, keep_counts(&keep_tuple)?),
    })
}

#[pymethods]
impl RsCompoundKeep {
    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.compound.keep_tuple())
    }

    /// `_set_keep_tuple`.
    fn with_keep_tuple(&self, keep_tuple: Vec<Bound<'_, PyAny>>) -> PyResult<RsCompoundKeep> {
        Ok(RsCompoundKeep {
            compound: self.compound.with_keep_tuple(keep_counts(&keep_tuple)?),
        })
    }

    /// `KeepGenerator.keep` for a keep tuple without negative counts (the facade checks), short of evaluating
    /// the result for an `int` index.
    fn keep(&self, index: &Bound<'_, PyAny>) -> PyResult<RsCompoundKeep> {
        let compound = self.compound.keep(&keep_index(index)?).map_err(to_py_err)?;
        Ok(RsCompoundKeep { compound })
    }

    /// `KeepGenerator.multiply_counts`.
    fn multiply_counts(&self, constant: &Bound<'_, PyAny>) -> PyResult<RsCompoundKeep> {
        let compound = self
            .compound
            .multiply_counts(exact_count(constant)?)
            .map_err(to_py_err)?;
        Ok(RsCompoundKeep { compound })
    }

    /// `_make_source`.
    fn source(&self) -> PyResult<RsCompoundKeepSource> {
        Ok(RsCompoundKeepSource {
            source: self.compound.source().map_err(to_py_err)?,
        })
    }

    /// Python's `hash_key` `(CompoundKeepGenerator, inner_generators, keep_tuple)`: the inners' sources compare
    /// as their generators do.
    fn __hash__(&self) -> isize {
        hash_of(&self.compound)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsCompoundKeep>()
            .is_ok_and(|o| self.compound == o.get().compound)
    }
}

#[pymethods]
impl RsCompoundKeepSource {
    fn outcomes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        outcome_tuple(py, self.source.outcomes().map_err(to_py_err)?)
    }

    fn keep_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.source.keep_tuple())
    }

    /// The inner sources as `icepool_rs` objects, for the facade to wrap.
    fn inner_sources<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let items = self
            .source
            .inner_sources()
            .iter()
            .map(|s| source_object(py, s))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, items)
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

    /// `CompoundKeepSource.pop(order, outcome)`: `(popped source, count, weight)` for each possibility.
    fn pop<'py>(
        &self,
        py: Python<'py>,
        order: i64,
        outcome: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<(RsCompoundKeepSource, i64, Bound<'py, PyAny>)>> {
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
                    RsCompoundKeepSource { source },
                    count,
                    quantity_to_py(py, &weight)?,
                ))
            })
            .collect()
    }

    /// Python's `hash_key` `(CompoundKeepSource, inner_sources, keep_tuple)`.
    fn __hash__(&self) -> isize {
        hash_of(&self.source)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<RsCompoundKeepSource>()
            .is_ok_and(|o| self.source == o.get().source)
    }
}

/// A Python exception from a callback inside the evaluation: `UnsupportedOrder` (raised anywhere under
/// `Dungeon.evaluate`'s try) is the core's retry signal; anything else is re-raised as it was.
fn callback_error(py: Python<'_>, e: PyErr) -> Error {
    let unsupported = py
        .import("icepool")
        .and_then(|m| m.getattr("UnsupportedOrder"))
        .and_then(|c| c.cast_into::<PyType>().map_err(PyErr::from));
    match unsupported {
        Ok(cls) if e.is_instance(py, &cls) => {
            let msg = e.value(py).str().map(|s| s.to_string()).unwrap_or_default();
            Error::UnsupportedOrder(msg)
        }
        _ => host_error(e),
    }
}

/// `MultisetMapCounts._function`, called as `function(outcome, *counts)`. Python's counts can be anything it
/// returns; the core's are `int`s, so another result (a bool too) is a facade gap.
struct HostCountFunction(Py<PyAny>);

impl CountFunction<Value> for HostCountFunction {
    fn call(&self, outcome: &Value, counts: &[i64]) -> icepool_core::Result<i64> {
        Python::attach(|py| {
            let mut args = vec![to_py(py, outcome).map_err(host_error)?];
            for &c in counts {
                args.push(c.into_bound_py_any(py).map_err(host_error)?);
            }
            let args = PyTuple::new(py, args).map_err(host_error)?;
            let count = self
                .0
                .bind(py)
                .call1(args)
                .map_err(|e| callback_error(py, e))?;
            if count.is_instance_of::<PyBool>() || !count.is_instance_of::<PyInt>() {
                return Err(Error::Unsupported(
                    "a map_counts function returning something other than an int".into(),
                ));
            }
            let count = int_from_py(&count).map_err(host_error)?;
            i64::try_from(&count)
                .map_err(|_| Error::Unsupported("a map_counts count beyond 64 bits".into()))
        })
    }
}

/// `MultisetFilterOutcomes._func` given as a callable: `bool(func(outcome))`.
struct HostPredicate(Py<PyAny>);

impl OutcomePredicate<Value> for HostPredicate {
    fn test(&self, outcome: &Value) -> icepool_core::Result<bool> {
        Python::attach(|py| {
            let o = to_py(py, outcome).map_err(host_error)?;
            let r = self
                .0
                .bind(py)
                .call1((o,))
                .map_err(|e| callback_error(py, e))?;
            r.is_truthy().map_err(|e| callback_error(py, e))
        })
    }
}

fn unsupported(what: &str) -> PyErr {
    PyNotImplementedError::new_err(format!("icepool_rs: {what}"))
}

/// An `int` attribute of an operator (a constant, an `Order`): `i64`, or a facade gap.
fn int_attr(x: &Bound<'_, PyAny>, what: &str) -> PyResult<i64> {
    if !x.is_instance_of::<PyInt>() {
        return Err(unsupported(&format!("{what} that is not an int")));
    }
    i64::try_from(&int_from_py(x)?).map_err(|_| unsupported(&format!("{what} beyond 64 bits")))
}

/// A count constant that `min(count, constant)` would return as is (`unique`): a bool would stay a bool.
fn strict_int_attr(x: &Bound<'_, PyAny>, what: &str) -> PyResult<i64> {
    if x.is_instance_of::<PyBool>() {
        return Err(unsupported(&format!("{what} that is a bool")));
    }
    int_attr(x, what)
}

/// A `str` attribute that Python only compares with string literals: anything else never matches (ValueError
/// once evaluated).
fn str_attr(x: &Bound<'_, PyAny>) -> Arc<str> {
    match x.extract::<String>() {
        Ok(s) => Arc::from(s),
        _ => Arc::from("\0not a str"),
    }
}

fn bool_attr(x: &Bound<'_, PyAny>, what: &str) -> PyResult<bool> {
    if !x.is_instance_of::<PyBool>() {
        return Err(unsupported(&format!("{what} that is not a bool")));
    }
    x.extract()
}

/// `MultisetKeep._index`: a slice with `int` (or `None`) bounds, or a list / tuple of `int`s and `...`.
fn keep_index_attr(index: &Bound<'_, PyAny>) -> PyResult<MultisetKeepIndex> {
    if let Ok(slice) = index.cast::<PySlice>() {
        let bound = |name| -> PyResult<Option<i64>> {
            let b = slice.getattr(name)?;
            if b.is_none() {
                return Ok(None);
            }
            // Past `i64` a bound is clamped: the keep tuples it would build are refused anyway.
            if !b.is_instance_of::<PyInt>() {
                return Err(unsupported("a MultisetKeep slice bound that is not an int"));
            }
            let i = int_from_py(&b)?;
            Ok(Some(i64::try_from(&i).unwrap_or(if i < 0.into() {
                i64::MIN
            } else {
                i64::MAX
            })))
        };
        return Ok(MultisetKeepIndex::Slice {
            start: bound("start")?,
            stop: bound("stop")?,
            step: !slice.getattr("step")?.is_none(),
        });
    }
    if index.is_instance_of::<PyList>() || index.is_instance_of::<PyTuple>() {
        let items = index
            .try_iter()?
            .map(|x| {
                let x = x?;
                if x.is(PyEllipsis::get(x.py())) {
                    Ok(None)
                } else {
                    int_attr(&x, "a MultisetKeep index element").map(Some)
                }
            })
            .collect::<PyResult<_>>()?;
        return Ok(MultisetKeepIndex::Seq(items));
    }
    Err(unsupported(
        "a MultisetKeep index that is not a slice, list or tuple",
    ))
}

/// An operator spec built by evaluator_rust.py `_operator_spec`: `(name, *attributes)`.
fn operator(spec: &Bound<'_, PyTuple>) -> PyResult<Operator<Value>> {
    let name: String = spec.get_item(0)?.extract()?;
    let a = |i: usize| spec.get_item(i);
    Ok(match name.as_str() {
        "binary" => Operator::Binary(match a(1)?.extract::<String>()?.as_str() {
            "intersection" => BinaryOperator::Intersection,
            "difference_drop_negative" => BinaryOperator::DifferenceDropNegative,
            "difference_keep_negative" => BinaryOperator::DifferenceKeepNegative,
            "union" => BinaryOperator::Union,
            "additive_union" => BinaryOperator::AdditiveUnion,
            "symmetric_difference" => BinaryOperator::SymmetricDifference,
            other => {
                return Err(PyValueError::new_err(format!(
                    "icepool_rs: no binary operator {other}"
                )))
            }
        }),
        "map_counts" => Operator::MapCounts(Arc::new(HostCountFunction(a(1)?.unbind()))),
        "multiply_counts" => {
            Operator::MultiplyCounts(int_attr(&a(1)?, "a multiply_counts constant")?)
        }
        "floordiv_counts" => {
            Operator::FloordivCounts(int_attr(&a(1)?, "a divide_counts constant")?)
        }
        "modulo_counts" => Operator::ModuloCounts(int_attr(&a(1)?, "a modulo_counts constant")?),
        "unique" => Operator::Unique(strict_int_attr(&a(1)?, "a unique constant")?),
        "keep_counts" => {
            let comparison: String = a(1)?.extract()?;
            Operator::KeepCounts(
                KeepCountsComparison::parse(&comparison).map_err(to_py_err)?,
                int_attr(&a(2)?, "a keep_counts constant")?,
            )
        }
        "filter_function" => Operator::FilterOutcomes {
            filter: OutcomeFilter::Function(Arc::new(HostPredicate(a(1)?.unbind()))),
            invert: bool_attr(&a(2)?, "an invert flag")?,
        },
        "filter_set" => {
            let outcomes = a(1)?
                .try_iter()?
                .map(|x| from_py(&x?))
                .collect::<PyResult<Vec<_>>>()?;
            Operator::FilterOutcomes {
                filter: OutcomeFilter::Set(outcomes.into()),
                invert: bool_attr(&a(2)?, "an invert flag")?,
            }
        }
        "filter_binary" => Operator::FilterOutcomesBinary {
            invert: bool_attr(&a(1)?, "an invert flag")?,
        },
        "keep" => Operator::Keep(keep_index_attr(&a(1)?)?),
        "sort_pair" => Operator::SortPair {
            comparison: str_attr(&a(1)?),
            sort_order: int_attr(&a(2)?, "a sort order")?,
            extra: str_attr(&a(3)?),
        },
        "sort_pair_while" => Operator::SortPairWhile {
            keep: a(1)?.is_truthy()?,
            comparison: str_attr(&a(2)?),
            sort_order: int_attr(&a(3)?, "a sort order")?,
            extra: str_attr(&a(4)?),
        },
        "max_pair_late" | "max_pair_early" => {
            let (order, pair_equal, keep) = (
                int_attr(&a(1)?, "an order")?,
                a(2)?.is_truthy()?,
                a(3)?.is_truthy()?,
            );
            if name == "max_pair_late" {
                Operator::MaxPairLate {
                    order,
                    pair_equal,
                    keep,
                }
            } else {
                Operator::MaxPairEarly {
                    order,
                    pair_equal,
                    keep,
                }
            }
        }
        "versus" => {
            let lexi: Vec<Bound<'_, PyAny>> = a(1)?.extract()?;
            let lexi = lexi
                .iter()
                .map(|x| int_attr(x, "a lexi tuple element"))
                .collect::<PyResult<Vec<_>>>()?;
            let [l, t, r] = lexi[..] else {
                return Err(unsupported("a lexi tuple that is not a triple"));
            };
            Operator::Versus {
                lexi_tuple: (l, t, r),
                order: int_attr(&a(2)?, "an order")?,
            }
        }
        "force_order" => Operator::ForceOrder(int_attr(&a(1)?, "an order")?),
        other => {
            return Err(PyValueError::new_err(format!(
                "icepool_rs: no operator {other}"
            )))
        }
    })
}

/// The index of a `MultisetTupleIndex` (an int) or `MultisetTupleSlice` (a slice).
fn tuple_index(index: &Bound<'_, PyAny>) -> PyResult<Index> {
    if let Ok(s) = index.cast::<PySlice>() {
        let bound = |name| from_py(&s.getattr(name)?);
        return Ok(Index::Slice(
            bound("start")?,
            bound("stop")?,
            bound("step")?,
        ));
    }
    Ok(Index::Item(from_py(index)?))
}

/// One prepared input as evaluator_rust.py describes it: `None` for a free variable, `('param', arg_index,
/// star_index)` for a `@multiset_function` parameter, `(spec, child_indexes)` for an operator or a tuple item
/// (spec `('tuple_item', index)`), with Python's relative (negative) child indexes.
pub(crate) fn flat(nodes: &Bound<'_, PyAny>) -> PyResult<Flat<Value>> {
    let mut result = Vec::new();
    for node in nodes.try_iter()? {
        let node = node?;
        if node.is_none() {
            result.push(Dungeonlet::FreeVariable);
            continue;
        }
        if let Ok(("param", arg_index, star_index)) =
            node.extract::<(String, usize, Option<usize>)>()
                .as_ref()
                .map(|(k, a, s)| (k.as_str(), *a, *s))
        {
            result.push(Dungeonlet::Parameter {
                arg_index,
                star_index,
            });
            continue;
        }
        let (spec, child_indexes): (Bound<'_, PyTuple>, Vec<isize>) = node.extract()?;
        let here = result.len() as isize;
        let children = child_indexes
            .iter()
            .map(|&i| {
                usize::try_from(here + i)
                    .map_err(|_| PyValueError::new_err("icepool_rs: a child index out of range"))
            })
            .collect::<PyResult<Vec<_>>>()?;
        if spec.get_item(0)?.extract::<String>().ok().as_deref() == Some("tuple_item") {
            let [child] = children[..] else {
                return Err(PyValueError::new_err("icepool_rs: a tuple item has one child"));
            };
            result.push(Dungeonlet::TupleItem {
                child,
                index: tuple_index(&spec.get_item(1)?)?,
            });
            continue;
        }
        result.push(Dungeonlet::Operator {
            operator: operator(&spec)?,
            children,
        });
    }
    Ok(Flat::from(result))
}
