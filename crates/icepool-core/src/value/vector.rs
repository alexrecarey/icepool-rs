//! `Value::Vector`: icepool's `Vector` (collection/vector.py), an immutable tuple-like outcome that applies most
//! operators elementwise, and `VectorWithTruthOnly` (collection/vector_with_truth_only.py), what comparing two
//! vectors of different lengths gives.
//!
//! A `Vector` carries a truth value only when it is the result of a comparison: `bool(Vector(...))` raises
//! otherwise. The truth value takes no part in `==` or hashing (Python hashes `(Vector, self._data)`), so when equal
//! vectors merge in a die, the first one's truth value is kept, as in a Python dict.
//!
//! The truth value is whatever `op(self._data, other._data)` returned, and Python's tuple comparison returns the
//! comparison of the first differing elements *as is*: for vectors of vectors that is a vector, not a bool, and
//! `bool()` then raises (`__bool__ should return bool`). So a die of nested vectors does not sort.

use std::sync::Arc;

use super::{ops, BinOp, UnOp, Value};
use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Vector {
    /// `None` for a `VectorWithTruthOnly`, whose `_data` property raises.
    data: Option<Arc<[Value]>>,
    /// `self._truth_value`: `None`, or what the comparison returned (usually a bool).
    truth: Option<Arc<Value>>,
}

/// An operator applied elementwise: Python's binary operators and comparisons, and `==` / `!=` (which the core
/// keeps apart from `BinOp`, as they never raise between native values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOp {
    Bin(BinOp),
    /// `operator.ne` if true, else `operator.eq`.
    Eq(bool),
}

impl VecOp {
    /// Python's `op(x, y)` on two elements.
    pub fn apply(self, x: &Value, y: &Value) -> Result<Value> {
        match self {
            VecOp::Bin(op) => x.binary(op, y),
            VecOp::Eq(ne) => x.eq_op(y, ne),
        }
    }

    fn is_comparison(self) -> bool {
        matches!(
            self,
            VecOp::Eq(_) | VecOp::Bin(BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
        )
    }
}

impl Vector {
    pub fn new(items: Vec<Value>) -> Self {
        Vector {
            data: Some(Arc::from(items)),
            truth: None,
        }
    }

    fn with_truth(items: Vec<Value>, truth: Option<Value>) -> Self {
        Vector {
            data: Some(Arc::from(items)),
            truth: truth.map(Arc::new),
        }
    }

    /// `VectorWithTruthOnly(truth_value)`.
    pub fn truth_only(truth: Value) -> Self {
        Vector {
            data: None,
            truth: Some(Arc::new(truth)),
        }
    }

    pub fn is_truth_only(&self) -> bool {
        self.data.is_none()
    }

    /// `self._data`, which raises for a `VectorWithTruthOnly`. Everything but `bool()` and `repr()` reads it.
    pub fn data(&self) -> Result<&Arc<[Value]>> {
        self.data.as_ref().ok_or_else(truth_only_error)
    }

    /// `bool(self)`.
    pub fn truthy(&self) -> Result<bool> {
        match self.truth.as_deref() {
            None => Err(Error::Type(
                "Vector only has a truth value if it is the result of a comparison operator."
                    .into(),
            )),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(Error::Type(format!(
                "__bool__ should return bool, returned {}",
                other.type_name()
            ))),
        }
    }

    /// `type(self).__qualname__`.
    pub fn type_name(&self) -> &'static str {
        if self.data.is_some() {
            "Vector"
        } else {
            "VectorWithTruthOnly"
        }
    }

    /// `Vector.unary_operator`: elementwise. (`round` goes through `map`.)
    pub fn unary(&self, op: UnOp) -> Result<Value> {
        self.map(|x| x.unary(op))
    }

    /// `Vector(f(x) for x in self)`.
    pub fn map(&self, f: impl Fn(&Value) -> Result<Value>) -> Result<Value> {
        let items = self.data()?.iter().map(f).collect::<Result<Vec<_>>>()?;
        Ok(Value::Vector(Vector::new(items)))
    }

    /// `Vector.binary_operator(other, op)` with `self` on the left. `None` is `NotImplemented` (never returned for
    /// values: icepool returns it only for a `Population` or an `AgainExpression`, which are not values).
    pub fn binary(&self, op: VecOp, other: &Value) -> Result<Value> {
        let data = self.data()?;
        // The comparators also give the result a truth value: lexicographic for two vectors, and for a non-vector
        // `False` for `==`, `True` for `!=`, none for the orderings.
        if let Value::Vector(o) = other {
            let odata = o.data()?;
            let truth = if op.is_comparison() {
                Some(tuple_op(op, data, odata)?)
            } else {
                None
            };
            if data.len() == odata.len() {
                let items = data
                    .iter()
                    .zip(odata.iter())
                    .map(|(x, y)| op.apply(x, y))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Vector(Vector::with_truth(items, truth)))
            } else if let Some(truth) = truth {
                Ok(Value::Vector(Vector::truth_only(truth)))
            } else {
                Err(Error::Index(format!(
                    "Binary operators on Vectors are only valid if both are the same length ({} vs. {}).",
                    data.len(),
                    odata.len()
                )))
            }
        } else {
            let truth = match op {
                VecOp::Eq(ne) => Some(Value::Bool(ne)),
                _ => None,
            };
            let items = data
                .iter()
                .map(|x| op.apply(x, other))
                .collect::<Result<Vec<_>>>()?;
            Ok(Value::Vector(Vector::with_truth(items, truth)))
        }
    }

    /// `Vector.reverse_binary_operator(other, op)`: `other op self` where `other` is not a vector (two vectors
    /// always use the left one's `binary_operator`). Comparisons never get here: Python reflects `x < v` to
    /// `v > x`, a forward operator.
    pub fn reverse_binary(&self, op: BinOp, other: &Value) -> Result<Value> {
        self.map(|x| other.binary(op, x))
    }

    /// Python's `repr()` / `str()` (the same for a vector: `str(tuple)` shows its elements' reprs).
    /// `elem` is `repr` of an element; `str` of the truth value of a `VectorWithTruthOnly` is `str_of`.
    pub fn repr_with(
        &self,
        elem: impl Fn(&Value) -> Result<String>,
        str_of: impl Fn(&Value) -> Result<String>,
    ) -> Result<String> {
        match &self.data {
            None => Ok(format!(
                "VectorWithTruthOnly({})",
                str_of(self.truth.as_deref().expect("always has one"))?
            )),
            Some(d) => {
                let items = d.iter().map(elem).collect::<Result<Vec<_>>>()?;
                Ok(if items.len() == 1 {
                    format!("Vector(({},))", items[0])
                } else {
                    format!("Vector(({}))", items.join(", "))
                })
            }
        }
    }
}

/// `op(self._data, other._data)`: Python's tuple comparison. `==` and `!=` give a bool; an ordering gives the
/// comparison of the first elements that differ (by `==`), whatever it returns, or compares the lengths.
fn tuple_op(op: VecOp, a: &Arc<[Value]>, b: &Arc<[Value]>) -> Result<Value> {
    match op {
        VecOp::Eq(ne) => Ok(Value::Bool(
            (Value::Tuple(a.clone()) == Value::Tuple(b.clone())) != ne,
        )),
        VecOp::Bin(op) => match a.iter().zip(b.iter()).find(|(x, y)| x != y) {
            Some((x, y)) => x.binary(op, y),
            None => ops::compare(
                op,
                &Value::from(a.len() as i64),
                &Value::from(b.len() as i64),
            )
            .map(Value::Bool),
        },
    }
}

/// What reading `VectorWithTruthOnly._data` raises.
pub fn truth_only_error() -> Error {
    Error::Value(
        "The result of a comparison between vectors of different lengths can only be used as a truth value."
            .into(),
    )
}

/// `==` between two vectors as a dict sees it: Python compares `_data` tuples (a `VectorWithTruthOnly` cannot be a
/// key: hashing it raises, see `Outcome::check_key`).
impl PartialEq for Vector {
    fn eq(&self, other: &Self) -> bool {
        match (&self.data, &other.data) {
            (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y),
            _ => false,
        }
    }
}
