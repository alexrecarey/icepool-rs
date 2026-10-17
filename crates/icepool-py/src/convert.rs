//! Python objects <-> core `Value`s, host objects, and errors.
//!
//! Exact instances of the types the core models natively (`bool`, `int`, `float`, `fractions.Fraction`, `str`,
//! `tuple`, `None`, and icepool's `Vector`, `VectorWithTruthOnly` and `Symbols`) become native values, so the
//! core's own port of Python's semantics does the work (and the upstream test suite exercises it). Everything else,
//! subclasses included (`IntEnum`, namedtuples, user classes, `complex`), becomes a `Value::Host` whose operators
//! and comparisons call back into Python: the core never decides how a host object behaves. (A `Symbols` whose
//! keys are not all single characters, which only `symbols[tuple]` makes, is a host object too.)

use icepool_core::ownership::{owned_hosts, Traverse};
use pyo3::gc::PyVisit;
use pyo3::PyTraverseError;
use std::any::Any;
use std::sync::Arc;

use dashu_int::{IBig, UBig};
use icepool_core::value::recursion::{level, Level};
use icepool_core::value::{BinOp, HostError, HostObject, HostRef, Symbols, UnOp, Vector};
use icepool_core::{Error, Int, Value};
use pyo3::exceptions::{
    PyAttributeError, PyIndexError, PyKeyError, PyMemoryError, PyNotImplementedError, PyOverflowError,
    PyRecursionError, PyTypeError, PyValueError, PyZeroDivisionError,
};
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyString, PyTuple, PyType};

static FRACTION: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static VECTOR: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static TRUTH_ONLY: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static SYMBOLS: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static DEFAULTDICT: PyOnceLock<Py<PyType>> = PyOnceLock::new();

fn fraction_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    FRACTION.import(py, "fractions", "Fraction")
}

pub fn vector_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    VECTOR.import(py, "icepool.collection.vector", "Vector")
}

fn truth_only_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    TRUTH_ONLY.import(
        py,
        "icepool.collection.vector_with_truth_only",
        "VectorWithTruthOnly",
    )
}

pub fn symbols_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    SYMBOLS.import(py, "icepool.collection.symbols", "Symbols")
}

/// A facade `Symbols` as a core one, if every key is a single character.
fn symbols_from_py(x: &Bound<'_, PyAny>) -> PyResult<Option<Symbols>> {
    let data = x.getattr("_data")?;
    let mut items = Vec::new();
    for item in data.call_method0("items")?.try_iter()? {
        let (k, n): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
        let c = match k.cast::<PyString>() {
            Ok(s) if k.is_exact_instance_of::<PyString>() => {
                let mut chars = s.to_str()?.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => c,
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        items.push((c, from_py(&n)?));
    }
    Ok(Some(Symbols::raw(items)))
}

/// A core `Symbols` as the facade's: `Symbols._new_raw(defaultdict(int, data))`, keeping the dict's order.
pub fn symbols_to_py<'py>(py: Python<'py>, s: &Symbols) -> PyResult<Bound<'py, PyAny>> {
    let dict = PyDict::new(py);
    for (c, n) in s.items() {
        dict.set_item(c.to_string(), to_py(py, n)?)?;
    }
    let defaultdict = DEFAULTDICT.import(py, "collections", "defaultdict")?;
    let data = defaultdict.call1((py.get_type::<PyInt>(), dict))?;
    symbols_type(py)?.call_method1("_new_raw", (data,))
}

/// A Python object outcome. Its hash is taken once, when it enters the core: Python hashes dict keys on insertion
/// too, so an unhashable outcome raises `TypeError` at the same point.
#[derive(Debug)]
pub struct PyHost {
    pub obj: Py<PyAny>,
    hash: i64,
}

/// `__traverse__` for an `icepool_rs` object holding the core value `root`: tells Python's cycle collector about
/// the Python objects (host outcomes, user sources) only `root` holds, so that a cycle through one (an outcome
/// object referring back to the die it is an outcome of) is collected as it is on the Python backend. Objects
/// shared with another core value are not reported (`icepool_core::ownership`: reporting a shared one could free
/// it while in use). The classes are frozen, and immutable references need no `__clear__`: every cycle passes
/// through a mutable Python object, which Python clears.
pub fn traverse(root: &(impl Traverse + ?Sized), visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
    for host in owned_hosts(root) {
        if let Some(h) = host.downcast_ref::<PyHost>() {
            visit.call(&h.obj)?;
        } else if let Some(s) = host.downcast_ref::<crate::host_source::PyHostSource>() {
            visit.call(&s.object)?;
        }
    }
    Ok(())
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

    fn truthy(&self) -> icepool_core::Result<bool> {
        Python::attach(|py| self.obj.bind(py).is_truthy()).map_err(host_error)
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
///
/// Tuples and vectors nest, so this recurses: a level each (`recursion.rs`), and only through small frames (the
/// other types convert in `scalar_from_py`), so a deep one raises `RecursionError` rather than overflowing.
pub fn from_py(x: &Bound<'_, PyAny>) -> PyResult<Value> {
    if x.is_exact_instance_of::<PyTuple>() {
        let _level = py_level()?;
        Ok(Value::Tuple(items_from_py(x.cast::<PyTuple>()?)?.into()))
    } else if is_vector(x)? {
        let _level = py_level()?;
        vector_from_py(x)
    } else {
        scalar_from_py(x)
    }
}

/// A level of recursion (`recursion.rs`), or `RecursionError`.
#[inline(never)]
fn py_level() -> PyResult<Level> {
    level().map_err(|_| PyRecursionError::new_err("maximum recursion depth exceeded"))
}

#[inline(never)]
fn is_vector(x: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = x.py();
    let t = x.get_type();
    Ok(t.is(vector_type(py)?) || t.is(truth_only_type(py)?))
}

#[inline(never)]
fn items_from_py(t: &Bound<'_, PyTuple>) -> PyResult<Vec<Value>> {
    let mut items = Vec::with_capacity(t.len());
    for e in t.iter() {
        items.push(from_py(&e)?);
    }
    Ok(items)
}

/// A `Vector` or `VectorWithTruthOnly`.
#[inline(never)]
fn vector_from_py(x: &Bound<'_, PyAny>) -> PyResult<Value> {
    let truth = x.getattr("_truth_value")?;
    if !x.get_type().is(vector_type(x.py())?) {
        return Ok(Value::Vector(Vector::truth_only(from_py(&truth)?)));
    }
    let items = items_from_py(x.getattr("_data")?.cast::<PyTuple>()?)?;
    let truth = if truth.is_none() {
        None
    } else {
        Some(from_py(&truth)?)
    };
    Ok(Value::Vector(Vector::from_parts(items, truth)))
}

/// `from_py` for everything but tuples and vectors.
#[inline(never)]
fn scalar_from_py(x: &Bound<'_, PyAny>) -> PyResult<Value> {
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
    } else if x.get_type().is(symbols_type(py)?) && symbols_from_py(x)?.is_some() {
        Value::Symbols(symbols_from_py(x)?.expect("checked"))
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

/// `Value` -> Python object. A host object comes back as the very object that went in. Recurses like `from_py`.
pub fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    match v {
        Value::Tuple(t) => {
            let _level = py_level()?;
            Ok(PyTuple::new(py, items_to_py(py, t)?)?.into_any())
        }
        Value::Vector(v) => {
            let _level = py_level()?;
            vector_to_py(py, v)
        }
        _ => scalar_to_py(py, v),
    }
}

fn items_to_py<'py>(py: Python<'py>, items: &[Value]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut out = Vec::with_capacity(items.len());
    for e in items {
        out.push(to_py(py, e)?);
    }
    Ok(out)
}

fn vector_to_py<'py>(py: Python<'py>, v: &Vector) -> PyResult<Bound<'py, PyAny>> {
    match v.data() {
        Ok(items) => {
            let items = PyTuple::new(py, items_to_py(py, items)?)?;
            let kwargs = PyDict::new(py);
            if let Some(t) = v.truth() {
                kwargs.set_item("truth_value", to_py(py, t)?)?;
            }
            vector_type(py)?.call((items,), Some(&kwargs))
        }
        Err(_) => truth_only_type(py)?.call1((to_py(
            py,
            v.truth().expect("a VectorWithTruthOnly has a truth value"),
        )?,)),
    }
}

/// `to_py` for everything but tuples and vectors.
#[inline(never)]
fn scalar_to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
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
        Value::Complex(re, im) => pyo3::types::PyComplex::from_doubles(py, *re, *im).into_any(),
        Value::Symbols(s) => symbols_to_py(py, s)?,
        Value::Host(h) => h
            .as_any()
            .downcast_ref::<PyHost>()
            .expect("the only host objects are Python objects")
            .obj
            .bind(py)
            .clone(),
        Value::Tuple(_) | Value::Vector(_) => to_py(py, v)?,
    })
}

/// How many more frames Python's stack holds: `sys.getrecursionlimit()` minus the frames on it now. The core asks
/// this when its emulation of icepool's `Vector` methods has many frames in use (`recursion.rs`).
pub fn frame_budget() -> usize {
    Python::attach(|py| -> PyResult<usize> {
        let sys = py.import("sys")?;
        let limit: usize = sys.call_method0("getrecursionlimit")?.extract()?;
        let mut depth = 0;
        let mut frame = sys.call_method1("_getframe", (0,)).ok();
        while let Some(f) = frame.filter(|f| !f.is_none()) {
            depth += 1;
            frame = Some(f.getattr("f_back")?);
        }
        Ok(limit.saturating_sub(depth))
    })
    .unwrap_or(usize::MAX)
}

/// Runs Python's signal handlers, for the core's long loops (`icepool_core::interrupt`): Ctrl-C raises
/// `KeyboardInterrupt` from there, as it would between two bytecodes of pure-Python icepool. Python runs signal
/// handlers on the main thread only; elsewhere this does nothing. Takes the GIL back if the computation released it.
pub fn check_signals() -> icepool_core::Result<()> {
    Python::attach(|py| py.check_signals()).map_err(host_error)
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

/// A multiset count (unbounded, as Python's ints) as a Python `int`.
pub fn count_to_py<'py>(py: Python<'py>, c: &Int) -> PyResult<Bound<'py, PyAny>> {
    match c.to_i64() {
        Some(small) => Ok(small.into_pyobject(py)?.into_any()),
        None => int_to_py(py, &c.to_ibig()),
    }
}

/// Counts (a keep tuple, a tuple count) as a Python tuple of `int`s.
pub fn counts_to_py<'py>(py: Python<'py>, cs: &[Int]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        cs.iter()
            .map(|c| count_to_py(py, c))
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// A Python `int` as a multiset count.
pub fn count_from_py(x: &Bound<'_, PyAny>) -> PyResult<Int> {
    if let Ok(i) = x.extract::<i64>() {
        return Ok(Int::from(i));
    }
    Ok(int_from_py(x)?.into())
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
        Error::Recursion(m) => PyRecursionError::new_err(m),
        Error::Value(m) => PyValueError::new_err(m),
        Error::Index(m) => PyIndexError::new_err(m),
        Error::Key(m) => PyKeyError::new_err(m),
        Error::ZeroDivision(m) => PyZeroDivisionError::new_err(m),
        Error::Overflow(m) => PyOverflowError::new_err(m),
        Error::Memory(m) => PyMemoryError::new_err(m),
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
