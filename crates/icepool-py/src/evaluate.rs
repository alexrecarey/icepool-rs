//! The evaluation driver of evaluator/multiset_evaluator_base.py (`Dungeon.evaluate`) on the core, for generator
//! inputs: every input is a `Pool` whose `PoolSource` is the core's.
//!
//! The facade's `MultisetEvaluatorBase.evaluate` keeps its argument conversion, `_prepare` and the outer
//! `Die(final_data)`; for each prepared evaluation over core sources it calls in here instead of
//! `dungeon.evaluate(quest, sources, kwargs)`, and builds `icepool.Die(final_outcomes, final_weights)` from what
//! comes back (as `Quest.finalize_evaluation` does). Two ways in:
//!
//! - [`evaluate_builtin`]: the built-in evaluators the core has (`sum_evaluator`, `size_evaluator`, poker, keep,
//!   comparisons...), named by the facade with their parameters. Everything runs in Rust.
//! - [`evaluate_host`]: any other `MultisetEvaluator` (user subclasses included). The core driver pops the sources
//!   and memoises rooms; `next_state` / `initial_state` / `extra_outcomes` / `final_outcome` are host callbacks,
//!   and states are Python objects (native `Value`s where the core models the type, `Value::Host` otherwise).
//!
//! Rooms are cached per call (ANALYSIS.md decision 5), not on the dungeon across calls as Python does.

use std::cell::RefCell;

use icepool_core::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, ProductOfCountsEvaluator, SizeEvaluator, SumEvaluator,
};
use icepool_core::evaluator::comparison::{Comparison, ComparisonEvaluator};
use icepool_core::evaluator::keep::KeepEvaluator;
use icepool_core::evaluator::poker::{
    AllCountsEvaluator, AllStraightsEvaluator, CountSubsetEvaluator,
    HighestOutcomeAndCountEvaluator, LargestCountAndOutcomeEvaluator, LargestCountEvaluator,
    LargestStraightAndOutcomeEvaluator, LargestStraightEvaluator,
};
use icepool_core::evaluator::{evaluate_with_depth_check, Final, MultisetEvaluator};
use icepool_core::order::Order;
use icepool_core::pool::MultisetSource;
use icepool_core::{Error, Quantity, Value};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple, PyType};
use pyo3::IntoPyObjectExt;

use crate::convert::{from_py, host_error, quantity_to_py, to_py, to_py_err};
use crate::deck::RsDealSource;
use crate::pool::RsPoolSource;

/// Python's evaluation recurses once per outcome (`evaluate_backward` / `evaluate_forward`), so it raises
/// `RecursionError` once the outcomes exceed what is left of the recursion limit. The facade passes
/// `depth_budget` = the recursion limit minus its current stack depth minus the frames Python's own driver uses
/// around the recursion; the core raises from there (it recurses natively and has no such limit).
fn depth_check(depth_budget: isize) -> impl Fn(usize) -> icepool_core::Result<()> {
    move |n| {
        if n as isize > depth_budget {
            Err(Error::Value(
                "RecursionError: maximum recursion depth exceeded".into(),
            ))
        } else {
            Ok(())
        }
    }
}

/// The core sources of the facade's `PoolSource`s and `DealSource`s (their `_rs`).
fn sources(sources: &[Bound<'_, PyAny>]) -> PyResult<Vec<MultisetSource<Value>>> {
    sources
        .iter()
        .map(|s| {
            if let Ok(s) = s.cast::<RsPoolSource>() {
                return Ok(MultisetSource::Pool(s.get().source.clone()));
            }
            Ok(MultisetSource::Deal(
                s.cast::<RsDealSource>()?.get().source.clone(),
            ))
        })
        .collect()
}

/// `(final_outcomes, final_weights)` as Python lists, for `icepool.Die(final_outcomes, final_weights)`.
type Finals<'py> = (Bound<'py, PyList>, Bound<'py, PyList>);

fn finals<'py, F>(
    py: Python<'py>,
    finals: Vec<(F, Quantity)>,
    outcome: impl Fn(F) -> PyResult<Bound<'py, PyAny>>,
) -> PyResult<Finals<'py>> {
    let outcomes = PyList::empty(py);
    let weights = PyList::empty(py);
    for (o, w) in finals {
        outcomes.append(outcome(o)?)?;
        weights.append(quantity_to_py(py, &w)?)?;
    }
    Ok((outcomes, weights))
}

fn run<'py, E: MultisetEvaluator<Value, Final = Value>>(
    py: Python<'py>,
    evaluator: &E,
    srcs: &[Bound<'py, PyAny>],
    depth_budget: isize,
) -> PyResult<Finals<'py>> {
    let result = evaluate_with_depth_check(evaluator, &sources(srcs)?, depth_check(depth_budget))
        .map_err(to_py_err)?;
    finals(py, result, |o| to_py(py, &o))
}

/// A built-in evaluator of the core, named by the facade (evaluator_rust.py `builtin_spec`) with its parameters.
/// The facade only names one when the evaluator's type is exactly the built-in class, the number of inputs is the
/// one its `next_state` takes, and the keyword arguments are the ones it reads.
#[pyfunction]
pub fn evaluate_builtin<'py>(
    py: Python<'py>,
    name: &str,
    params: Vec<Bound<'py, PyAny>>,
    sources: Vec<Bound<'py, PyAny>>,
    depth_budget: isize,
) -> PyResult<Finals<'py>> {
    let param = |i: usize| -> PyResult<&Bound<'py, PyAny>> {
        params
            .get(i)
            .ok_or_else(|| PyValueError::new_err(format!("icepool_rs: {name} needs a parameter")))
    };
    let s = &sources;
    let b = depth_budget;
    match name {
        "sum" => run(py, &SumEvaluator, s, b),
        "size" => run(py, &SizeEvaluator, s, b),
        "product_of_counts" => run(py, &ProductOfCountsEvaluator, s, b),
        "empty" => {
            let any = param(0)?.extract()?;
            run(py, &EmptyEvaluator { any }, s, b)
        }
        "expand" => {
            let ascending = param(0)?.extract()?;
            run(py, &ExpandEvaluator { ascending }, s, b)
        }
        "keep" => {
            let index = param(0)?.extract()?;
            run(py, &KeepEvaluator { index }, s, b)
        }
        "highest_outcome_and_count" => run(py, &HighestOutcomeAndCountEvaluator, s, b),
        "all_counts" => {
            let filter = from_py(param(0)?)?;
            run(py, &AllCountsEvaluator { filter }, s, b)
        }
        "largest_count" => run(py, &LargestCountEvaluator, s, b),
        "largest_count_and_outcome" => run(py, &LargestCountAndOutcomeEvaluator, s, b),
        "count_subset" => {
            let empty_divisor = from_py(param(0)?)?;
            run(py, &CountSubsetEvaluator { empty_divisor }, s, b)
        }
        "largest_straight" => run(py, &LargestStraightEvaluator, s, b),
        "largest_straight_and_outcome" => {
            let prioritize_highest = param(0)?.extract()?;
            let e = LargestStraightAndOutcomeEvaluator { prioritize_highest };
            run(py, &e, s, b)
        }
        "all_straights" => run(py, &AllStraightsEvaluator, s, b),
        "is_proper_subset" => run(py, &ComparisonEvaluator(Comparison::IsProperSubset), s, b),
        "is_subset" => run(py, &ComparisonEvaluator(Comparison::IsSubset), s, b),
        "is_proper_superset" => run(py, &ComparisonEvaluator(Comparison::IsProperSuperset), s, b),
        "is_superset" => run(py, &ComparisonEvaluator(Comparison::IsSuperset), s, b),
        "is_equal_set" => run(py, &ComparisonEvaluator(Comparison::IsEqualSet), s, b),
        "is_not_equal_set" => run(py, &ComparisonEvaluator(Comparison::IsNotEqualSet), s, b),
        "is_disjoint_set" => run(py, &ComparisonEvaluator(Comparison::IsDisjointSet), s, b),
        _ => Err(PyValueError::new_err(format!(
            "icepool_rs: no built-in evaluator {name:?}"
        ))),
    }
}

/// A `MultisetEvaluator` whose methods are Python callables: the bound methods the facade's
/// `MultisetEvaluatorDungeon` / `MultisetEvaluatorQuest` hold (`next_state_main`, `initial_state_main`,
/// `extra_outcomes`, `final_outcome`), called exactly as `Dungeon.evaluate` calls them.
struct HostEvaluator<'py> {
    next_state: Bound<'py, PyAny>,
    initial_state: Bound<'py, PyAny>,
    extra_outcomes: Bound<'py, PyAny>,
    final_outcome: Bound<'py, PyAny>,
    kwargs: Option<Bound<'py, PyDict>>,
    /// `icepool.Order`: the order is passed to the callbacks as the enum member.
    order_type: Bound<'py, PyAny>,
    /// `icepool.REROLL_TYPES`: `x in REROLL_TYPES` is Python's own containment test.
    reroll_types: Bound<'py, PyAny>,
    /// `icepool.UnsupportedOrder` (and subclasses): the core's signal to retry in the other order.
    unsupported_order: Bound<'py, PyType>,
    /// The `outcomes` tuple passed to `initial_state` / `final_outcome`: always the evaluation's `all_outcomes`,
    /// so it is converted once per attempt (keyed by the slice it came from).
    outcomes: RefCell<Option<(usize, usize, Bound<'py, PyTuple>)>>,
}

impl<'py> HostEvaluator<'py> {
    fn py(&self) -> Python<'py> {
        self.next_state.py()
    }

    fn order(&self, order: Order) -> icepool_core::Result<Bound<'py, PyAny>> {
        self.order_type.call1((order.value(),)).map_err(host_error)
    }

    /// A Python exception from `initial_state` / `next_state` / `final_outcome`. `UnsupportedOrder` is caught by
    /// `Dungeon.evaluate` around all three, so it becomes the core's retry signal (its `str` goes into the
    /// `ConflictingOrderError` message); anything else is re-raised as it was.
    fn callback_error(&self, e: PyErr) -> Error {
        let py = self.py();
        if e.is_instance(py, &self.unsupported_order) {
            let msg = e.value(py).str().map(|s| s.to_string()).unwrap_or_default();
            return Error::UnsupportedOrder(msg);
        }
        host_error(e)
    }

    fn outcomes_tuple(&self, outcomes: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
        let key = (outcomes.as_ptr() as usize, outcomes.len());
        if let Some((p, n, t)) = &*self.outcomes.borrow() {
            if (*p, *n) == key {
                return Ok(t.clone());
            }
        }
        let t = values_tuple(self.py(), outcomes)?;
        *self.outcomes.borrow_mut() = Some((key.0, key.1, t.clone()));
        Ok(t)
    }

    /// `(order, outcomes, *sizes)`: the positional arguments of `initial_state` / `final_outcome` after the state.
    fn order_outcomes_sizes(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> icepool_core::Result<Vec<Bound<'py, PyAny>>> {
        let py = self.py();
        let mut args = vec![
            self.order(order)?,
            self.outcomes_tuple(outcomes)
                .map_err(host_error)?
                .into_any(),
        ];
        for s in sizes {
            args.push(s.into_bound_py_any(py).map_err(host_error)?);
        }
        Ok(args)
    }

    fn call(
        &self,
        f: &Bound<'py, PyAny>,
        args: Vec<Bound<'py, PyAny>>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> icepool_core::Result<Bound<'py, PyAny>> {
        let args = PyTuple::new(self.py(), args).map_err(host_error)?;
        f.call(args, kwargs).map_err(|e| self.callback_error(e))
    }

    fn is_reroll(&self, x: &Bound<'py, PyAny>) -> icepool_core::Result<bool> {
        self.reroll_types.contains(x).map_err(host_error)
    }
}

fn values_tuple<'py>(py: Python<'py>, xs: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        xs.iter()
            .map(|x| to_py(py, x))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

impl<'py> MultisetEvaluator<Value> for HostEvaluator<'py> {
    /// A state is any hashable Python object: `Value`'s `==`/hash are Python's, so equal states merge in the
    /// core's `defaultdict` as in Python's (keeping the first one).
    type State = Value;
    /// The final outcome as Python returned it (it may be a die), for `icepool.Die(final_outcomes, ...)`.
    type Final = Py<PyAny>;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> icepool_core::Result<Value> {
        // `quest.initial_state_main(order, outcomes, *arg_sizes, **kwargs)`
        let args = self.order_outcomes_sizes(order, outcomes, sizes)?;
        let state = self.call(&self.initial_state, args, self.kwargs.as_ref())?;
        from_py(&state).map_err(host_error)
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> icepool_core::Result<Option<Value>> {
        // `self.next_state_main(prev_state_main, eval_order, outcome, *count_tree)`
        let py = self.py();
        let mut args = vec![
            to_py(py, state).map_err(host_error)?,
            self.order(order)?,
            to_py(py, outcome).map_err(host_error)?,
        ];
        for c in counts {
            args.push(c.into_bound_py_any(py).map_err(host_error)?);
        }
        let next = self.call(&self.next_state, args, None)?;
        // `if state_main not in icepool.REROLL_TYPES`: a Reroll drops this branch.
        if self.is_reroll(&next)? {
            return Ok(None);
        }
        from_py(&next).map(Some).map_err(host_error)
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> icepool_core::Result<Vec<Value>> {
        // Outside `Dungeon.evaluate`'s try: an `UnsupportedOrder` here propagates as it is.
        let py = self.py();
        let outcomes = values_tuple(py, outcomes).map_err(host_error)?;
        let extra = self.extra_outcomes.call1((outcomes,)).map_err(host_error)?;
        extra
            .try_iter()
            .and_then(|it| it.map(|x| from_py(&x?)).collect::<PyResult<Vec<_>>>())
            .map_err(host_error)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> icepool_core::Result<Final<Py<PyAny>>> {
        // `self.final_outcome(state, order, outcomes, *arg_sizes, **kwargs)`
        let mut args = vec![to_py(self.py(), final_state).map_err(host_error)?];
        args.extend(self.order_outcomes_sizes(order, outcomes, sizes)?);
        let outcome = self.call(&self.final_outcome, args, self.kwargs.as_ref())?;
        Ok(if outcome.is_none() {
            Final::None
        } else if self.is_reroll(&outcome)? {
            Final::Reroll
        } else {
            Final::Outcome(outcome.unbind())
        })
    }
}

/// Any `MultisetEvaluator` over core sources, its methods called back in Python (see [`HostEvaluator`]).
#[pyfunction]
#[pyo3(signature = (next_state, initial_state, extra_outcomes, final_outcome, kwargs, sources, depth_budget))]
#[allow(clippy::too_many_arguments)]
pub fn evaluate_host<'py>(
    py: Python<'py>,
    next_state: Bound<'py, PyAny>,
    initial_state: Bound<'py, PyAny>,
    extra_outcomes: Bound<'py, PyAny>,
    final_outcome: Bound<'py, PyAny>,
    kwargs: Option<Bound<'py, PyDict>>,
    sources: Vec<Bound<'py, PyAny>>,
    depth_budget: isize,
) -> PyResult<Finals<'py>> {
    let icepool = py.import("icepool")?;
    let evaluator = HostEvaluator {
        next_state,
        initial_state,
        extra_outcomes,
        final_outcome,
        kwargs,
        order_type: icepool.getattr("Order")?,
        reroll_types: icepool.getattr("REROLL_TYPES")?,
        unsupported_order: icepool.getattr("UnsupportedOrder")?.cast_into::<PyType>()?,
        outcomes: RefCell::new(None),
    };
    let result = evaluate_with_depth_check(
        &evaluator,
        &self::sources(&sources)?,
        depth_check(depth_budget),
    )
    .map_err(to_py_err)?;
    finals(py, result, |o| Ok(o.into_bound(py)))
}
