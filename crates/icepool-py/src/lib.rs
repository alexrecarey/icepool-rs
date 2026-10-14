//! `icepool_rs`: the Rust core as a Python extension, for the facade in python/icepool (see its `backend.py`).
//!
//! The facade keeps Python's argument juggling and every dunder trick (`itemize`, `Again`, `DieWithTruth`...) and
//! calls in here for the work: a `Die` built in Rust is held by the facade's `Die` as `_rs`, and its items become
//! the facade's `Counts`.

mod convert;
mod ops;

use dashu_int::UBig;
use icepool_core::{Die as CoreDie, DieArg, Error, Quantity, Value};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use convert::{from_py, int_from_py, quantity_to_py, to_py, to_py_err};

/// A die of the Rust core.
#[pyclass(frozen, name = "Die", module = "icepool_rs")]
pub struct RsDie {
    die: CoreDie<Value>,
}

#[pymethods]
impl RsDie {
    /// The `(outcome, quantity)` pairs in sorted order, zero quantities included: the facade's `Counts` items.
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let items = self
            .die
            .items()
            .iter()
            .map(|(o, q)| PyTuple::new(py, [to_py(py, o)?, quantity_to_py(py, q)?]))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, items)
    }

    fn __len__(&self) -> usize {
        self.die.len()
    }
}

impl RsDie {
    /// Wraps a core result for Python. `Counts` rejects `Reroll` and `Restart` as keys (they are host objects to
    /// the core, which only rejects `None`): so does this.
    fn wrap(py: Python<'_>, die: icepool_core::Result<CoreDie<Value>>) -> PyResult<RsDie> {
        let die = die.map_err(to_py_err)?;
        let reroll_types = py.import("icepool")?.getattr("REROLL_TYPES")?;
        for (o, _) in die.items() {
            if let Value::Host(_) = o {
                let obj = to_py(py, o)?;
                if reroll_types.contains(&obj)? {
                    return Err(PyTypeError::new_err(format!(
                        "{} is not a valid key.",
                        obj.str()?
                    )));
                }
            }
        }
        Ok(RsDie { die })
    }
}

/// One element of `Die(outcomes)` after `creation_args.expand_arg`, as the facade passes it:
/// `(0, outcome)`, `(1, rs_die)`, `(2, None)` for `Reroll`, or `(3, items)` for another population (a `Deck`).
fn die_arg(arg: &Bound<'_, PyAny>) -> PyResult<DieArg<Value>> {
    let (kind, payload): (u8, Bound<'_, PyAny>) = arg.extract()?;
    Ok(match kind {
        0 => DieArg::Outcome(from_py(&payload)?),
        1 => DieArg::Die(payload.cast::<RsDie>()?.get().die.clone()),
        2 => DieArg::Reroll,
        _ => {
            let items = payload
                .try_iter()?
                .map(|item| {
                    let (o, q): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
                    Ok((from_py(&o)?, quantity(&q)?))
                })
                .collect::<PyResult<Vec<_>>>()?;
            DieArg::Die(CoreDie::from_counts(
                icepool_core::counts::Counts::new(items).map_err(to_py_err)?,
            ))
        }
    })
}

/// A non-negative Python `int` as a quantity.
fn quantity(q: &Bound<'_, PyAny>) -> PyResult<Quantity> {
    UBig::try_from(int_from_py(q)?)
        .map_err(|_| pyo3::exceptions::PyValueError::new_err("negative quantity"))
}

/// One element of `times` as the weight `merge_weights_lcm` gives it. `itemize` has already rejected negatives.
/// An `int` (or `bool`) is the weight. Another number only survives if Python never passes it to `math.gcd`:
/// when it is zero, or the argument's denominator is 0 (an empty die, `Reroll`); either way its scale factor is
/// 0 and it contributes nothing. Otherwise `math.gcd` raises `TypeError`.
fn weight(arg: &DieArg<Value>, t: &Bound<'_, PyAny>) -> PyResult<Quantity> {
    if t.is_instance_of::<pyo3::types::PyInt>() {
        return quantity(t);
    }
    let empty = match arg {
        DieArg::Die(d) => d.denominator() == Quantity::ZERO,
        DieArg::Reroll => true,
        DieArg::Outcome(_) => false,
    };
    if empty || t.eq(0)? {
        return Ok(Quantity::ZERO);
    }
    Err(PyTypeError::new_err(format!(
        "'{}' object cannot be interpreted as an integer",
        t.get_type().name()?
    )))
}

/// `creation_args.expand_args_for_die(args, times)`: mixes the arguments with `merge_weights_lcm` and sorts the
/// result into a die. `args` as described at `die_arg`.
#[pyfunction]
fn expand_args_for_die(
    py: Python<'_>,
    args: Vec<Bound<'_, PyAny>>,
    times: Vec<Bound<'_, PyAny>>,
) -> PyResult<RsDie> {
    let args = args.iter().map(die_arg).collect::<PyResult<Vec<_>>>()?;
    let weights = args
        .iter()
        .zip(&times)
        .map(|(a, t)| weight(a, t))
        .collect::<PyResult<Vec<_>>>()?;
    if args.len() != weights.len() {
        return Err(to_py_err(Error::Value("one weight per argument".into())));
    }
    // `Die::new` returns a single die with weight 1 as-is; `Die.__new__` handled that case already (comparing
    // `times[0] == 1` as Python does), so only the merge is left.
    let counts = icepool_core::creation_args::expand_args_for_die(&args, &weights);
    RsDie::wrap(py, counts.map(CoreDie::from_counts))
}

/// A die from the items of a facade `Counts` (already sorted and merged, by Python or by Rust).
#[pyfunction]
fn from_counts_items(items: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)>) -> PyResult<RsDie> {
    let items = items
        .iter()
        .map(|(o, q)| Ok((from_py(o)?, quantity(q)?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(RsDie {
        die: CoreDie::from_counts(icepool_core::counts::Counts::from_sorted(items)),
    })
}

/// `Population._unary_operator(op, *args, **kwargs)` for a die.
#[pyfunction]
#[pyo3(signature = (die, op, *args, **kwargs))]
fn unary_operator(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    op: &Bound<'_, PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<RsDie> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::unary(
        op,
        if kwargs.is_some() {
            usize::MAX
        } else {
            args.len()
        },
    )?;
    RsDie::wrap(
        py,
        die.get().die.unary_operator(|a| op.apply(a, args, kwargs)),
    )
}

/// `Die.binary_operator(other, op, *args, **kwargs)`.
#[pyfunction]
#[pyo3(signature = (die, other, op, *args, **kwargs))]
fn binary_operator(
    py: Python<'_>,
    die: &Bound<'_, RsDie>,
    other: &Bound<'_, RsDie>,
    op: &Bound<'_, PyAny>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<RsDie> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::binary(op, args.is_empty() && kwargs.is_none())?;
    let result = die
        .get()
        .die
        .binary_operator(&other.get().die, |a, b| op.apply(a, b, args, kwargs));
    RsDie::wrap(py, result)
}

#[pymodule]
fn icepool_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RsDie>()?;
    m.add_function(wrap_pyfunction!(expand_args_for_die, m)?)?;
    m.add_function(wrap_pyfunction!(from_counts_items, m)?)?;
    m.add_function(wrap_pyfunction!(unary_operator, m)?)?;
    m.add_function(wrap_pyfunction!(binary_operator, m)?)?;
    Ok(())
}
