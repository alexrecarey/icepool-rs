//! population/again.py on the core: `evaluate_agains_using_depth` and `evaluate_agains_using_count`.
//!
//! The facade keeps `AgainExpression` itself (its dunders, `is_additive`, `hash_key`) and the argument juggling of
//! the two functions (the checks on `again_depth` / `again_count`, the default `again_end`, `compute_not_again_die`,
//! `Die([again_end])`). Here each `AgainExpression` among the itemized outcomes becomes the core's `Again` tree
//! (reading its `_func`, `_args` and `is_additive`), over Python objects: its functions (`operator.add`, any
//! callable) are host callbacks, and so is `icepool.Die(objects, times)`, the facade's die constructor (whose
//! mixing is the core's). Depth mode is the core's generic loop; count mode builds its step die the same way and
//! then runs the core's Markov chain on `Die<Value>`.

use std::sync::Arc;

use icepool_core::again::{self, Again, AgainArg, StepOutcome};
use icepool_core::{Die as CoreDie, DieArg, Error, Quantity, Value};
use pyo3::exceptions::{PyNotImplementedError, PyTypeError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyInt, PyList, PyTuple, PyType};

use crate::convert::{from_py, to_py_err};
use crate::{matmul_core, RsDie};

/// A Python object in the tree (`Arc`ed: the core clones objects, and cloning a `Py` needs the GIL).
type Obj = Arc<Py<PyAny>>;
type Arg = AgainArg<Py<PyAny>, Obj>;

static AGAIN_EXPRESSION: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// A Python exception passing through the core's generic functions, which want an error type they can convert a
/// core `Error` into.
pub(crate) struct HostErr(pub(crate) PyErr);

impl From<Error> for HostErr {
    fn from(e: Error) -> Self {
        HostErr(to_py_err(e))
    }
}

impl From<PyErr> for HostErr {
    fn from(e: PyErr) -> Self {
        HostErr(e)
    }
}

/// An itemized outcome: an `AgainExpression` becomes the core tree, anything else is passed as-is.
fn arg(x: &Bound<'_, PyAny>, cls: &Bound<'_, PyType>) -> PyResult<Arg> {
    if !x.is_instance(cls)? {
        return Ok(AgainArg::Other(Arc::new(x.clone().unbind())));
    }
    let function = x.getattr("_func")?;
    // No function: the expression represents the again roll directly.
    if function.is_none() {
        return Ok(AgainArg::Again(Arc::new(Again::Again)));
    }
    let args = x
        .getattr("_args")?
        .try_iter()?
        .map(|a| arg(&a?, cls))
        .collect::<PyResult<_>>()?;
    Ok(AgainArg::Again(Arc::new(Again::Apply {
        function: function.unbind(),
        args,
        is_additive: x.getattr("is_additive")?.is_truthy()?,
    })))
}

fn args(py: Python<'_>, outcomes: &[Bound<'_, PyAny>]) -> PyResult<Vec<Arg>> {
    let cls = AGAIN_EXPRESSION.import(py, "icepool.population.again", "AgainExpression")?;
    outcomes.iter().map(|x| arg(x, cls)).collect()
}

/// `self._func(*args)`, args already evaluated.
fn call(py: Python<'_>, function: &Py<PyAny>, args: Vec<Obj>) -> Result<Obj, HostErr> {
    let args = PyTuple::new(py, args.iter().map(|a| a.bind(py)))?;
    Ok(Arc::new(function.bind(py).call1(args)?.unbind()))
}

/// `icepool.Die(objects, times)`.
fn die<'py>(
    py: Python<'py>,
    objects: Vec<Obj>,
    times: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("icepool")?
        .getattr("Die")?
        .call1((PyList::new(py, objects.iter().map(|o| o.bind(py)))?, times))
}

/// `evaluate_agains_using_depth(outcomes, times, again_depth, again_end)` after the facade has checked
/// `again_depth >= 0`, replaced a `None` `again_end` with the zero outcome and rejected `Restart`. Returns the
/// facade's die.
#[pyfunction]
pub fn evaluate_agains_using_depth<'py>(
    py: Python<'py>,
    outcomes: Vec<Bound<'py, PyAny>>,
    times: Bound<'py, PyAny>,
    again_depth: Bound<'py, PyAny>,
    again_end: Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let outcomes = args(py, &outcomes)?;
    let reroll = py.import("icepool")?.getattr("Reroll")?;
    // `again_end is icepool.Reroll`: every expression becomes `Reroll` at the innermost level.
    let again_end = (!again_end.is(&reroll)).then(|| Arc::new(again_end.unbind()));
    let run = |depth: u64| {
        again::evaluate_agains_using_depth(
            &outcomes,
            depth,
            again_end.as_ref(),
            &Arc::new(reroll.clone().unbind()),
            |objects| Ok::<_, HostErr>(Arc::new(die(py, objects, &times)?.unbind())),
            |f, a| call(py, f, a),
        )
        .map_err(|e| e.0)
    };
    // `for _ in range(again_depth)` runs after the innermost level is built, so a non-int depth raises then.
    if !again_depth.is_instance_of::<PyInt>() {
        run(0)?;
        return Err(PyTypeError::new_err(format!(
            "'{}' object cannot be interpreted as an integer",
            again_depth.get_type().name()?
        )));
    }
    Ok(run(again_depth.extract()?)?.bind(py).clone())
}

/// `evaluate_agains_using_count(outcomes, times, again_count, again_end)` after the facade has checked
/// `again_count >= 0` and computed `not_again_die`, its `zero` outcome and `again_end_die` (`None` for
/// `Restart`).
#[pyfunction]
pub fn evaluate_agains_using_count<'py>(
    py: Python<'py>,
    outcomes: Vec<Bound<'py, PyAny>>,
    times: Bound<'py, PyAny>,
    again_count: Bound<'py, PyAny>,
    zero: Bound<'py, PyAny>,
    not_again_die: &Bound<'py, RsDie>,
    again_end_die: Option<&Bound<'py, RsDie>>,
) -> PyResult<RsDie> {
    let cls = AGAIN_EXPRESSION.import(py, "icepool.population.again", "AgainExpression")?;
    for x in &outcomes {
        // The core's bare `Again` is always additive; one made as `AgainExpression()` is not.
        if x.is_instance(cls)?
            && x.getattr("_func")?.is_none()
            && !x.getattr("is_additive")?.is_truthy()?
        {
            return Err(PyNotImplementedError::new_err(
                "icepool_rs: again_count with a bare AgainExpression that is not additive",
            ));
        }
    }
    let outcomes = args(py, &outcomes)?;
    let icepool = py.import("icepool")?;
    let tupleize = icepool.getattr("tupleize")?;
    let int = |i: i64| -> PyResult<Obj> { Ok(Arc::new(i.into_pyobject(py)?.into_any().unbind())) };
    let step_objects = again::step_outcomes(
        &outcomes,
        &Arc::new(zero.clone().unbind()),
        &int(1)?,
        &int(0)?,
        |f, a| call(py, f, a),
        // `icepool.tupleize(outcome._evaluate(zero), 0, outcome._again_count() - 1)`, or `(zero, 1, -1)`.
        |step| {
            let t = match step {
                StepOutcome::NotAgain => tupleize.call1((&zero, 1, -1))?,
                StepOutcome::Again { flat, count } => {
                    tupleize.call1((flat.bind(py), 0, count.bind(py).sub(1)?))?
                }
            };
            Ok::<_, HostErr>(Arc::new(t.unbind()))
        },
    )
    .map_err(|e| e.0)?;
    let step_die = die(py, step_objects, &times)?.call_method0("_rust_die")?;
    let step_die = step_die.cast::<RsDie>()?.get().die.clone();
    // `repeat=again_count + 1` reaches `range(repeat)`.
    if !again_count.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "'{}' object cannot be interpreted as an integer",
            again_count.get_type().name()?
        )));
    }
    let not_again_die = &not_again_die.get().die;
    let end = again_end_die.map(|d| d.get().die.clone());
    // `n @ die` with an int `n` is `Die([n]) @ die` (`Die.__rmatmul__`).
    let matmul = |n: &Value, die: &CoreDie<Value>| {
        let n = CoreDie::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
        Ok::<_, HostErr>(matmul_core(py, n, die)?)
    };
    let result = again::evaluate_agains_using_count(
        &step_die,
        not_again_die,
        &from_py(&zero)?,
        again_count.extract()?,
        end.as_ref(),
        matmul,
    )
    .map_err(|e| e.0)?;
    RsDie::wrap(py, Ok(result))
}
