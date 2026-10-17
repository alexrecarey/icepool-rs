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
//!
//! These are Python methods in icepool, so nested vectors cost Python frames and raise `RecursionError` where
//! Python's stack would overflow (`recursion.rs`): the operator (`__add__`) and `binary_operator`, then
//! `Vector.__init__` and the generator expression it consumes around the elementwise results. The facade calls
//! `binary_operator`, `reverse_binary_operator` and `map` from those Python methods, so only the elementwise frames
//! are counted there; `binary`, `reverse_binary` and `unary` are the operator dispatch, which adds the others.

use std::sync::Arc;

use super::recursion::frames;
use super::{BinOp, UnOp, Value};
use crate::error::{Error, Result};

/// icepool's `Vector`: a tuple-like outcome with elementwise operators.
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
    /// A binary operator or comparison.
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
    /// The vector of `items`.
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

    /// Whether this is a `VectorWithTruthOnly`: the result of a comparison, which has a truth value but no data.
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
        // `__neg__` and `unary_operator`.
        let _frames = frames(2)?;
        self.map(|x| x.unary(op))
    }

    /// `Vector(f(x) for x in self)`.
    pub fn map(&self, f: impl Fn(&Value) -> Result<Value>) -> Result<Value> {
        let data = self.data()?;
        // `Vector.__init__` and the generator expression.
        let _frames = frames(2)?;
        let items = data.iter().map(f).collect::<Result<Vec<_>>>()?;
        Ok(Value::Vector(Vector::new(items)))
    }

    /// `Vector.binary_operator(other, op)` with `self` on the left, for the standard operators: the comparators
    /// give the result a truth value (for a non-vector: `False` for `==`, `True` for `!=`, none for the orderings).
    pub fn binary(&self, op: VecOp, other: &Value) -> Result<Value> {
        let compare_non_vector = match op {
            VecOp::Eq(ne) => Some(Value::Bool(ne)),
            _ => None,
        };
        // `__add__` (or `__eq__`, ...) and `binary_operator`.
        let _frames = frames(2)?;
        self.binary_operator(
            other,
            |x, y| op.apply(x, y),
            op.is_comparison(),
            compare_non_vector,
        )
    }

    /// `Vector.binary_operator(other, op, compare_for_truth=..., compare_non_vector=...)` for any `op`.
    ///
    /// If the other operand is also a `Vector`, the operator is applied to each pair of elements from `self` and
    /// `other`. Both must have the same length. Otherwise the other operand is broadcast to each element of `self`.
    /// For a comparison (`compare_for_truth`), the result also has a truth value based on lexicographic ordering:
    /// `op(self._data, other._data)`, computed first.
    pub fn binary_operator(
        &self,
        other: &Value,
        op: impl Fn(&Value, &Value) -> Result<Value>,
        compare_for_truth: bool,
        compare_non_vector: Option<Value>,
    ) -> Result<Value> {
        let data = self.data()?;
        let Value::Vector(o) = other else {
            // `Vector.__init__` and the generator expression.
            let _frames = frames(2)?;
            let items = data
                .iter()
                .map(|x| op(x, other))
                .collect::<Result<Vec<_>>>()?;
            return Ok(Value::Vector(Vector::with_truth(items, compare_non_vector)));
        };
        let odata = o.data()?;
        let truth = if compare_for_truth {
            Some(op(
                &Value::Tuple(data.clone()),
                &Value::Tuple(odata.clone()),
            )?)
        } else {
            None
        };
        if data.len() == odata.len() {
            let _frames = frames(2)?;
            let items = data
                .iter()
                .zip(odata.iter())
                .map(|(x, y)| op(x, y))
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
    }

    /// `Vector.reverse_binary_operator(other, op)`: `op(other, x)` for each element. Two vectors always use the
    /// left one's `binary_operator` in Python's dispatch; called directly, they must have the same length.
    /// Comparisons never get here from an operator: Python reflects `x < v` to `v > x`.
    pub fn reverse_binary_operator(
        &self,
        other: &Value,
        op: impl Fn(&Value, &Value) -> Result<Value>,
    ) -> Result<Value> {
        let data = self.data()?;
        if let Value::Vector(o) = other {
            let odata = o.data()?;
            if data.len() != odata.len() {
                return Err(Error::Index(format!(
                    "Binary operators on Vectors are only valid if both are the same length ({} vs. {}).",
                    odata.len(),
                    data.len()
                )));
            }
            let _frames = frames(2)?;
            let items = data
                .iter()
                .zip(odata.iter())
                .map(|(x, y)| op(y, x))
                .collect::<Result<Vec<_>>>()?;
            return Ok(Value::Vector(Vector::new(items)));
        }
        self.map(|x| op(other, x))
    }

    /// `other <op> self` for a standard arithmetic operator.
    pub fn reverse_binary(&self, op: BinOp, other: &Value) -> Result<Value> {
        // `__radd__` and `reverse_binary_operator`.
        let _frames = frames(2)?;
        self.reverse_binary_operator(other, |x, y| x.binary(op, y))
    }

    /// `self._truth_value`.
    pub fn truth(&self) -> Option<&Value> {
        self.truth.as_deref()
    }

    /// A vector as the facade holds it: its elements and truth value.
    pub fn from_parts(items: Vec<Value>, truth: Option<Value>) -> Self {
        Vector::with_truth(items, truth)
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

/// What reading `VectorWithTruthOnly._data` raises.
pub fn truth_only_error() -> Error {
    Error::Value(
        "The result of a comparison between vectors of different lengths can only be used as a truth value."
            .into(),
    )
}

/// `==` between two vectors as a dict sees it: Python compares `_data` tuples (a `VectorWithTruthOnly` cannot be a
/// key: hashing it raises, see `Outcome::check_key`).
impl crate::ownership::Traverse for Vector {
    fn traverse<'a>(&'a self, owners: &mut crate::ownership::Owners<'a>) {
        if let Some(data) = &self.data {
            owners.arc(data);
        }
        if let Some(truth) = &self.truth {
            owners.arc(truth);
        }
    }
}

impl PartialEq for Vector {
    fn eq(&self, other: &Self) -> bool {
        match (&self.data, &other.data) {
            (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[i64]) -> Value {
        Value::Vector(Vector::new(xs.iter().map(|&x| Value::from(x)).collect()))
    }

    #[test]
    fn elementwise_with_truth_values() {
        assert_eq!(
            v(&[1, 2]).binary(BinOp::Add, &Value::from(1)).unwrap(),
            v(&[2, 3])
        );
        let lt = v(&[1, 5]).binary(BinOp::Lt, &v(&[2, 0])).unwrap();
        assert_eq!(lt.to_string(), "Vector((True, False))");
        assert!(lt.truthy().unwrap());
        // Different lengths: only a truth value; without a comparison, no truth value at all.
        let short = v(&[1]).binary(BinOp::Lt, &v(&[1, 0])).unwrap();
        assert_eq!(short.to_string(), "VectorWithTruthOnly(True)");
        assert!(short.check_hashable().is_err());
        assert!(v(&[1]).binary(BinOp::Add, &v(&[1, 0])).is_err());
        assert!(v(&[1]).truthy().is_err());
        assert!(!v(&[1])
            .eq_op(&Value::from(1), false)
            .unwrap()
            .truthy()
            .unwrap());
    }
}
