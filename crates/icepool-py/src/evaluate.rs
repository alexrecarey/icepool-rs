//! The evaluation driver of evaluator/multiset_evaluator_base.py (`Dungeon.evaluate`) on the core, for inputs over
//! core sources: every free variable is a `Pool`, `Deal`, `MultiDeal` or compound keep whose source is the core's. A `MultiDeal`'s counts and
//! size are tuples; only [`evaluate_host`] takes them (the facade never names a built-in for one).
//!
//! The facade's `MultisetEvaluatorBase.evaluate` keeps its argument conversion, `_prepare` and the outer
//! `Die(final_data)`; for each prepared evaluation over core sources it calls in here instead of
//! `dungeon.evaluate(quest, sources, kwargs)`, and builds `icepool.Die(final_outcomes, final_weights)` from what
//! comes back (as `Quest.finalize_evaluation` does). Three ways in:
//!
//! - [`evaluate_builtin`]: the built-in evaluators the core has (`sum_evaluator`, `size_evaluator`, poker, keep,
//!   comparisons...), named by the facade with their parameters. Everything runs in Rust.
//! - [`evaluate_host`]: any other `MultisetEvaluator` (user subclasses included). The core driver pops the sources
//!   and memoises rooms; `next_state` / `initial_state` / `extra_outcomes` / `final_outcome` are host callbacks,
//!   and states are Python objects (native `Value`s where the core models the type, `Value::Host` otherwise).
//! - [`evaluate_tree`]: a `@multiset_function` evaluation, traced by the facade and described as a call tree
//!   (evaluator_rust.py `rust_call_tree`): the core drives every call's expressions and the function's main state
//!   (`icepool_core::evaluator::multiset_function`); the evaluators called in the body are built-ins (natively) or
//!   host callbacks, constant results included.
//!
//! Rooms are cached per call (ANALYSIS.md decision 5), not on the dungeon across calls as Python does.

use std::cell::RefCell;

use icepool_core::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, ProductOfCountsEvaluator, SizeEvaluator, SumEvaluator,
};
use icepool_core::evaluator::comparison::{Comparison, ComparisonEvaluator};
use icepool_core::evaluator::keep::KeepEvaluator;
use icepool_core::evaluator::multiset_function::{
    Call, DynEvaluator, Erased, MultisetFunction, Tupleize,
};
use icepool_core::evaluator::poker::{
    AllCountsEvaluator, AllStraightsEvaluator, CountSubsetEvaluator,
    HighestOutcomeAndCountEvaluator, LargestCountAndOutcomeEvaluator, LargestCountEvaluator,
    LargestStraightAndOutcomeEvaluator, LargestStraightEvaluator,
};
use icepool_core::evaluator::{evaluate_call_tree, evaluate_expressions, Final, MultisetEvaluator};
use icepool_core::expression::{free_variable, CallTree};
use icepool_core::order::Order;
use icepool_core::pool::{Count, MultisetSource};
use icepool_core::{Error, Quantity, Value};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple, PyType};
use pyo3::IntoPyObjectExt;

use crate::convert::{from_py, host_error, quantity_to_py, to_py, to_py_err};
use crate::expression::{core_source, flat};

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

/// The core sources of the facade's `PoolSource`s, `DealSource`s, `MultiDealSource`s and `CompoundKeepSource`s
/// (their `_rs`).
fn sources(sources: &[Bound<'_, PyAny>]) -> PyResult<Vec<MultisetSource<Value>>> {
    sources.iter().map(core_source).collect()
}

/// The prepared inputs: one flat per input as evaluator_rust.py describes it (see expression.rs).
type Inputs<'py> = Vec<Bound<'py, PyAny>>;

/// The evaluation over these sources: each input either the given flat or, without flats, one free variable per
/// source.
fn run_evaluation<E: MultisetEvaluator<Value>>(
    evaluator: &E,
    flats: Option<&Inputs<'_>>,
    srcs: &[Bound<'_, PyAny>],
    depth_budget: isize,
) -> PyResult<Vec<(E::Final, Quantity)>> {
    let srcs = sources(srcs)?;
    let flats = match flats {
        Some(flats) => flats.iter().map(flat).collect::<PyResult<Vec<_>>>()?,
        None => srcs.iter().map(|_| free_variable()).collect(),
    };
    evaluate_expressions(evaluator, &flats, &srcs, depth_check(depth_budget)).map_err(to_py_err)
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

/// Something to do with a built-in evaluator of the core, whatever its type.
trait BuiltinVisitor<'py> {
    type Out;
    fn visit<E>(self, evaluator: E) -> PyResult<Self::Out>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static;
}

/// The built-in evaluator of the core named by the facade (evaluator_rust.py `builtin_spec`) with its parameters,
/// handed to `visitor`.
fn with_builtin<'py, V: BuiltinVisitor<'py>>(
    name: &str,
    params: &[Bound<'py, PyAny>],
    visitor: V,
) -> PyResult<V::Out> {
    let param = |i: usize| -> PyResult<&Bound<'py, PyAny>> {
        params
            .get(i)
            .ok_or_else(|| PyValueError::new_err(format!("icepool_rs: {name} needs a parameter")))
    };
    let v = visitor;
    match name {
        "sum" => v.visit(SumEvaluator),
        "size" => v.visit(SizeEvaluator),
        "product_of_counts" => v.visit(ProductOfCountsEvaluator),
        "empty" => v.visit(EmptyEvaluator {
            any: param(0)?.extract()?,
        }),
        "expand" => v.visit(ExpandEvaluator {
            ascending: param(0)?.extract()?,
        }),
        "keep" => v.visit(KeepEvaluator {
            index: param(0)?.extract()?,
        }),
        "highest_outcome_and_count" => v.visit(HighestOutcomeAndCountEvaluator),
        "all_counts" => v.visit(AllCountsEvaluator {
            filter: from_py(param(0)?)?,
        }),
        "largest_count" => v.visit(LargestCountEvaluator),
        "largest_count_and_outcome" => v.visit(LargestCountAndOutcomeEvaluator),
        "count_subset" => v.visit(CountSubsetEvaluator {
            empty_divisor: from_py(param(0)?)?,
        }),
        "largest_straight" => v.visit(LargestStraightEvaluator),
        "largest_straight_and_outcome" => v.visit(LargestStraightAndOutcomeEvaluator {
            prioritize_highest: param(0)?.extract()?,
        }),
        "all_straights" => v.visit(AllStraightsEvaluator),
        "is_proper_subset" => v.visit(ComparisonEvaluator(Comparison::IsProperSubset)),
        "is_subset" => v.visit(ComparisonEvaluator(Comparison::IsSubset)),
        "is_proper_superset" => v.visit(ComparisonEvaluator(Comparison::IsProperSuperset)),
        "is_superset" => v.visit(ComparisonEvaluator(Comparison::IsSuperset)),
        "is_equal_set" => v.visit(ComparisonEvaluator(Comparison::IsEqualSet)),
        "is_not_equal_set" => v.visit(ComparisonEvaluator(Comparison::IsNotEqualSet)),
        "is_disjoint_set" => v.visit(ComparisonEvaluator(Comparison::IsDisjointSet)),
        _ => Err(PyValueError::new_err(format!(
            "icepool_rs: no built-in evaluator {name:?}"
        ))),
    }
}

/// Runs a built-in evaluation (see [`evaluate_builtin`]).
struct RunBuiltin<'a, 'py> {
    py: Python<'py>,
    flats: Option<&'a Inputs<'py>>,
    sources: &'a [Bound<'py, PyAny>],
    depth_budget: isize,
}

impl<'py> BuiltinVisitor<'py> for RunBuiltin<'_, 'py> {
    type Out = Finals<'py>;

    fn visit<E>(self, evaluator: E) -> PyResult<Finals<'py>>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static,
    {
        let result = run_evaluation(&evaluator, self.flats, self.sources, self.depth_budget)?;
        finals(self.py, result, |o| to_py(self.py, &o))
    }
}

/// A built-in evaluator of the core, named by the facade (evaluator_rust.py `builtin_spec`) with its parameters.
/// The facade only names one when the evaluator's type is exactly the built-in class, the number of inputs is the
/// one its `next_state` takes, and the keyword arguments are the ones it reads. `flats` describes inputs that are
/// multiset expressions (`None`: every input is one source).
#[pyfunction]
#[pyo3(signature = (name, params, sources, depth_budget, flats=None))]
pub fn evaluate_builtin<'py>(
    py: Python<'py>,
    name: &str,
    params: Vec<Bound<'py, PyAny>>,
    sources: Vec<Bound<'py, PyAny>>,
    depth_budget: isize,
    flats: Option<Inputs<'py>>,
) -> PyResult<Finals<'py>> {
    let run = RunBuiltin {
        py,
        flats: flats.as_ref(),
        sources: &sources,
        depth_budget,
    };
    with_builtin(name, &params, run)
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
    /// An evaluator calling back these bound methods (`kwargs` go to `initial_state` and `final_outcome`).
    fn new(
        next_state: Bound<'py, PyAny>,
        initial_state: Bound<'py, PyAny>,
        extra_outcomes: Bound<'py, PyAny>,
        final_outcome: Bound<'py, PyAny>,
        kwargs: Option<Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        let icepool = next_state.py().import("icepool")?;
        Ok(HostEvaluator {
            next_state,
            initial_state,
            extra_outcomes,
            final_outcome,
            kwargs,
            order_type: icepool.getattr("Order")?,
            reroll_types: icepool.getattr("REROLL_TYPES")?,
            unsupported_order: icepool.getattr("UnsupportedOrder")?.cast_into::<PyType>()?,
            outcomes: RefCell::new(None),
        })
    }

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
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<Vec<Bound<'py, PyAny>>> {
        let py = self.py();
        let mut args = vec![
            self.order(order)?,
            self.outcomes_tuple(outcomes)
                .map_err(host_error)?
                .into_any(),
        ];
        for s in sizes {
            args.push(match s {
                None => py.None().into_bound(py),
                Some(c) => count_py(py, c)?,
            });
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

/// A count or size as Python gets it: an `int`, or a tuple of `int`s for a `MultiDeal` input.
pub(crate) fn count_py<'py>(py: Python<'py>, c: &Count) -> icepool_core::Result<Bound<'py, PyAny>> {
    match c {
        Count::Int(c) => c.into_bound_py_any(py),
        Count::Tuple(cs) => PyTuple::new(py, cs.iter()).map(Bound::into_any),
    }
    .map_err(host_error)
}

fn int_counts(counts: &[i64]) -> Vec<Count> {
    counts.iter().map(|&c| Count::Int(c)).collect()
}

fn int_sizes(sizes: &[Option<i64>]) -> Vec<Option<Count>> {
    sizes.iter().map(|s| s.map(Count::Int)).collect()
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
        self.initial_state_counts(order, outcomes, &int_sizes(sizes))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> icepool_core::Result<Option<Value>> {
        self.next_state_counts(state, order, outcome, &int_counts(counts))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> icepool_core::Result<Final<Py<PyAny>>> {
        self.final_outcome_counts(final_state, order, outcomes, &int_sizes(sizes))
    }

    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> icepool_core::Result<Value> {
        // `quest.initial_state_main(order, outcomes, *arg_sizes, **kwargs)`
        let args = self.order_outcomes_sizes(order, outcomes, sizes)?;
        let state = self.call(&self.initial_state, args, self.kwargs.as_ref())?;
        from_py(&state).map_err(host_error)
    }

    fn next_state_counts(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> icepool_core::Result<Option<Value>> {
        // `self.next_state_main(prev_state_main, eval_order, outcome, *count_tree)`
        let py = self.py();
        let mut args = vec![
            to_py(py, state).map_err(host_error)?,
            self.order(order)?,
            to_py(py, outcome).map_err(host_error)?,
        ];
        for c in counts {
            args.push(count_py(py, c)?);
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

    fn final_outcome_counts(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
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

/// Any `MultisetEvaluator` over core sources, its methods called back in Python (see [`HostEvaluator`]). `flats`
/// as for [`evaluate_builtin`].
#[pyfunction]
#[pyo3(signature = (next_state, initial_state, extra_outcomes, final_outcome, kwargs, sources, depth_budget, flats=None))]
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
    flats: Option<Inputs<'py>>,
) -> PyResult<Finals<'py>> {
    let evaluator = HostEvaluator::new(
        next_state,
        initial_state,
        extra_outcomes,
        final_outcome,
        kwargs,
    )?;
    let result = run_evaluation(&evaluator, flats.as_ref(), &sources, depth_budget)?;
    finals(py, result, |o| Ok(o.into_bound(py)))
}

/// An evaluator called in a `@multiset_function`, type-erased, its final outcomes Python objects.
type Leaf<'py> = Box<dyn DynEvaluator<Value, Py<PyAny>> + 'py>;

/// One call of a `@multiset_function` evaluation: its call tree and its main side.
type PyCall<'py> = (CallTree<Value>, Call<'py, Value, Py<PyAny>>);

/// A built-in evaluator as a [`Leaf`].
struct BuiltinLeaf<'py>(Python<'py>);

impl<'py> BuiltinVisitor<'py> for BuiltinLeaf<'py> {
    type Out = Leaf<'py>;

    fn visit<E>(self, evaluator: E) -> PyResult<Leaf<'py>>
    where
        E: MultisetEvaluator<Value, Final = Value> + 'static,
    {
        let py = self.0;
        Ok(Box::new(Erased {
            evaluator,
            map_final: move |v: Value| to_py(py, &v).map(Bound::unbind).map_err(host_error),
        }))
    }
}

/// One call of a `@multiset_function` evaluation as evaluator_rust.py `_call_tree` describes it: `(kind, flats,
/// ...)` with kind `function` (one inner call), `joint` (a list of them), `builtin` (name, params) or `host` (the
/// dungeon's `next_state_main` and the quest's `initial_state_main`, `extra_outcomes`, `final_outcome`, kwargs).
fn call_tree<'py>(py: Python<'py>, desc: &Bound<'py, PyAny>) -> PyResult<PyCall<'py>> {
    let t = desc.cast::<PyTuple>()?;
    let kind: String = t.get_item(0)?.extract()?;
    let flats = t
        .get_item(1)?
        .try_iter()?
        .map(|f| flat(&f?))
        .collect::<PyResult<Vec<_>>>()?;
    let leaf = |call| Ok((CallTree::leaf(flats.clone()), Call::Evaluator(call)));
    match kind.as_str() {
        "function" => {
            let (inner_tree, inner) = call_tree(py, &t.get_item(2)?)?;
            let tree = CallTree {
                flats,
                calls: vec![inner_tree],
            };
            Ok((tree, Call::Function(Box::new(inner))))
        }
        "joint" => {
            let mut calls = Vec::new();
            let mut inners = Vec::new();
            for inner in t.get_item(2)?.try_iter()? {
                let (inner_tree, inner) = call_tree(py, &inner?)?;
                calls.push(inner_tree);
                inners.push(inner);
            }
            Ok((CallTree { flats, calls }, Call::Joint(inners)))
        }
        "builtin" => {
            let name: String = t.get_item(2)?.extract()?;
            let params: Vec<Bound<'py, PyAny>> = t.get_item(3)?.extract()?;
            leaf(with_builtin(&name, &params, BuiltinLeaf(py))?)
        }
        "host" => {
            let host = HostEvaluator::new(
                t.get_item(2)?,
                t.get_item(3)?,
                t.get_item(4)?,
                t.get_item(5)?,
                t.get_item(6)?.extract()?,
            )?;
            leaf(Box::new(Erased {
                evaluator: host,
                map_final: Ok,
            }))
        }
        _ => Err(PyValueError::new_err(format!(
            "icepool_rs: no call kind {kind:?}"
        ))),
    }
}

/// `icepool.tupleize(*final_outcomes)` of a joint `@multiset_function` result: `None` stays `None`, a reroll is
/// `icepool.Reroll`, and a reroll result drops the state.
fn host_tupleize(py: Python<'_>) -> PyResult<Box<Tupleize<'_, Py<PyAny>>>> {
    let icepool = py.import("icepool")?;
    let tupleize = icepool.getattr("tupleize")?;
    let reroll = icepool.getattr("Reroll")?;
    let reroll_types = icepool.getattr("REROLL_TYPES")?;
    Ok(Box::new(move |finals: Vec<Final<Py<PyAny>>>| {
        let args = finals
            .into_iter()
            .map(|f| match f {
                Final::None => py.None(),
                Final::Reroll => reroll.clone().unbind(),
                Final::Outcome(o) => o,
            })
            .collect::<Vec<_>>();
        let result = PyTuple::new(py, args)
            .and_then(|args| tupleize.call1(args))
            .map_err(host_error)?;
        Ok(if reroll_types.contains(&result).map_err(host_error)? {
            Final::Reroll
        } else if result.is_none() {
            Final::None
        } else {
            Final::Outcome(result.unbind())
        })
    }))
}

/// A `@multiset_function` evaluation (evaluator/multiset_function.py's dungeon and quest), described by
/// evaluator_rust.py `rust_call_tree`, over core sources: the core drives the expressions of every call and the
/// function's main state; the evaluators called in it are built-ins or host callbacks.
#[pyfunction]
pub fn evaluate_tree<'py>(
    py: Python<'py>,
    tree: Bound<'py, PyAny>,
    sources: Vec<Bound<'py, PyAny>>,
    depth_budget: isize,
) -> PyResult<Finals<'py>> {
    let (tree, call) = call_tree(py, &tree)?;
    let evaluator = MultisetFunction {
        call,
        tupleize: host_tupleize(py)?,
    };
    let srcs = self::sources(&sources)?;
    let result = evaluate_call_tree(&evaluator, &tree, &srcs, depth_check(depth_budget))
        .map_err(to_py_err)?;
    finals(py, result, |o| Ok(o.into_bound(py)))
}
