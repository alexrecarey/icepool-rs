//! `Value`: the dynamic outcome type both bindings use, with Python's semantics inside it (docs/semantics.md).
//!
//! - `bool` is an `int`: `True == 1` and they hash alike, so `Die([True, 1])` has a single outcome.
//! - `int`, `float`, `Fraction` and `bool` compare and merge together *exactly* (Python never rounds an `int` to a
//!   `float` to compare them).
//! - `str` and `tuple` compare only with their own kind; `<` between kinds raises `TypeError`, `==` is just `False`.
//!
//! - icepool's `Vector` and `Symbols` are variants too (`vector.rs`, `symbols.rs`); objects of the host language
//!   the core has no variant for are `Host` (`host.rs`).

pub mod builtins;
mod host;
pub mod libm;
mod number;
pub mod ops;
pub mod pow;
pub mod pystr;
mod repr;
mod symbols;
mod vector;

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;
use dashu_ratio::RBig;

use crate::error::{type_error, Result};
use crate::outcome::Outcome;

pub use host::{HostError, HostObject, HostRef, LenKind};
pub use number::{int_to_f64, ratio_to_f64, Num};
pub use ops::{BinOp, UnOp};
pub use repr::float_repr;
pub use symbols::Symbols;
pub use vector::{VecOp, Vector};

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
    /// Python's `complex`, which only arises from `**` (a negative number to a fractional power). It is not
    /// orderable, so a die with a complex outcome raises when it sorts its outcomes (unless its quantity is zero
    /// and it drops out first). Arithmetic on it is not ported.
    Complex(f64, f64),
    /// icepool's `Vector` (or `VectorWithTruthOnly`): operators apply elementwise.
    Vector(Vector),
    /// icepool's `Symbols`: a multiset of characters.
    Symbols(Symbols),
    /// An object of the host language the core has no variant for (see `host.rs`). Every operator and comparison
    /// involving one is answered by the host.
    Host(HostRef),
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
            Value::Complex(..) => "complex",
            Value::Vector(v) => v.type_name(),
            Value::Symbols(_) => "Symbols",
            Value::Host(_) => "object",
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

    /// Python's `bool(x)`, which raises for a `Vector` that is not the result of a comparison.
    pub fn truthy(&self) -> Result<bool> {
        Ok(match self {
            Value::None => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != IBig::ZERO,
            Value::Float(f) => *f != 0.0,
            Value::Fraction(r) => *r != dashu_ratio::RBig::ZERO,
            Value::Str(s) => !s.is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Complex(re, im) => *re != 0.0 || *im != 0.0,
            Value::Vector(v) => v.truthy()?,
            // A mapping: its number of keys (zero counts included).
            Value::Symbols(s) => !s.is_empty(),
            Value::Host(h) => h.truthy()?,
        })
    }

    /// Whether Python can hash this value: hashing a `VectorWithTruthOnly` raises `ValueError` (it reads `_data`),
    /// also inside a tuple or a vector.
    pub fn check_hashable(&self) -> Result<()> {
        match self {
            Value::Vector(v) => v.data()?.iter().try_for_each(Value::check_hashable),
            Value::Tuple(t) => t.iter().try_for_each(Value::check_hashable),
            _ => Ok(()),
        }
    }

    /// Whether this is a `Vector` or `Symbols`, whose operators follow their own classes (`ops::collection_binary`).
    pub fn is_collection(&self) -> bool {
        matches!(self, Value::Vector(_) | Value::Symbols(_))
    }

    /// Python's `max(a, b)`: `b` only if `b > a`, so the first of equal values wins.
    pub fn py_max(a: Value, b: Value) -> crate::Result<Value> {
        Ok(if ops::compare(BinOp::Gt, &b, &a)? {
            b
        } else {
            a
        })
    }

    /// Python's `min(a, b)`: `b` only if `b < a`.
    pub fn py_min(a: Value, b: Value) -> crate::Result<Value> {
        Ok(if ops::compare(BinOp::Lt, &b, &a)? {
            b
        } else {
            a
        })
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
        if let Some(h) = host::host_of(self, other) {
            return h.eq(self, other);
        }
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
            }
            (Value::Complex(ar, ai), Value::Complex(br, bi)) => ar == br && ai == bi,
            // As a dict sees them: a vector or a `Symbols` only ever equals its own kind. (Python's `==` also makes
            // `Symbols('ab') == 'ab'`, but their hashes differ, so a dict never merges them; `eq_op` is the
            // operator.)
            (Value::Vector(a), Value::Vector(b)) => a == b,
            (Value::Symbols(a), Value::Symbols(b)) => a == b,
            (Value::Vector(_) | Value::Symbols(_), _)
            | (_, Value::Vector(_) | Value::Symbols(_)) => false,
            // A complex number equals a real one when its imaginary part is zero.
            (Value::Complex(re, im), x) | (x, Value::Complex(re, im)) => {
                *im == 0.0 && x.as_num().is_some_and(|n| n == Num::Float(*re))
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
            // CPython: hash(real) + 1000003 * hash(imag), so a zero imaginary part hashes as the real number.
            Value::Complex(re, im) => {
                state.write_u8(0);
                let h = Num::Float(*re)
                    .py_hash()
                    .wrapping_add(1000003i64.wrapping_mul(Num::Float(*im).py_hash()));
                state.write_i64(if h == -1 { -2 } else { h });
            }
            // Python's hash, fed like a number's: a host object equal to a number (an `IntEnum`) hashes alike, as
            // Python requires. One equal to a `str` or a plain `tuple` (a `StrEnum`, a namedtuple) does not, so
            // the two do not merge as they would in a Python dict (see NOTES.md).
            Value::Host(h) => {
                state.write_u8(0);
                state.write_i64(h.py_hash());
            }
            Value::Tuple(t) => {
                state.write_u8(2);
                state.write_usize(t.len());
                t.iter().for_each(|x| x.hash(state));
            }
            // Python hashes `(Vector, self._data)`. (A `VectorWithTruthOnly` cannot be hashed: `check_key`.)
            Value::Vector(v) => {
                state.write_u8(4);
                if let Ok(d) = v.data() {
                    state.write_usize(d.len());
                    d.iter().for_each(|x| x.hash(state));
                }
            }
            // Python hashes `(Symbols, str(self))`: the nonzero counts in key order.
            Value::Symbols(s) => {
                state.write_u8(5);
                for (c, n) in s.nonzero_sorted() {
                    state.write_u32(c as u32);
                    n.hash(state);
                }
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
            _ => self.check_hashable(),
        }
    }

    fn py_le(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Le, self, other)
    }

    fn py_ge(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Ge, self, other)
    }

    fn py_gt(&self, other: &Self) -> Result<bool> {
        ops::compare(BinOp::Gt, self, other)
    }

    fn py_lt(&self, other: &Self) -> Result<bool> {
        if let Some(h) = host::host_of(self, other) {
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
            Value::Complex(re, im) => write!(f, "{}", ops::complex_repr(*re, *im)),
            Value::Host(h) => write!(f, "{}", h.repr()),
            Value::Vector(v) => match v.repr_with(|x| Ok(x.to_string()), |x| Ok(x.to_string())) {
                Ok(s) => write!(f, "{s}"),
                Err(_) => write!(f, "Vector(?)"),
            },
            Value::Symbols(s) => match s.py_repr() {
                Ok(r) => write!(f, "{r}"),
                Err(_) => write!(f, "Symbols(?)"),
            },
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
