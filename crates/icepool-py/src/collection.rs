//! icepool's `Vector` and `Symbols` (collection/vector.py, collection/symbols.py) on the core.
//!
//! The facade's classes keep their Python API, their `_data` and `_truth_value` attributes, the container protocol
//! and their argument juggling (`NotImplemented` for populations, `Symbols(other)` conversions, `isinstance` checks).
//! The elementwise and multiset work runs here, on the core's `Value::Vector` / `Value::Symbols`. Python callables
//! given to `Vector.binary_operator` / `unary_operator` are host callbacks, applied to each element as Python does.

use icepool_core::value::{BinOp, Symbols, UnOp, Vector};
use icepool_core::{Error, Value};
use pyo3::exceptions::{PyNotImplementedError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::convert::{from_py, to_py, to_py_err};
use crate::ops;

fn vector(v: &Bound<'_, PyAny>) -> PyResult<Vector> {
    match from_py(v)? {
        Value::Vector(v) => Ok(v),
        _ => Err(PyTypeError::new_err("icepool_rs: not a Vector")),
    }
}

fn symbols(s: &Bound<'_, PyAny>) -> PyResult<Symbols> {
    match from_py(s)? {
        Value::Symbols(s) => Ok(s),
        _ => Err(PyNotImplementedError::new_err(
            "icepool_rs: a Symbols whose keys are not all single characters",
        )),
    }
}

fn core<T>(r: icepool_core::Result<T>) -> PyResult<T> {
    r.map_err(to_py_err)
}

/// `Vector.binary_operator(other, op, *args, compare_for_truth=..., compare_non_vector=..., **kwargs)` once the
/// facade has returned `NotImplemented` for a population.
#[pyfunction]
#[pyo3(signature = (v, other, op, args, kwargs, compare_for_truth, compare_non_vector))]
#[allow(clippy::too_many_arguments)]
pub fn vector_binary_operator<'py>(
    py: Python<'py>,
    v: &Bound<'py, PyAny>,
    other: &Bound<'py, PyAny>,
    op: &Bound<'py, PyAny>,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
    compare_for_truth: bool,
    compare_non_vector: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::binary(op, args.is_empty() && kwargs.is_none())?;
    let compare_non_vector = if compare_non_vector.is_none() {
        None
    } else {
        Some(from_py(compare_non_vector)?)
    };
    let result = vector(v)?.binary_operator(
        &from_py(other)?,
        |x, y| op.apply(x, y, args, kwargs),
        compare_for_truth,
        compare_non_vector,
    );
    to_py(py, &core(result)?)
}

/// `Vector.reverse_binary_operator(other, op, *args, **kwargs)`.
#[pyfunction]
pub fn vector_reverse_binary_operator<'py>(
    py: Python<'py>,
    v: &Bound<'py, PyAny>,
    other: &Bound<'py, PyAny>,
    op: &Bound<'py, PyAny>,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let op = ops::binary(op, args.is_empty() && kwargs.is_none())?;
    let result =
        vector(v)?.reverse_binary_operator(&from_py(other)?, |x, y| op.apply(x, y, args, kwargs));
    to_py(py, &core(result)?)
}

/// `Vector.unary_operator(op, *args, **kwargs)`: elementwise.
#[pyfunction]
pub fn vector_unary_operator<'py>(
    py: Python<'py>,
    v: &Bound<'py, PyAny>,
    op: &Bound<'py, PyAny>,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = kwargs.filter(|k| !k.is_empty());
    let nargs = if kwargs.is_some() {
        usize::MAX
    } else {
        args.len()
    };
    let op = ops::unary(op, args, nargs)?;
    let result = vector(v)?.map(|x| op.apply(x, args, kwargs));
    to_py(py, &core(result)?)
}

/// A `Symbols`' `_data` items, for the facade's `defaultdict(int, items)`.
fn items<'py>(py: Python<'py>, s: &Symbols) -> PyResult<Bound<'py, PyList>> {
    let items = s
        .items()
        .iter()
        .map(|(c, n)| {
            PyTuple::new(
                py,
                [c.to_string().into_pyobject(py)?.into_any(), to_py(py, n)?],
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    PyList::new(py, items)
}

/// `Symbols(symbols)` for a string: negative symbols after a dash. The sorted `_data` items.
#[pyfunction]
pub fn symbols_from_str<'py>(py: Python<'py>, s: &str) -> PyResult<Bound<'py, PyList>> {
    items(py, &core(Symbols::parse(s))?)
}

/// `Symbols(symbols)` for an iterable of characters (given as a tuple).
#[pyfunction]
pub fn symbols_from_iter<'py>(
    py: Python<'py>,
    elements: Vec<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let elements = elements.iter().map(from_py).collect::<PyResult<Vec<_>>>()?;
    items(py, &core(Symbols::from_elements(&elements))?)
}

/// `Symbols(symbols)` for a mapping (given as its items).
#[pyfunction]
pub fn symbols_from_mapping<'py>(
    py: Python<'py>,
    mapping: Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>,
) -> PyResult<Bound<'py, PyList>> {
    let mapping = mapping
        .iter()
        .map(|(k, v)| Ok((from_py(k)?, from_py(v)?)))
        .collect::<PyResult<Vec<_>>>()?;
    items(py, &core(Symbols::from_mapping(mapping))?)
}

/// A binary operation of `Symbols` once the facade has juggled the arguments: for everything but `mul`, `mod` and
/// `divide_counts` the other operand is already a `Symbols`.
#[pyfunction]
pub fn symbols_op<'py>(
    py: Python<'py>,
    name: &str,
    a: &Bound<'py, PyAny>,
    b: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let s = symbols(a)?;
    let bv = from_py(b)?;
    let other = || match &bv {
        Value::Symbols(o) => Ok(o),
        _ => Err(PyTypeError::new_err("icepool_rs: not a Symbols")),
    };
    let bool_value = |b: bool| Value::Bool(b);
    let op = |op: BinOp| -> PyResult<Value> {
        match s.binary(op, &bv) {
            Some(r) => core(r),
            None => Err(PyTypeError::new_err("icepool_rs: not a Symbols operation")),
        }
    };
    let result = match name {
        "add" => op(BinOp::Add)?,
        "sub" => op(BinOp::Sub)?,
        "and" => op(BinOp::And)?,
        "or" => op(BinOp::Or)?,
        "xor" => op(BinOp::Xor)?,
        "rsub" => match s.reverse_binary(BinOp::Sub, &bv) {
            Some(r) => core(r)?,
            None => unreachable!("Symbols defines __rsub__"),
        },
        // `count * other`, `count % other`, `count // other` for every count.
        "mul" => core(s.map_counts(|n| n.binary(BinOp::Mul, &bv)))?,
        "mod" => core(s.map_counts(|n| n.binary(BinOp::Mod, &bv)))?,
        "divide_counts" => core(s.divide_counts(&bv))?,
        "lt" => bool_value(core(s.lexicographic(BinOp::Lt, other()?))?),
        "gt" => bool_value(core(s.lexicographic(BinOp::Gt, other()?))?),
        "le" => bool_value(core(s.all_counts(other()?, BinOp::Le))?),
        "ge" => bool_value(core(s.all_counts(other()?, BinOp::Ge))?),
        "eq" | "ne" => match s.eq_op(&bv, name == "ne") {
            Some(r) => bool_value(core(r)?),
            None => unreachable!("the facade converted the other operand"),
        },
        "isdisjoint" => bool_value(core(s.isdisjoint(&bv))?),
        _ => return Err(to_py_err(Error::Unsupported(format!("Symbols.{name}")))),
    };
    to_py(py, &result)
}

/// A unary operation or query of `Symbols`: `pos`, `neg`, `has_negative_counts`, `size`, `str`.
#[pyfunction]
pub fn symbols_unary<'py>(
    py: Python<'py>,
    name: &str,
    a: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let s = symbols(a)?;
    let result = match name {
        "pos" => core(s.unary(UnOp::Pos).expect("defined"))?,
        "neg" => core(s.unary(UnOp::Neg).expect("defined"))?,
        "has_negative_counts" => Value::Bool(core(s.has_negative_counts())?),
        "size" => core(s.size())?,
        "str" => Value::str(&core(s.py_str())?),
        _ => return Err(to_py_err(Error::Unsupported(format!("Symbols.{name}")))),
    };
    to_py(py, &result)
}

/// `count_subset(divisor, empty_divisor=...)`, the divisor given as its items (a mapping's, or `Counter`'s).
#[pyfunction]
pub fn symbols_count_subset<'py>(
    py: Python<'py>,
    a: &Bound<'py, PyAny>,
    divisor: Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>,
    empty_divisor: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let s = symbols(a)?;
    let divisor = divisor
        .iter()
        .map(|(k, v)| Ok((from_py(k)?, from_py(v)?)))
        .collect::<PyResult<Vec<_>>>()?;
    let empty = from_py(empty_divisor)?;
    to_py(py, &core(s.count_subset_items(divisor, Some(&empty)))?)
}

/// `symbols[key]` for a `str` key.
#[pyfunction]
pub fn symbols_getitem<'py>(
    py: Python<'py>,
    a: &Bound<'py, PyAny>,
    key: &str,
) -> PyResult<Bound<'py, PyAny>> {
    to_py(py, &symbols(a)?.getitem(key))
}
