//! population/format.py, `Population.sample` and `Die.equals` / `hash_key` for a die of the core (`value/format.rs`,
//! `population.rs`).
//!
//! The facade keeps the argument juggling: `type(self).__name__`, the draw from Python's own `random` (the user
//! seeds it), and writing CSV through Python's `csv` module when `format` is given writer arguments.

use icepool_core::value::PyDie;
use icepool_core::format::{self as core_format, Formatted};
use icepool_core::value::format as value_format;
use icepool_core::host::LenKind;
use icepool_core::value::BinOp;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::types::PyString;

use crate::convert::{from_py, to_py, to_py_err};
use crate::RsDie;

/// `Population.format(format_spec)`: the text, or with `csv_table` and CSV output, `(headers, rows)` for the
/// facade's `csv.writer`.
#[pyfunction]
pub fn format<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    type_name: &str,
    format_spec: &str,
    csv_table: bool,
) -> PyResult<Bound<'py, PyAny>> {
    match core_format::format(&die.get().die, type_name, format_spec, csv_table)
        .map_err(to_py_err)?
    {
        Formatted::Text(s) => Ok(PyString::new(py, &s).into_any()),
        Formatted::CsvTable(headers, rows) => Ok((headers, rows).into_pyobject(py)?.into_any()),
    }
}

/// `format_probability_inverse(probability, int_start)`.
#[pyfunction]
pub fn format_probability_inverse(
    probability: &Bound<'_, PyAny>,
    int_start: &Bound<'_, PyAny>,
) -> PyResult<String> {
    value_format::format_probability_inverse(&from_py(probability)?, &from_py(int_start)?)
        .map_err(to_py_err)
}

/// `Population.common_outcome_length()`.
#[pyfunction]
pub fn common_outcome_length(die: &Bound<'_, RsDie>) -> PyResult<Option<usize>> {
    core_format::common_outcome_length(&die.get().die).map_err(to_py_err)
}

/// The outcome `Population.sample` picks for `r = random.randrange(self.denominator())`.
#[pyfunction]
pub fn sample_at<'py>(
    py: Python<'py>,
    die: &Bound<'py, RsDie>,
    r: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let r = crate::quantity(r)?;
    to_py(py, &die.get().die.sample_at(&r).map_err(to_py_err)?)
}

/// `Die.equals(other, simplify=simplify)` for two dice (the facade answers `self is other` and non-dice).
#[pyfunction]
pub fn die_equals(a: &Bound<'_, RsDie>, b: &Bound<'_, RsDie>, simplify: bool) -> PyResult<bool> {
    a.get()
        .die
        .equals(&b.get().die, simplify)
        .map_err(to_py_err)
}

/// What `common_outcome_length` asks of a Python outcome: `isinstance(x, Mapping)`, then `isinstance(x, Sized)`.
pub fn host_len_kind(obj: &Bound<'_, PyAny>) -> PyResult<LenKind> {
    let abc = obj.py().import("collections.abc")?;
    if obj.is_instance(&abc.getattr("Mapping")?)? {
        Ok(LenKind::Mapping)
    } else if obj.is_instance(&abc.getattr("Sized")?)? {
        Ok(LenKind::Len(obj.len()?))
    } else {
        Ok(LenKind::Unsized)
    }
}

/// The second element of a die's `hash_key` `(Die, rs)`: equal and ordered as `tuple(items)` is.
pub fn richcmp(die: &RsDie, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<Py<PyAny>> {
    let py = other.py();
    let Ok(other) = other.cast::<RsDie>() else {
        return Ok(py.NotImplemented());
    };
    let (a, b) = (&die.die, &other.get().die);
    let result = match op {
        CompareOp::Eq => icepool_core::Value::Bool(a == b),
        CompareOp::Ne => icepool_core::Value::Bool(a != b),
        CompareOp::Lt => a.hash_key_compare(b, BinOp::Lt).map_err(to_py_err)?,
        CompareOp::Le => a.hash_key_compare(b, BinOp::Le).map_err(to_py_err)?,
        CompareOp::Gt => a.hash_key_compare(b, BinOp::Gt).map_err(to_py_err)?,
        CompareOp::Ge => a.hash_key_compare(b, BinOp::Ge).map_err(to_py_err)?,
    };
    Ok(crate::convert::to_py(py, &result)?.unbind())
}
