//! population/base.py's quantities, probabilities and statistics for a die of the core (`population.rs`).
//!
//! The facade keeps each method's argument juggling (optional arguments, `star` inference, the selection made by a
//! Python callable) and calls in here for the work. Results are Python objects: ints, `Fraction`s, floats, the
//! outcomes themselves; methods that make a new population return the `dict` items Python passes to `Die(...)`
//! (or `None` where Python returns the population itself).

use icepool_core::population::{Comparison, Data};
use icepool_core::value::builtins::Index;
use icepool_core::{Error, Value};
use pyo3::prelude::*;
use pyo3::types::{PyList, PySlice, PyTuple};

use crate::convert::{from_py, host_error, quantity_to_py, to_py, to_py_err};
use crate::RsDie;

fn comparison(c: &Bound<'_, PyAny>) -> PyResult<Comparison> {
    Comparison::parse(&from_py(c)?).map_err(to_py_err)
}

fn value<'py>(py: Python<'py>, v: icepool_core::Result<Value>) -> PyResult<Bound<'py, PyAny>> {
    to_py(py, &v.map_err(to_py_err)?)
}

fn tuple<'py>(py: Python<'py>, values: &[Value]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        values
            .iter()
            .map(|v| to_py(py, v))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// `x[key]`'s key as the core takes it: a `slice` becomes `Index::Slice`, anything else an item.
pub fn index(key: &Bound<'_, PyAny>) -> PyResult<Index> {
    Ok(match key.cast::<PySlice>() {
        Ok(s) => Index::Slice(
            from_py(&s.getattr("start")?)?,
            from_py(&s.getattr("stop")?)?,
            from_py(&s.getattr("step")?)?,
        ),
        Err(_) => Index::Item(from_py(key)?),
    })
}

/// The Python key for an `Index` (for a host object's own `__getitem__`).
pub fn index_to_py<'py>(py: Python<'py>, key: &Index) -> PyResult<Bound<'py, PyAny>> {
    match key {
        Index::Item(v) => to_py(py, v),
        Index::Slice(a, b, c) => py.import("builtins")?.getattr("slice")?.call1((
            to_py(py, a)?,
            to_py(py, b)?,
            to_py(py, c)?,
        )),
    }
}

/// `dict` items for `Die(...)`, or `None` for the population itself.
pub(crate) fn data<'py>(
    py: Python<'py>,
    d: icepool_core::Result<Option<Data>>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    let Some(d) = d.map_err(to_py_err)? else {
        return Ok(None);
    };
    let items = d
        .iter()
        .map(|(o, q)| PyTuple::new(py, [to_py(py, o)?, to_py(py, q)?]))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(Some(PyList::new(py, items)?))
}

/// `quantities(comparison)` for a comparison other than `'=='` (which is the die's own values view).
#[pyfunction]
pub fn quantities<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    comparison: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyTuple>> {
    let qs = die.get().die.quantities(self::comparison(comparison)?);
    PyTuple::new(
        py,
        qs.iter()
            .map(|q| quantity_to_py(py, q))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// `quantity(comparison, outcome)` once the facade has settled the one-argument form.
#[pyfunction]
pub fn quantity<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    comparison: &Bound<'py, PyAny>,
    outcome: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let c = self::comparison(comparison)?;
    let q = die
        .get()
        .die
        .quantity(c, &from_py(outcome)?)
        .map_err(to_py_err)?;
    quantity_to_py(py, &q)
}

/// `probability(comparison, outcome, percent=percent)`.
#[pyfunction]
pub fn probability<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    comparison: &Bound<'py, PyAny>,
    outcome: &Bound<'py, PyAny>,
    percent: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let c = self::comparison(comparison)?;
    value(
        py,
        die.get().die.probability(c, &from_py(outcome)?, percent),
    )
}

/// `probabilities(comparison, percent=percent)`.
#[pyfunction]
pub fn probabilities<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    comparison: &Bound<'py, PyAny>,
    percent: bool,
) -> PyResult<Bound<'py, PyTuple>> {
    let c = self::comparison(comparison)?;
    let ps = die.get().die.probabilities(c, percent).map_err(to_py_err)?;
    tuple(py, &ps)
}

/// `nearest(comparison, outcome)`: `None` if no outcome fits.
#[pyfunction]
pub fn nearest<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    comparison: &Bound<'py, PyAny>,
    outcome: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let c = self::comparison(comparison)?;
    match die
        .get()
        .die
        .nearest(c, &from_py(outcome)?)
        .map_err(to_py_err)?
    {
        Some(o) => to_py(py, &o),
        None => Ok(py.None().into_bound(py)),
    }
}

/// `quantity_where(which)`, the facade having called `which` on each outcome.
#[pyfunction]
pub fn quantity_where<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    selected: Vec<bool>,
) -> PyResult<Bound<'py, PyAny>> {
    quantity_to_py(py, &die.get().die.quantity_where(&selected))
}

/// `probability_where(which, percent=percent)`, the facade having called `which` on each outcome.
#[pyfunction]
pub fn probability_where<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    selected: Vec<bool>,
    percent: bool,
) -> PyResult<Bound<'py, PyAny>> {
    value(py, die.get().die.probability_where(&selected, percent))
}

#[pyfunction]
pub fn mode<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<Bound<'py, PyTuple>> {
    tuple(py, &die.get().die.mode())
}

#[pyfunction]
pub fn modal_quantity<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
) -> PyResult<Bound<'py, PyAny>> {
    quantity_to_py(py, &die.get().die.modal_quantity().map_err(to_py_err)?)
}

#[pyfunction]
pub fn denominator<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<Bound<'py, PyAny>> {
    quantity_to_py(py, &die.get().die.denominator())
}

#[pyfunction]
pub fn is_empty(die: &Bound<'_, RsDie>) -> bool {
    die.get().die.is_empty()
}

#[pyfunction]
pub fn min_outcome<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<Bound<'py, PyAny>> {
    value(py, die.get().die.min_outcome().cloned())
}

#[pyfunction]
pub fn max_outcome<'py>(py: Python<'py>, die: &Bound<'py, RsDie>) -> PyResult<Bound<'py, PyAny>> {
    value(py, die.get().die.max_outcome().cloned())
}

/// `kolmogorov_smirnov(other)` (`cramer` false) or `cramer_von_mises(other)` against another die.
#[pyfunction]
pub fn cdf_distance<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    other: &Bound<'py, RsDie>,
    cramer: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let (a, b) = (&die.get().die, &other.get().die);
    value(
        py,
        if cramer {
            a.cramer_von_mises(b)
        } else {
            a.kolmogorov_smirnov(b)
        },
    )
}

/// `quantile(n, d)`, or with `tie` `'low'` / `'high'`, `quantile_low` / `quantile_high`.
#[pyfunction]
pub fn quantile<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    n: &Bound<'py, PyAny>,
    d: &Bound<'py, PyAny>,
    tie: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let (die, n, d) = (&die.get().die, from_py(n)?, from_py(d)?);
    value(
        py,
        match tie {
            "low" => die.quantile_low(&n, &d),
            "high" => die.quantile_high(&n, &d),
            _ => die.quantile(&n, &d),
        },
    )
}

/// `mean`, `variance`, `standard_deviation`, `skewness` or `excess_kurtosis`.
#[pyfunction]
pub fn statistic<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let die = &die.get().die;
    value(
        py,
        match name {
            "mean" => die.mean(),
            "variance" => die.variance(),
            "standard_deviation" => die.standard_deviation(),
            "skewness" => die.skewness(),
            "excess_kurtosis" => die.excess_kurtosis(),
            _ => Err(Error::Unsupported(format!("statistic {name}"))),
        },
    )
}

#[pyfunction]
pub fn standardized_moment<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    k: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    value(py, die.get().die.standardized_moment(&from_py(k)?))
}

#[pyfunction]
pub fn entropy<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    base: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    value(py, die.get().die.entropy(&from_py(base)?))
}

/// `len(die.marginals)`.
#[pyfunction]
pub fn marginals_len(die: &Bound<'_, RsDie>) -> PyResult<usize> {
    die.get().die.marginals_len().map_err(to_py_err)
}

/// `covariance(i, j)` (`correlation` false) or `correlation(i, j)`.
#[pyfunction]
pub fn joint<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    i: &Bound<'py, PyAny>,
    j: &Bound<'py, PyAny>,
    correlation: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let (die, i, j) = (&die.get().die, index(i)?, index(j)?);
    value(
        py,
        if correlation {
            die.correlation(&i, &j)
        } else {
            die.covariance(&i, &j)
        },
    )
}

/// `group_by(key_map)`: `key_function` (the facade's, star and mapping handled) is called on each outcome in
/// order, and each key enters the core at once (so an unhashable key raises before the next call). Returns
/// `[(key, die), ...]` in first-seen order.
#[pyfunction]
pub fn group_by<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    key_function: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let die = &die.get().die;
    let keys = die
        .outcomes()
        .map(|o| from_py(&key_function.call1((to_py(py, o)?,))?))
        .collect::<PyResult<Vec<_>>>()?;
    let groups = die.group_by(keys).map_err(to_py_err)?;
    let items = groups
        .into_iter()
        .map(|(k, d)| {
            let d = Bound::new(py, RsDie { die: d })?.into_any();
            PyTuple::new(py, [to_py(py, &k)?, d])
        })
        .collect::<PyResult<Vec<_>>>()?;
    PyList::new(py, items)
}

/// `multiply_quantities` / `divide_quantities` / `modulo_quantities` (by `method`) of `x`.
#[pyfunction]
pub fn quantities_op<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    method: &str,
    x: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    let (die, x) = (&die.get().die, from_py(x)?);
    data(
        py,
        match method {
            "multiply" => die.multiply_quantities(&x),
            "divide" => die.divide_quantities(&x),
            "modulo" => die.modulo_quantities(&x).map(Some),
            _ => Err(Error::Unsupported(format!("{method}_quantities"))),
        },
    )
}

#[pyfunction]
pub fn pad_to_denominator<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    denominator: &Bound<'py, PyAny>,
    outcome: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    let d = die
        .get()
        .die
        .pad_to_denominator(&from_py(denominator)?, &from_py(outcome)?);
    data(py, d.map(Some))
}

#[pyfunction]
pub fn multiply_to_denominator<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    denominator: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    data(
        py,
        die.get()
            .die
            .multiply_to_denominator(&from_py(denominator)?),
    )
}

/// The `HostObject` hooks the statistics need, for a Python object.
pub fn host_to_float(obj: &Bound<'_, PyAny>) -> icepool_core::Result<f64> {
    obj.extract::<f64>().map_err(host_error)
}

pub fn host_as_rational(obj: &Bound<'_, PyAny>) -> icepool_core::Result<Option<dashu_ratio::RBig>> {
    (|| {
        let py = obj.py();
        let rational = py.import("numbers")?.getattr("Rational")?;
        if !obj.is_instance(&rational)? {
            return Ok(None);
        }
        let n = crate::convert::int_from_py(&obj.getattr("numerator")?)?;
        let d = crate::convert::int_from_py(&obj.getattr("denominator")?)?;
        if d == dashu_int::IBig::ZERO {
            return Err(pyo3::exceptions::PyZeroDivisionError::new_err(
                "Fraction(n, 0)",
            ));
        }
        Ok(Some(
            dashu_ratio::RBig::from(n) / dashu_ratio::RBig::from(d),
        ))
    })()
    .map_err(host_error)
}

pub fn host_as_index(obj: &Bound<'_, PyAny>) -> icepool_core::Result<Option<dashu_int::IBig>> {
    (|| {
        if !obj.hasattr("__index__")? {
            return Ok(None);
        }
        let i = obj
            .py()
            .import("operator")?
            .getattr("index")?
            .call1((obj,))?;
        Ok(Some(crate::convert::int_from_py(&i)?))
    })()
    .map_err(host_error)
}

pub fn host_getitem(obj: &Bound<'_, PyAny>, key: &Index) -> icepool_core::Result<Value> {
    (|| from_py(&obj.get_item(index_to_py(obj.py(), key)?)?))().map_err(host_error)
}

pub fn host_len(obj: &Bound<'_, PyAny>) -> icepool_core::Result<usize> {
    obj.len().map_err(host_error)
}
