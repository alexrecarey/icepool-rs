//! The Python callables `Die.unary_operator` / `binary_operator` receive, mapped to the core's operators.
//!
//! The facade passes the very function Python icepool would call (`operator.add`, `math.floor`, `round`...). The
//! known ones (`operator.getitem` from `marginals` too) run on the core's `Value` semantics; any other callable (a
//! user's function, `operator.attrgetter`) is a host callback, called on each outcome exactly as Python icepool
//! calls it.

use icepool_core::value::builtins::{getitem, Index};
use icepool_core::value::{BinOp, UnOp};
use icepool_core::{Error, Value};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyTuple};

use crate::convert::{from_py, host_error, to_py};

/// What `binary_operator` does per pair of outcomes.
pub enum Binary<'py> {
    Op(BinOp, Bound<'py, PyAny>),
    /// `operator.eq` / `operator.ne`: the core's `Value::eq_op` (the core has no `BinOp` for it); the flag is
    /// `true` for `!=`.
    Eq(bool, Bound<'py, PyAny>),
    Call(Bound<'py, PyAny>),
}

/// What `unary_operator` does per outcome.
pub enum Unary<'py> {
    Op(UnOp, Bound<'py, PyAny>),
    Round(Bound<'py, PyAny>),
    /// `operator.getitem(outcome, key)` (`marginals[key]`), with the key as the core takes it.
    GetItem(Index, Bound<'py, PyAny>),
    Call(Bound<'py, PyAny>),
}

const BINARY: [(&str, BinOp); 16] = [
    ("add", BinOp::Add),
    ("sub", BinOp::Sub),
    ("mul", BinOp::Mul),
    ("truediv", BinOp::TrueDiv),
    ("floordiv", BinOp::FloorDiv),
    ("mod", BinOp::Mod),
    ("pow", BinOp::Pow),
    ("lshift", BinOp::LShift),
    ("rshift", BinOp::RShift),
    ("and_", BinOp::And),
    ("or_", BinOp::Or),
    ("xor", BinOp::Xor),
    ("lt", BinOp::Lt),
    ("le", BinOp::Le),
    ("gt", BinOp::Gt),
    ("ge", BinOp::Ge),
];

const UNARY: [(&str, &str, UnOp); 7] = [
    ("operator", "neg", UnOp::Neg),
    ("operator", "pos", UnOp::Pos),
    ("operator", "invert", UnOp::Invert),
    ("operator", "abs", UnOp::Abs),
    ("math", "trunc", UnOp::Trunc),
    ("math", "floor", UnOp::Floor),
    ("math", "ceil", UnOp::Ceil),
];

static BINARY_FNS: PyOnceLock<Vec<Py<PyAny>>> = PyOnceLock::new();
static UNARY_FNS: PyOnceLock<Vec<Py<PyAny>>> = PyOnceLock::new();
static EQ_FNS: PyOnceLock<[Py<PyAny>; 2]> = PyOnceLock::new();
static ROUND: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static GETITEM: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn binary_fns(py: Python<'_>) -> PyResult<&Vec<Py<PyAny>>> {
    BINARY_FNS.get_or_try_init(py, || {
        let m = py.import("operator")?;
        BINARY
            .iter()
            .map(|(name, _)| Ok(m.getattr(*name)?.unbind()))
            .collect()
    })
}

fn unary_fns(py: Python<'_>) -> PyResult<&Vec<Py<PyAny>>> {
    UNARY_FNS.get_or_try_init(py, || {
        UNARY
            .iter()
            .map(|(module, name, _)| Ok(py.import(*module)?.getattr(*name)?.unbind()))
            .collect()
    })
}

/// `operator.<op>` for a core operator.
pub fn binop_function(py: Python<'_>, op: BinOp) -> PyResult<Bound<'_, PyAny>> {
    let i = BINARY
        .iter()
        .position(|(_, o)| *o == op)
        .expect("every BinOp is listed");
    Ok(binary_fns(py)?[i].bind(py).clone())
}

/// The Python function for a core unary operator.
pub fn unop_function(py: Python<'_>, op: UnOp) -> PyResult<Bound<'_, PyAny>> {
    let i = UNARY
        .iter()
        .position(|(_, _, o)| *o == op)
        .expect("every UnOp is listed");
    Ok(unary_fns(py)?[i].bind(py).clone())
}

/// Classifies `op` for `binary_operator(other, op, *args, **kwargs)`. Extra arguments always make it a callback.
pub fn binary<'py>(op: &Bound<'py, PyAny>, plain: bool) -> PyResult<Binary<'py>> {
    let py = op.py();
    if plain {
        if let Some(i) = binary_fns(py)?.iter().position(|f| op.is(f)) {
            return Ok(Binary::Op(BINARY[i].1, op.clone()));
        }
        let eq = EQ_FNS.get_or_try_init(py, || -> PyResult<_> {
            let m = py.import("operator")?;
            Ok([m.getattr("eq")?.unbind(), m.getattr("ne")?.unbind()])
        })?;
        if let Some(i) = eq.iter().position(|f| op.is(f)) {
            return Ok(Binary::Eq(i == 1, op.clone()));
        }
    }
    Ok(Binary::Call(op.clone()))
}

/// Classifies `op` for `unary_operator(op, *args, **kwargs)`: `round(x, ndigits)` and `operator.getitem(x, key)`
/// take one extra argument.
pub fn unary<'py>(
    op: &Bound<'py, PyAny>,
    args: &Bound<'py, PyTuple>,
    nargs: usize,
) -> PyResult<Unary<'py>> {
    let py = op.py();
    if nargs == 0 {
        if let Some(i) = unary_fns(py)?.iter().position(|f| op.is(f)) {
            return Ok(Unary::Op(UNARY[i].2, op.clone()));
        }
    }
    let round = ROUND.get_or_try_init(py, || -> PyResult<_> {
        Ok(py.import("builtins")?.getattr("round")?.unbind())
    })?;
    if nargs == 1 && op.is(round) {
        return Ok(Unary::Round(op.clone()));
    }
    let getitem = GETITEM.get_or_try_init(py, || -> PyResult<_> {
        Ok(py.import("operator")?.getattr("getitem")?.unbind())
    })?;
    if nargs == 1 && op.is(getitem) {
        return Ok(Unary::GetItem(
            crate::stats::index(&args.get_item(0)?)?,
            op.clone(),
        ));
    }
    Ok(Unary::Call(op.clone()))
}

/// Calls a Python function on outcomes: `f(*values, *args, **kwargs)`.
pub fn call(
    f: &Bound<'_, PyAny>,
    values: &[&Value],
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> icepool_core::Result<Value> {
    let py = f.py();
    (|| {
        let mut all: Vec<Bound<'_, PyAny>> = values
            .iter()
            .map(|v| to_py(py, v))
            .collect::<PyResult<_>>()?;
        all.extend(args.iter());
        from_py(&f.call(PyTuple::new(py, all)?, kwargs)?)
    })()
    .map_err(host_error)
}

/// A value containing a host object anywhere (inside tuples, vectors and counts too).
pub fn has_host(v: &Value) -> bool {
    match v {
        Value::Host(_) => true,
        Value::Tuple(t) => t.iter().any(has_host),
        Value::Vector(x) => {
            x.data().is_ok_and(|d| d.iter().any(has_host)) || x.truth().is_some_and(has_host)
        }
        Value::Symbols(s) => s.items().iter().any(|(_, n)| has_host(n)),
        _ => false,
    }
}

/// A built-in binary operator as the core computes it on native outcomes, without Python (see [`Binary::native`]).
#[derive(Clone, Copy)]
pub enum NativeBinary {
    Op(BinOp),
    Eq(bool),
}

impl NativeBinary {
    /// `Binary::apply` for outcomes without host objects, except that a case the core does not model stays
    /// `Unsupported` rather than going to Python's operator.
    pub fn apply(self, a: &Value, b: &Value) -> icepool_core::Result<Value> {
        match self {
            NativeBinary::Op(op) => a.binary(op, b),
            NativeBinary::Eq(ne) => a.eq_op(b, ne),
        }
    }
}

impl Binary<'_> {
    /// The operator without its Python function, if it is a built-in one.
    pub fn native(&self) -> Option<NativeBinary> {
        match self {
            Binary::Op(op, _) => Some(NativeBinary::Op(*op)),
            Binary::Eq(ne, _) => Some(NativeBinary::Eq(*ne)),
            Binary::Call(_) => None,
        }
    }

    pub fn apply(
        &self,
        a: &Value,
        b: &Value,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> icepool_core::Result<Value> {
        match self {
            Binary::Op(op, f) => match a.binary(*op, b) {
                // The core does not model this case for native values yet (printf-style `str %` with conversion
                // specifiers): the outcome operation is Python's, as for host objects.
                Err(Error::Unsupported(_)) => call(f, &[a, b], args, kwargs),
                r => r,
            },
            // `==` on host objects is Python's own (it need not be an equivalence, nor return a bool).
            Binary::Eq(_, f) if has_host(a) || has_host(b) => call(f, &[a, b], args, kwargs),
            Binary::Eq(ne, _) => a.eq_op(b, *ne),
            Binary::Call(f) => call(f, &[a, b], args, kwargs),
        }
    }
}

impl Unary<'_> {
    pub fn apply(
        &self,
        a: &Value,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> icepool_core::Result<Value> {
        let r = match self {
            Unary::Op(op, _) => a.unary(*op),
            Unary::Round(_) => {
                let ndigits = (|| from_py(&args.get_item(0)?))().map_err(host_error)?;
                if has_host(&ndigits) {
                    return call(self.function(), &[a], args, kwargs);
                }
                a.round(&ndigits)
            }
            Unary::GetItem(key, _) => getitem(a, key),
            Unary::Call(f) => return call(f, &[a], args, kwargs),
        };
        match r {
            // As in `Binary::apply`: a case the core does not model yet is Python's own.
            Err(Error::Unsupported(_)) => call(self.function(), &[a], args, kwargs),
            r => r,
        }
    }

    fn function(&self) -> &Bound<'_, PyAny> {
        match self {
            Unary::Op(_, f) | Unary::Round(f) | Unary::GetItem(_, f) | Unary::Call(f) => f,
        }
    }
}
