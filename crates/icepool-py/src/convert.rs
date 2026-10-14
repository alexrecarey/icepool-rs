//! Python objects <-> core `Value`s, host objects, and errors.
//!
//! Exact instances of the types the core models natively (`bool`, `int`, `float`, `fractions.Fraction`, `str`,
//! `tuple`, `None`) become native values, so the core's own port of Python's semantics does the work (and the
//! upstream test suite exercises it). Everything else, subclasses included (`IntEnum`, namedtuples, `Vector`,
//! `Symbols`, user classes, `complex`), becomes a `Value::Host` whose operators and comparisons call back into
//! Python: the core never decides how a host object behaves.

use std::any::Any;
use std::sync::Arc;

use dashu_int::{IBig, UBig};
use icepool_core::value::{BinOp, HostError, HostObject, HostRef, UnOp};
use icepool_core::{Error, Value};
use pyo3::exceptions::{
    PyAttributeError, PyIndexError, PyKeyError, PyNotImplementedError, PyOverflowError,
    PyRecursionError, PyTypeError, PyValueError, PyZeroDivisionError,
};
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString, PyTuple, PyType};

static FRACTION: PyOnceLock<Py<PyType>> = PyOnceLock::new();

fn fraction_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    FRACTION.import(py, "fractions", "Fraction")
}

/// A Python object outcome. Its hash is taken once, when it enters the core: Python hashes dict keys on insertion
/// too, so an unhashable outcome raises `TypeError` at the same point.
#[derive(Debug)]
pub struct PyHost {
    pub obj: Py<PyAny>,
    hash: i64,
}

impl HostObject for PyHost {
    fn py_hash(&self) -> i64 {
        self.hash
    }

    fn eq(&self, a: &Value, b: &Value) -> bool {
        Python::attach(|py| {
            let (Ok(a), Ok(b)) = (to_py(py, a), to_py(py, b)) else {
                return false;
            };
            a.is(&b) || a.eq(&b).unwrap_or(false)
        })
    }

    fn compare(&self, op: BinOp, a: &Value, b: &Value) -> icepool_core::Result<bool> {
        Python::attach(|py| {
            let op = match op {
                BinOp::Lt => CompareOp::Lt,
                BinOp::Le => CompareOp::Le,
                BinOp::Gt => CompareOp::Gt,
                BinOp::Ge => CompareOp::Ge,
                _ => unreachable!("not a comparison"),
            };
            to_py(py, a)?.rich_compare(to_py(py, b)?, op)?.is_truthy()
        })
        .map_err(host_error)
    }

    fn binary(&self, op: BinOp, a: &Value, b: &Value) -> icepool_core::Result<Value> {
        Python::attach(|py| {
            let f = crate::ops::binop_function(py, op)?;
            from_py(&f.call1((to_py(py, a)?, to_py(py, b)?))?)
        })
        .map_err(host_error)
    }

    fn unary(&self, op: UnOp) -> icepool_core::Result<Value> {
        Python::attach(|py| {
            let f = crate::ops::unop_function(py, op)?;
            from_py(&f.call1((self.obj.bind(py),))?)
        })
        .map_err(host_error)
    }

    fn truthy(&self) -> bool {
        Python::attach(|py| self.obj.bind(py).is_truthy().unwrap_or(false))
    }

    fn repr(&self) -> String {
        Python::attach(|py| match self.obj.bind(py).repr() {
            Ok(s) => s.to_string(),
            Err(_) => "<object>".into(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_float(&self) -> icepool_core::Result<f64> {
        Python::attach(|py| crate::stats::host_to_float(self.obj.bind(py)))
    }

    fn as_rational(&self) -> icepool_core::Result<Option<dashu_ratio::RBig>> {
        Python::attach(|py| crate::stats::host_as_rational(self.obj.bind(py)))
    }

    fn as_index(&self) -> icepool_core::Result<Option<IBig>> {
        Python::attach(|py| crate::stats::host_as_index(self.obj.bind(py)))
    }

    fn getitem(&self, key: &icepool_core::value::builtins::Index) -> icepool_core::Result<Value> {
        Python::attach(|py| crate::stats::host_getitem(self.obj.bind(py), key))
    }

    fn py_len(&self) -> icepool_core::Result<usize> {
        Python::attach(|py| crate::stats::host_len(self.obj.bind(py)))
    }

    fn py_repr(&self) -> icepool_core::Result<String> {
        Python::attach(|py| Ok(self.obj.bind(py).repr()?.to_string())).map_err(host_error)
    }

    fn py_str(&self) -> icepool_core::Result<String> {
        Python::attach(|py| Ok(self.obj.bind(py).str()?.to_string())).map_err(host_error)
    }

    fn len_kind(&self) -> icepool_core::Result<icepool_core::value::LenKind> {
        Python::attach(|py| crate::format::host_len_kind(self.obj.bind(py))).map_err(host_error)
    }
}

/// Python object -> `Value` (see the module comment for which types are native).
pub fn from_py(x: &Bound<'_, PyAny>) -> PyResult<Value> {
    let py = x.py();
    Ok(if x.is_none() {
        Value::None
    } else if x.is_exact_instance_of::<PyBool>() {
        Value::Bool(x.extract::<bool>()?)
    } else if x.is_exact_instance_of::<PyInt>() {
        Value::Int(int_from_py(x)?)
    } else if x.is_exact_instance_of::<PyFloat>() {
        Value::Float(x.extract::<f64>()?)
    } else if x.is_exact_instance_of::<PyString>() {
        Value::str(x.cast::<PyString>()?.to_str()?)
    } else if x.is_exact_instance_of::<PyTuple>() {
        let t = x.cast::<PyTuple>()?;
        Value::tuple(t.iter().map(|e| from_py(&e)).collect::<PyResult<_>>()?)
    } else if x.get_type().is(fraction_type(py)?) {
        let n = int_from_py(&x.getattr("numerator")?)?;
        let d = int_from_py(&x.getattr("denominator")?)?;
        Value::Fraction(dashu_ratio::RBig::from_parts(
            n,
            d.try_into().expect("positive"),
        ))
    } else {
        Value::Host(HostRef::new(PyHost {
            obj: x.clone().unbind(),
            hash: x.hash()? as i64,
        }))
    })
}

/// `Value` -> Python object. A host object comes back as the very object that went in.
pub fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Value::None => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        Value::Int(i) => int_to_py(py, i)?,
        Value::Float(f) => PyFloat::new(py, *f).into_any(),
        Value::Fraction(r) => fraction_type(py)?.call1((
            int_to_py(py, r.numerator())?,
            int_to_py(py, &IBig::from(r.denominator().clone()))?,
        ))?,
        Value::Str(s) => PyString::new(py, s).into_any(),
        Value::Tuple(t) => PyTuple::new(
            py,
            t.iter()
                .map(|e| to_py(py, e))
                .collect::<PyResult<Vec<_>>>()?,
        )?
        .into_any(),
        Value::Complex(re, im) => pyo3::types::PyComplex::from_doubles(py, *re, *im).into_any(),
        Value::Host(h) => h
            .as_any()
            .downcast_ref::<PyHost>()
            .expect("the only host objects are Python objects")
            .obj
            .bind(py)
            .clone(),
    })
}

/// Python `int` -> `IBig`. Big ints go through hex: power-of-two bases are exempt from Python's limit on int/str
/// conversions (`sys.set_int_max_str_digits`).
pub fn int_from_py(x: &Bound<'_, PyAny>) -> PyResult<IBig> {
    if let Ok(i) = x.extract::<i64>() {
        return Ok(IBig::from(i));
    }
    let hex = x.call_method1("__format__", ("x",))?;
    let hex = hex.cast::<PyString>()?.to_str()?;
    let (neg, digits) = match hex.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, hex),
    };
    let mag = UBig::from_str_radix(digits, 16).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(if neg {
        -IBig::from(mag)
    } else {
        IBig::from(mag)
    })
}

pub fn int_to_py<'py>(py: Python<'py>, i: &IBig) -> PyResult<Bound<'py, PyAny>> {
    if let Ok(small) = i64::try_from(i) {
        return Ok(small.into_pyobject(py)?.into_any());
    }
    let hex = format!("{i:x}");
    py.get_type::<PyInt>().call1((hex, 16))
}

pub fn quantity_to_py<'py>(py: Python<'py>, q: &UBig) -> PyResult<Bound<'py, PyAny>> {
    if let Ok(small) = u64::try_from(q) {
        return Ok(small.into_pyobject(py)?.into_any());
    }
    int_to_py(py, &IBig::from(q.clone()))
}

/// A Python exception raised inside a callback, carried through the core.
pub fn host_error(e: PyErr) -> Error {
    Error::Host(HostError(Arc::new(e)))
}

/// A core error as the Python exception icepool raises; a host exception is re-raised as it was.
pub fn to_py_err(e: Error) -> PyErr {
    match e {
        Error::Host(h) => match h.0.downcast_ref::<PyErr>() {
            Some(err) => Python::attach(|py| err.clone_ref(py)),
            None => PyTypeError::new_err("a host error from another host"),
        },
        // Where Python raises AttributeError (`Reroll` expanded as a mapping), the core raises this.
        Error::Type(m) if m.starts_with("AttributeError:") => PyAttributeError::new_err(m),
        Error::Type(m) => PyTypeError::new_err(m),
        // Where Python would exceed its recursion limit (`_sum_all` of a big count), the core raises this.
        Error::Value(m) if m.starts_with("RecursionError:") => PyRecursionError::new_err(m),
        Error::Value(m) => PyValueError::new_err(m),
        Error::Index(m) => PyIndexError::new_err(m),
        Error::Key(m) => PyKeyError::new_err(m),
        Error::ZeroDivision(m) => PyZeroDivisionError::new_err(m),
        Error::Overflow(m) => PyOverflowError::new_err(m),
        // Where icepool raises a plain `OrderError` (`sort_pair` without an inferrable size), the core says so.
        Error::Order(m) if m.starts_with("OrderError:") => icepool_error("OrderError", m),
        Error::Order(m) => icepool_error("ConflictingOrderError", m),
        Error::UnsupportedOrder(m) => icepool_error("UnsupportedOrder", m),
        Error::NotImplemented(m) => PyNotImplementedError::new_err(m),
        // Python would answer, the core cannot yet: say so loudly rather than guess.
        Error::Unsupported(m) => PyNotImplementedError::new_err(format!("icepool_rs: {m}")),
    }
}

/// An exception class defined by icepool itself (`icepool.order`).
fn icepool_error(name: &str, msg: String) -> PyErr {
    Python::attach(
        |py| match py.import("icepool.order").and_then(|m| m.getattr(name)) {
            Ok(cls) => PyErr::from_type(cls.cast_into::<PyType>().expect("a class"), msg),
            Err(e) => e,
        },
    )
}
