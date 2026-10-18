//! Python's comparisons and arithmetic on `Value`, and with them `Value`'s implementation of the outcome traits:
//! the only outcome type whose order can refuse (`1 < 'a'` raises `TypeError`, a host class's `__lt__` can raise
//! anything).

use std::cmp::Ordering;

use super::{ops, recursion, sort, BinOp, UnOp, Value};
use crate::error::{wrong_type, Error, Result};
use crate::outcome::{OutcomeArith, OutcomeOrder};
use crate::Int;

impl Value {
    /// `Counts` rejects `None` (and, through `check_hashable`, unhashable values) as a key with `TypeError`.
    ///
    /// ```
    /// use icepool_core::Value;
    /// assert!(Value::from(1).py_check_key().is_ok());
    /// assert!(Value::None.py_check_key().is_err()); // TypeError: None is not a valid key.
    /// ```
    pub fn py_check_key(&self) -> Result<()> {
        match self {
            Value::None => wrong_type("None is not a valid key."),
            _ => self.check_hashable(),
        }
    }

    /// Python's `self <= other` (`Die.clip`).
    ///
    /// ```
    /// use icepool_core::Value;
    /// assert!(Value::Bool(true).py_le(&Value::from(1)).unwrap()); // `True <= 1`
    /// assert!(Value::from(1).py_le(&Value::str("a")).is_err()); // TypeError
    /// ```
    pub fn py_le(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Le, self, other)
    }

    /// Python's `bool(self == other)`, as a comparison of tuples asks it. `Eq` is the equality of dict keys,
    /// which also requires equal hashes: the two differ for a few types (`Symbols('a') == 'a'`, but their hashes
    /// differ).
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    /// // `Symbols('a') == 'a'` (though they are different dict keys), and `1 == 1.0`.
    /// let a = Value::Symbols(Symbols::parse("a").unwrap());
    /// assert!(a.py_eq(&Value::str("a")).unwrap() && a != Value::str("a"));
    /// assert!(Value::from(1).py_eq(&Value::Float(1.0)).unwrap());
    /// ```
    pub fn py_eq(&self, other: &Self) -> Result<bool> {
        if self.is_collection() || other.is_collection() {
            return self.eq_op(other, false)?.truthy();
        }
        Ok(self == other)
    }

    /// Python's `self >= other`.
    ///
    /// ```
    /// use icepool_core::Value;
    /// assert!(!Value::Float(1.5).py_ge(&Value::from(2)).unwrap());
    /// ```
    pub fn py_ge(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Ge, self, other)
    }

    /// Python's `self > other` (`max()`).
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `'b' > 'a'`; `(1,) > 'a'` raises TypeError.
    /// assert!(Value::str("b").py_gt(&Value::str("a")).unwrap());
    /// assert!(Value::tuple(vec![Value::from(1)]).py_gt(&Value::str("a")).is_err());
    /// ```
    pub fn py_gt(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Gt, self, other)
    }

    /// Python's `self < other`. Incomparable values (an `int` and a `str`) raise `TypeError`, as in Python.
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `(1, 2, 3) < (1, 5, 0)`: the first difference decides.
    /// let (a, b) = (Value::tuple([1, 2, 3].map(Value::from).into()), Value::tuple([1, 5, 0].map(Value::from).into()));
    /// assert!(a.py_lt(&b).unwrap());
    /// assert!(Value::from(1).py_lt(&Value::str("a")).is_err());
    /// ```
    pub fn py_lt(&self, other: &Self) -> Result<bool> {
        if let Some(h) = crate::host::host_of(self, other) {
            return h.compare(BinOp::Lt, self, other);
        }
        if self.is_collection() || other.is_collection() {
            return ops::compare(BinOp::Lt, self, other);
        }
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => Ok(a < b),
            // Python's sequence comparison: the first position where the elements differ (by `==`) decides with
            // `<`, which may itself raise; if one is a prefix of the other, the shorter is smaller.
            (Value::Tuple(a), Value::Tuple(b)) => {
                let _level = recursion::level()?;
                match Value::first_difference(a, b)? {
                    Some((x, y)) => x.py_lt(y),
                    None => Ok(a.len() < b.len()),
                }
            }
            _ => match (self.as_num(), other.as_num()) {
                (Some(a), Some(b)) => Ok(a.partial_cmp(&b) == Some(Ordering::Less)),
                _ => Err(Error::Incomparable {
                    op: "<",
                    left: self.type_name(),
                    right: other.type_name(),
                }),
            },
        }
    }
}

impl OutcomeOrder for Value {
    type Incomparable = Error;

    fn try_lt(&self, other: &Self) -> Result<bool> {
        self.py_lt(other)
    }

    fn try_le(&self, other: &Self) -> Result<bool> {
        self.py_le(other)
    }

    fn try_gt(&self, other: &Self) -> Result<bool> {
        self.py_gt(other)
    }

    fn try_ge(&self, other: &Self) -> Result<bool> {
        self.py_ge(other)
    }

    fn try_eq(&self, other: &Self) -> Result<bool> {
        self.py_eq(other)
    }

    fn check_key(&self) -> Result<()> {
        self.py_check_key()
    }

    fn try_sort_by<T>(items: Vec<T>, lt: impl Fn(&T, &T) -> Result<bool>) -> Result<Vec<T>> {
        sort::timsort(items, lt)
    }
}

/// Python's operators.
impl OutcomeArith for Value {
    fn try_add(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::Add, other)
    }

    fn try_sub(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::Sub, other)
    }

    fn try_neg(&self) -> Result<Self> {
        self.unary(UnOp::Neg)
    }

    fn try_mul(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::Mul, other)
    }

    fn try_mul_count(&self, count: &Int) -> Result<Self> {
        self.binary(BinOp::Mul, &Value::from(count))
    }

    fn try_zero(&self) -> Result<Self> {
        self.binary(BinOp::Mul, &Value::from(0))
    }

    fn try_floor_div(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::FloorDiv, other)
    }

    fn try_pow(&self, other: &Self) -> Result<Self> {
        self.binary(BinOp::Pow, other)
    }
}
