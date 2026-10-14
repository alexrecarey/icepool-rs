//! `Value`: the dynamic outcome type both bindings use, with Python's semantics inside it (docs/semantics.md).
//!
//! - `bool` is an `int`: `True == 1` and they hash alike, so `Die([True, 1])` has a single outcome.
//! - `int`, `float`, `Fraction` and `bool` compare and merge together *exactly* (Python never rounds an `int` to a
//!   `float` to compare them).
//! - `str` and `tuple` compare only with their own kind; `<` between kinds raises `TypeError`, `==` is just `False`.
//!
//! Later kinds (Vector, Symbols, host objects) are added as new variants.

mod number;
mod repr;

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;
use dashu_ratio::RBig;

use crate::error::{type_error, Result};
use crate::outcome::Outcome;

pub use number::Num;
pub use repr::float_repr;

#[derive(Clone, Debug)]
pub enum Value {
    /// Python's `None`. Not a valid outcome by itself (`Counts` rejects it), but it may sit inside a tuple.
    None,
    Bool(bool),
    Int(IBig),
    Float(f64),
    /// Always normalised (dashu keeps it in lowest terms with a positive denominator), like Python's `Fraction`.
    Fraction(RBig),
    Str(Arc<str>),
    Tuple(Arc<[Value]>),
}

impl Value {
    /// Python's `type(x).__name__`, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::None => "NoneType",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Fraction(_) => "Fraction",
            Value::Str(_) => "str",
            Value::Tuple(_) => "tuple",
        }
    }

    /// The numeric view of `bool`, `int`, `float` and `Fraction`; `None` for everything else.
    pub fn as_num(&self) -> Option<Num<'_>> {
        match self {
            Value::Bool(b) => Some(Num::Int(std::borrow::Cow::Owned(IBig::from(*b as u8)))),
            Value::Int(i) => Some(Num::Int(std::borrow::Cow::Borrowed(i))),
            Value::Float(f) => Some(Num::Float(*f)),
            Value::Fraction(r) => Some(Num::Fraction(r)),
            _ => None,
        }
    }

    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn tuple(items: Vec<Value>) -> Value {
        Value::Tuple(Arc::from(items))
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(IBig::from(i))
    }
}

impl From<IBig> for Value {
    fn from(i: IBig) -> Self {
        Value::Int(i)
    }
}

/// Python's `==`. Never raises: values of unrelated kinds are simply unequal.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
            }
            _ => match (self.as_num(), other.as_num()) {
                (Some(a), Some(b)) => a.partial_cmp(&b) == Some(Ordering::Equal),
                _ => false,
            },
        }
    }
}

/// Python's `==` is an equivalence on everything except float NaN, which Python also treats as unequal to itself
/// (bar object identity, which the port does not model; see NOTES.md).
impl Eq for Value {}

/// Agrees with `==`: equal numbers of any type hash alike (Python's numeric hash, `number::py_hash`).
impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Str(s) => {
                state.write_u8(1);
                s.hash(state);
            }
            Value::None => state.write_u8(3),
            Value::Tuple(t) => {
                state.write_u8(2);
                state.write_usize(t.len());
                t.iter().for_each(|x| x.hash(state));
            }
            _ => {
                state.write_u8(0);
                state.write_i64(self.as_num().map_or(0, |n| n.py_hash()));
            }
        }
    }
}

impl Outcome for Value {
    fn check_key(&self) -> Result<()> {
        match self {
            Value::None => type_error("None is not a valid key."),
            _ => Ok(()),
        }
    }

    fn py_lt(&self, other: &Self) -> Result<bool> {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => Ok(a < b),
            // Python's sequence comparison: the first position where the elements differ (by `==`) decides with
            // `<`, which may itself raise; if one is a prefix of the other, the shorter is smaller.
            (Value::Tuple(a), Value::Tuple(b)) => {
                match a.iter().zip(b.iter()).find(|(x, y)| x != y) {
                    Some((x, y)) => x.py_lt(y),
                    None => Ok(a.len() < b.len()),
                }
            }
            _ => match (self.as_num(), other.as_num()) {
                (Some(a), Some(b)) => Ok(a.partial_cmp(&b) == Some(Ordering::Less)),
                _ => type_error(format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    self.type_name(),
                    other.type_name()
                )),
            },
        }
    }
}

/// Python's `repr`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => write!(f, "None"),
            Value::Bool(b) => write!(f, "{}", if *b { "True" } else { "False" }),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{}", float_repr(*x)),
            Value::Fraction(r) => write!(f, "Fraction({}, {})", r.numerator(), r.denominator()),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Tuple(t) => {
                write!(f, "(")?;
                for (i, x) in t.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{x}")?;
                }
                if t.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn h(v: &Value) -> u64 {
        let mut s = DefaultHasher::new();
        v.hash(&mut s);
        s.finish()
    }

    #[test]
    fn bool_int_float_fraction_are_one_number() {
        let vals = [
            Value::Bool(true),
            Value::from(1),
            Value::Float(1.0),
            Value::Fraction(RBig::ONE),
        ];
        for a in &vals {
            for b in &vals {
                assert_eq!(a, b);
                assert_eq!(h(a), h(b));
            }
        }
        let half = Value::Fraction(RBig::from_parts(IBig::from(1), 2u8.into()));
        assert_eq!(half, Value::Float(0.5));
        assert_eq!(h(&half), h(&Value::Float(0.5)));
    }

    #[test]
    fn incomparable_kinds_raise() {
        assert!(Value::from(1).py_lt(&Value::str("a")).is_err());
        assert_ne!(Value::from(1), Value::str("a"));
        // Tuples decide at the first differing element, so this one never compares 'a' with 2.
        let a = Value::tuple(vec![Value::from(1), Value::str("a")]);
        let b = Value::tuple(vec![Value::from(2), Value::from(3)]);
        assert_eq!(a.py_lt(&b), Ok(true));
    }
}
