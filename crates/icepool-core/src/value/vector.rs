//! `Value::Vector`: icepool's `Vector` (collection/vector.py), an immutable tuple-like outcome that applies most
//! operators elementwise, and `VectorWithTruthOnly` (collection/vector_with_truth_only.py), what comparing two
//! vectors of different lengths gives.
//!
//! A `Vector` carries a truth value only when it is the result of a comparison: `bool(Vector(...))` raises
//! otherwise. The truth value takes no part in `==` or hashing (Python hashes `(Vector, self._data)`), so when equal
//! vectors merge in a die, the first one's truth value is kept, as in a Python dict.
//!
//! The truth value follows Python's tuple comparison (the first pair of elements that differ decides, else the
//! lengths), converted to `bool` so that nested vectors compare, and reusing the elementwise results so that
//! comparing nested vectors takes polynomial rather than exponential time (`lexicographic_truth`).
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
///
/// ```
/// use icepool_core::value::{BinOp, VecOp, Vector};
/// use icepool_core::Value;
///
/// // `Vector((1, 2)) * 2` is `Vector((2, 4))`.
/// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
/// assert_eq!(v.binary(VecOp::Bin(BinOp::Mul), &Value::from(2)).unwrap().to_string(), "Vector((2, 4))");
/// assert_eq!(v.data().unwrap().len(), 2);
/// ```
#[derive(Clone, Debug)]
pub struct Vector {
    /// `None` for a `VectorWithTruthOnly`, whose `_data` property raises.
    data: Option<Arc<[Value]>>,
    /// `self._truth_value`: `None`, or what the comparison returned (usually a bool).
    truth: Option<Arc<Value>>,
}

/// An operator applied elementwise: Python's binary operators and comparisons, and `==` / `!=` (which the core
/// keeps apart from `BinOp`, as they never raise between native values).
///
/// ```
/// use icepool_core::value::{BinOp, VecOp};
/// use icepool_core::Value;
///
/// assert_eq!(VecOp::Bin(BinOp::Add).apply(&Value::from(1), &Value::from(2)).unwrap(), Value::from(3));
/// // `1 != 2`.
/// assert_eq!(VecOp::Eq(true).apply(&Value::from(1), &Value::from(2)).unwrap(), Value::Bool(true));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOp {
    /// A binary operator or comparison.
    Bin(BinOp),
    /// `operator.ne` if true, else `operator.eq`.
    Eq(bool),
}

impl VecOp {
    /// Python's `op(x, y)` on two elements.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, VecOp};
    /// use icepool_core::Value;
    /// // `operator.lt(1, 'a')` raises TypeError; `operator.eq(1, 'a')` is `False`.
    /// assert!(VecOp::Bin(BinOp::Lt).apply(&Value::from(1), &Value::str("a")).is_err());
    /// assert_eq!(VecOp::Eq(false).apply(&Value::from(1), &Value::str("a")).unwrap(), Value::Bool(false));
    /// ```
    pub fn apply(self, x: &Value, y: &Value) -> Result<Value> {
        match self {
            VecOp::Bin(op) => x.binary(op, y),
            VecOp::Eq(ne) => x.eq_op(y, ne),
        }
    }

    /// For a comparison, how its truth value is decided (`Vector.binary_operator`'s `compare_for_truth`).
    fn lexicographic(self) -> Option<Lexicographic> {
        match self {
            VecOp::Eq(false) => Some(Lexicographic::Eq),
            VecOp::Eq(true) => Some(Lexicographic::Ne),
            VecOp::Bin(BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) => Some(Lexicographic::Order),
            VecOp::Bin(_) => None,
        }
    }
}

/// Which comparison a truth value is for, as `_lexicographic_truth` tells them apart (`op is operator.eq`, `op is
/// operator.ne`, anything else).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lexicographic {
    /// `operator.eq`: `False` at the first difference.
    Eq,
    /// `operator.ne`: `True` at the first difference.
    Ne,
    /// Any other comparison: its result on the first difference, as a `bool`.
    Order,
}

/// `_lexicographic_truth(left, right, op, elementwise)`: the truth value of `op(left._data, right._data)` as a
/// `bool`. As Python's tuple comparison, the first pair of elements that are not identical and not equal decides,
/// otherwise the lengths do; unlike it, the deciding comparison is converted to `bool`. Already-computed `==`
/// results (`elementwise`) are reused, so `==` on nested vectors takes linear time.
fn lexicographic_truth(
    left: &[Value],
    right: &[Value],
    kind: Lexicographic,
    op: &impl Fn(&Value, &Value) -> Result<Value>,
    elementwise: Option<&[Value]>,
) -> Result<bool> {
    for (i, (x, y)) in left.iter().zip(right).enumerate() {
        // Rust's `==` stands in for `x is y`, as in `Value::first_difference`.
        if x == y {
            continue;
        }
        let equal = match (kind, elementwise) {
            (Lexicographic::Eq, Some(e)) => e[i].truthy()?,
            _ => x.py_eq(y)?,
        };
        if equal {
            continue;
        }
        return match (kind, elementwise) {
            (Lexicographic::Eq, _) => Ok(false),
            (Lexicographic::Ne, _) => Ok(true),
            (Lexicographic::Order, Some(e)) => e[i].truthy(),
            (Lexicographic::Order, None) => op(x, y)?.truthy(),
        };
    }
    op(&Value::from(left.len() as i64), &Value::from(right.len() as i64))?.truthy()
}

impl Vector {
    /// The vector of `items`.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert_eq!(Value::Vector(Vector::new(vec![Value::from(1)])).to_string(), "Vector((1,))");
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// let v = Vector::truth_only(Value::Bool(true));
    /// assert!(v.truthy().unwrap() && v.data().is_err());
    /// ```
    pub fn truth_only(truth: Value) -> Self {
        Vector {
            data: None,
            truth: Some(Arc::new(truth)),
        }
    }

    /// Whether this is a `VectorWithTruthOnly`: the result of a comparison, which has a truth value but no data.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, VecOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `Vector((1, 2)) < Vector((1, 2, 0))` is `VectorWithTruthOnly(True)`: the lengths differ.
    /// let longer = Value::Vector(Vector::new(vec![Value::from(1), Value::from(2), Value::from(0)]));
    /// let Value::Vector(lt) = v.binary(VecOp::Bin(BinOp::Lt), &longer).unwrap() else { unreachable!() };
    /// assert!(lt.is_truth_only() && !v.is_truth_only());
    /// ```
    pub fn is_truth_only(&self) -> bool {
        self.data.is_none()
    }

    /// `self._data`, which raises for a `VectorWithTruthOnly`. Everything but `bool()` and `repr()` reads it.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// assert_eq!(**v.data().unwrap(), [Value::from(1), Value::from(2)]);
    /// // `VectorWithTruthOnly(True)._data` raises ValueError.
    /// assert!(Vector::truth_only(Value::Bool(true)).data().is_err());
    /// ```
    pub fn data(&self) -> Result<&Arc<[Value]>> {
        self.data.as_ref().ok_or_else(truth_only_error)
    }

    /// `bool(self)`.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, VecOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `bool(Vector((1, 2)) < Vector((1, 3)))` is `True` (by `(1, 2) < (1, 3)`); `bool(Vector((1, 2)))` raises.
    /// let other = Value::Vector(Vector::new(vec![Value::from(1), Value::from(3)]));
    /// assert!(v.binary(VecOp::Bin(BinOp::Lt), &other).unwrap().truthy().unwrap());
    /// assert!(v.truthy().is_err());
    /// ```
    pub fn truthy(&self) -> Result<bool> {
        match self.truth.as_deref() {
            None => Err(Error::WrongType(
                "Vector only has a truth value if it is the result of a comparison operator."
                    .into(),
            )),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(Error::WrongType(format!(
                "__bool__ should return bool, returned {}",
                other.type_name()
            ))),
        }
    }

    /// `type(self).__qualname__`.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert_eq!(Vector::new(vec![]).type_name(), "Vector");
    /// assert_eq!(Vector::truth_only(Value::Bool(false)).type_name(), "VectorWithTruthOnly");
    /// ```
    pub fn type_name(&self) -> &'static str {
        if self.data.is_some() {
            "Vector"
        } else {
            "VectorWithTruthOnly"
        }
    }

    /// `Vector.unary_operator`: elementwise. (`round` goes through `map`.)
    ///
    /// ```
    /// use icepool_core::value::{UnOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `-Vector((1, 2))` is `Vector((-1, -2))`.
    /// assert_eq!(v.unary(UnOp::Neg).unwrap().to_string(), "Vector((-1, -2))");
    /// ```
    pub fn unary(&self, op: UnOp) -> Result<Value> {
        // `__neg__` and `unary_operator`.
        let _frames = frames(2)?;
        self.map(|x| x.unary(op))
    }

    /// `Vector(f(x) for x in self)`.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// assert_eq!(v.map(|x| x.binary(BinOp::Mul, &Value::from(10))).unwrap().to_string(), "Vector((10, 20))");
    /// ```
    pub fn map(&self, f: impl Fn(&Value) -> Result<Value>) -> Result<Value> {
        let data = self.data()?;
        // `Vector.__init__` and the generator expression.
        let _frames = frames(2)?;
        let items = data.iter().map(f).collect::<Result<Vec<_>>>()?;
        Ok(Value::Vector(Vector::new(items)))
    }

    /// `Vector.binary_operator(other, op)` with `self` on the left, for the standard operators: the comparators
    /// give the result a truth value (for a non-vector: `False` for `==`, `True` for `!=`, none for the orderings).
    ///
    /// ```
    /// use icepool_core::value::{BinOp, VecOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `Vector((1, 2)) + Vector((10, 20))` is `Vector((11, 22))`; `Vector((1, 2)) + Vector((1,))` raises IndexError.
    /// let tens = Value::Vector(Vector::new(vec![Value::from(10), Value::from(20)]));
    /// assert_eq!(v.binary(VecOp::Bin(BinOp::Add), &tens).unwrap().to_string(), "Vector((11, 22))");
    /// let one = Value::Vector(Vector::new(vec![Value::from(1)]));
    /// assert!(v.binary(VecOp::Bin(BinOp::Add), &one).is_err());
    /// ```
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
            op.lexicographic(),
            compare_non_vector,
        )
    }

    /// `Vector.binary_operator(other, op, compare_for_truth=..., compare_non_vector=...)` for any `op`.
    ///
    /// If the other operand is also a `Vector`, the operator is applied to each pair of elements from `self` and
    /// `other`. Both must have the same length. Otherwise the other operand is broadcast to each element of `self`.
    /// For a comparison (`compare_for_truth`, which says which), the result also has a truth value based on
    /// lexicographic ordering (`lexicographic_truth`), computed after the elementwise results.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Lexicographic, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `Vector((1, 2)).binary_operator(Vector((1, 3)), operator.lt, compare_for_truth=True)`:
    /// // `Vector((False, True))`, whose truth value is `(1, 2) < (1, 3)`.
    /// let other = Value::Vector(Vector::new(vec![Value::from(1), Value::from(3)]));
    /// let lt = v.binary_operator(&other, |x, y| x.binary(BinOp::Lt, y), Some(Lexicographic::Order), None).unwrap();
    /// assert_eq!(lt.to_string(), "Vector((False, True))");
    /// assert!(lt.truthy().unwrap());
    /// ```
    pub fn binary_operator(
        &self,
        other: &Value,
        op: impl Fn(&Value, &Value) -> Result<Value>,
        compare_for_truth: Option<Lexicographic>,
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
        if data.len() == odata.len() {
            let items = {
                // `Vector.__init__` and the generator expression.
                let _frames = frames(2)?;
                data.iter()
                    .zip(odata.iter())
                    .map(|(x, y)| op(x, y))
                    .collect::<Result<Vec<_>>>()?
            };
            let truth = match compare_for_truth {
                Some(kind) => {
                    // `_lexicographic_truth`.
                    let _frames = frames(1)?;
                    Some(Value::Bool(lexicographic_truth(data, odata, kind, &op, Some(&items))?))
                }
                None => None,
            };
            Ok(Value::Vector(Vector::with_truth(items, truth)))
        } else if let Some(kind) = compare_for_truth {
            let _frames = frames(1)?;
            let truth = lexicographic_truth(data, odata, kind, &op, None)?;
            Ok(Value::Vector(Vector::truth_only(Value::Bool(truth))))
        } else {
            Err(Error::IndexOutOfRange(format!(
                "Binary operators on Vectors are only valid if both are the same length ({} vs. {}).",
                data.len(),
                odata.len()
            )))
        }
    }

    /// `Vector.reverse_binary_operator(other, op)`: `op(other, x)` for each element. Two vectors always use the
    /// left one's `binary_operator` in Python's dispatch; called directly, they must have the same length.
    /// Comparisons never get here from an operator: Python reflects `x < v` to `v > x`.
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `Vector((1, 2)).reverse_binary_operator(10, operator.sub)` is `Vector((9, 8))`.
    /// let r = v.reverse_binary_operator(&Value::from(10), |x, y| x.binary(BinOp::Sub, y)).unwrap();
    /// assert_eq!(r.to_string(), "Vector((9, 8))");
    /// ```
    pub fn reverse_binary_operator(
        &self,
        other: &Value,
        op: impl Fn(&Value, &Value) -> Result<Value>,
    ) -> Result<Value> {
        let data = self.data()?;
        if let Value::Vector(o) = other {
            let odata = o.data()?;
            if data.len() != odata.len() {
                return Err(Error::IndexOutOfRange(format!(
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
    ///
    /// ```
    /// use icepool_core::value::{BinOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `3 - Vector((1, 2))` is `Vector((2, 1))`.
    /// assert_eq!(v.reverse_binary(BinOp::Sub, &Value::from(3)).unwrap().to_string(), "Vector((2, 1))");
    /// ```
    pub fn reverse_binary(&self, op: BinOp, other: &Value) -> Result<Value> {
        // `__radd__` and `reverse_binary_operator`.
        let _frames = frames(2)?;
        self.reverse_binary_operator(other, |x, y| x.binary(op, y))
    }

    /// `self._truth_value`.
    ///
    /// ```
    /// use icepool_core::value::{VecOp, Vector};
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// // `Vector((1, 2))._truth_value` is `None`; `(Vector((1, 2)) == 1)._truth_value` is `False`.
    /// assert!(v.truth().is_none());
    /// let Value::Vector(eq) = v.binary(VecOp::Eq(false), &Value::from(1)).unwrap() else { unreachable!() };
    /// assert_eq!(eq.truth(), Some(&Value::Bool(false)));
    /// ```
    pub fn truth(&self) -> Option<&Value> {
        self.truth.as_deref()
    }

    /// A vector as the facade holds it: its elements and truth value.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// let v = Vector::from_parts(vec![Value::from(1)], Some(Value::Bool(true)));
    /// assert!(v.truthy().unwrap() && !v.is_truth_only());
    /// ```
    pub fn from_parts(items: Vec<Value>, truth: Option<Value>) -> Self {
        Vector::with_truth(items, truth)
    }

    /// Python's `repr()` / `str()` (the same for a vector: `str(tuple)` shows its elements' reprs).
    /// `elem` is `repr` of an element; `str` of the truth value of a `VectorWithTruthOnly` is `str_of`.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// let v = Vector::new(vec![Value::from(1), Value::from(2)]);
    /// let repr = |x: &Value| Ok(x.to_string());
    /// assert_eq!(v.repr_with(repr, repr).unwrap(), "Vector((1, 2))");
    /// assert_eq!(Vector::truth_only(Value::Bool(true)).repr_with(repr, repr).unwrap(), "VectorWithTruthOnly(True)");
    /// ```
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
    Error::InvalidArgument(
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
