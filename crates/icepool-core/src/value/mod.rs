//! `Value`: the dynamic outcome type both bindings use, with Python's semantics inside it (docs/semantics.md).
//!
//! - `bool` is an `int`: `True == 1` and they hash alike, so `Die([True, 1])` has a single outcome.
//! - `int`, `float`, `Fraction` and `bool` compare and merge together *exactly* (Python never rounds an `int` to a
//!   `float` to compare them).
//! - `str` and `tuple` compare only with their own kind; `<` between kinds raises `TypeError`, `==` is just `False`.
//!
//! - icepool's `Vector` and `Symbols` are variants too (`vector.rs`, `symbols.rs`); objects of the host language
//!   the core has no variant for are `Host` (`crate::host`).
//!
//! For language bindings, this module also holds the pieces icepool's Python methods are assembled from (die
//! construction from `Die(...)` arguments, `Again` evaluation, cartesian products, formatting, `map(repeat=...)`,
//! the free functions of function.py): a binding keeps Python's argument handling and calls these for the work.

pub mod builtins;
mod complex;
mod die;
pub mod evaluators;
pub(crate) mod expand;
pub(crate) mod format;
pub(crate) mod function;
mod map_state;
mod matmul;
mod number;
pub mod ops;
mod order;
pub mod pystr;
pub mod recursion;
mod repr;
pub mod sort;
mod statistics;
mod symbols;
pub(crate) mod to_pool;
mod vector;

use crate::ownership::{Owners, Traverse};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;
use dashu_ratio::RBig;

use crate::error::Result;
use crate::host::HostRef;

/// Python's `collections.Counter`, as `Deck` arithmetic uses it.
pub use crate::deck::{Counter, CounterOp};
pub use die::{Data, PyDie};
pub use matmul::matmul_dice;
pub use number::{int_to_f64, ratio_to_f64, Num};
pub use ops::{BinOp, UnOp};
pub use repr::float_repr;
pub use statistics::PyStatistics;
pub use symbols::Symbols;
pub use vector::{Lexicographic, VecOp, Vector};

// For language bindings (and the program runner): the pieces icepool's Python methods are assembled from. A binding
// keeps Python's argument juggling (its own types, keyword arguments, callbacks) and calls these for the work. The
// generic ones work on typed dice too; by icepool file:
// creation_args.py, collection/counts.py
pub use crate::counts::{Counts, OrderedMerge};
pub use crate::creation_args::{expand_args_for_die, DieArg};
// population/again.py
pub use crate::again::{
    evaluate_agains_using_count, evaluate_agains_using_depth, step_outcomes, AgainArg, AgainExpression, StepOutcome,
};
// expand.py
pub use crate::expand::{iter_cartesian_product, product_size};
pub use expand::{cartesian_product_items, tupleize_dice, vectorize_dice};
// population/format.py
pub use crate::format::{common_outcome_length, format as format_population, Formatted};
pub use format::format_probability_inverse;
// function.py, math.py
pub use crate::function::{pointwise, sorted_union};
pub use function::{coin_data, consecutive, one_hot, weighted_lcm};
// population/keep.py
pub use crate::keep::{highest_single, lowest_single};
// map_tools, markov_chain.py
pub use crate::map_tools::{absorbing_markov_chain, map_and_time, map_repeat, MapState, Transition, TransitionCache};
// Die.explode_to_pool, Die.reroll_to_pool
pub use to_pool::{explode_counts, reroll_composition};
// population/base.py
pub use crate::population::check_public_attr;
// Allocations sized by user input: Python's MemoryError instead of an abort.
pub use crate::memory::{check_len, try_collect, try_repeat};

/// An outcome with Python's semantics: the dynamic outcome type of the language bindings. Its comparisons and
/// operators are Python's and may raise.
///
/// ```
/// use icepool_core::Value;
/// assert_eq!(Value::from(1), Value::Bool(true)); // `1 == True`, as dict keys
/// ```
#[derive(Clone, Debug)]
pub enum Value {
    /// Python's `None`. Not a valid outcome by itself (`Counts` rejects it), but it may sit inside a tuple.
    None,
    /// Python's `bool`.
    Bool(bool),
    /// Python's `int`.
    Int(IBig),
    /// Python's `float`.
    Float(f64),
    /// Always normalised (dashu keeps it in lowest terms with a positive denominator), like Python's `Fraction`.
    Fraction(RBig),
    /// Python's `str`.
    Str(Arc<str>),
    /// Python's `tuple`.
    Tuple(Arc<[Value]>),
    /// Python's `complex`, which arises from `**` (a negative number to a fractional power) and then from
    /// arithmetic on it (`complex.rs`). It is not orderable, so a die with a complex outcome raises when it sorts
    /// its outcomes (unless its quantity is zero and it drops out first).
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
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert_eq!(Value::from(1).type_name(), "int");
    /// assert_eq!(Value::Vector(Vector::new(vec![])).type_name(), "Vector");
    /// ```
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
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `True` is the int 1, and hashes as `1.0` does.
    /// assert_eq!(Value::Bool(true).as_num().unwrap().py_hash(), Value::Float(1.0).as_num().unwrap().py_hash());
    /// assert!(Value::str("1").as_num().is_none());
    /// ```
    pub fn as_num(&self) -> Option<Num<'_>> {
        match self {
            Value::Bool(b) => Some(Num::Int(std::borrow::Cow::Owned(IBig::from(*b as u8)))),
            Value::Int(i) => Some(Num::Int(std::borrow::Cow::Borrowed(i))),
            Value::Float(f) => Some(Num::Float(*f)),
            Value::Fraction(r) => Some(Num::Fraction(r)),
            _ => None,
        }
    }

    /// The `str` `s`.
    ///
    /// ```
    /// use icepool_core::Value;
    /// assert!(matches!(Value::str("a"), Value::Str(s) if &*s == "a"));
    /// ```
    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    /// The `tuple` of `items`.
    ///
    /// ```
    /// use icepool_core::Value;
    /// assert_eq!(Value::tuple(vec![Value::from(1)]).to_string(), "(1,)");
    /// ```
    pub fn tuple(items: Vec<Value>) -> Value {
        Value::Tuple(Arc::from(items))
    }

    /// Python's `bool(x)`, which raises for a `Vector` that is not the result of a comparison.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert!(!Value::tuple(vec![]).truthy().unwrap());
    /// assert!(Value::Float(0.5).truthy().unwrap());
    /// // `bool(Vector((1, 2)))` raises TypeError: it is not the result of a comparison.
    /// assert!(Value::Vector(Vector::new(vec![Value::from(1), Value::from(2)])).truthy().is_err());
    /// ```
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
    ///
    /// This is also where an outcome nested too deep raises `RecursionError` (`recursion.rs`): a die never holds
    /// one, so the infallible recursions on its outcomes (`==`, `hash`) stay shallow.
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert!(Value::tuple(vec![Value::from(1)]).check_hashable().is_ok());
    /// // `hash((VectorWithTruthOnly(True),))` raises ValueError.
    /// let truth_only = Value::Vector(Vector::truth_only(Value::Bool(true)));
    /// assert!(Value::tuple(vec![truth_only]).check_hashable().is_err());
    /// ```
    pub fn check_hashable(&self) -> Result<()> {
        match self {
            Value::Vector(v) => {
                let _level = recursion::level()?;
                v.data()?.iter().try_for_each(Value::check_hashable)
            }
            Value::Tuple(t) => {
                let _level = recursion::level()?;
                t.iter().try_for_each(Value::check_hashable)
            }
            _ => Ok(()),
        }
    }

    /// Python's sequence comparison: the first position where the elements of two tuples differ by `==`
    /// (`PyObject_RichCompare(v, w, Py_EQ)`, which for a `Vector` or `Symbols` is its own `__eq__`: `Symbols('ab')
    /// == ('a', 'b')`, so those tuple elements do not decide). Elements equal as dict keys are equal for Python too
    /// (which also stands in for its identity shortcut), so only the others ask `py_eq`, which may raise.
    ///
    /// ```
    /// use icepool_core::value::Symbols;
    /// use icepool_core::Value;
    /// let (a, b) = ([1, 2, 3].map(Value::from), [1, 5, 0].map(Value::from));
    /// assert_eq!(Value::first_difference(&a, &b).unwrap(), Some((&a[1], &b[1])));
    /// // `Symbols('ab') == ('a', 'b')`, so `(Symbols('ab'),)` and `(('a', 'b'),)` have no difference.
    /// let s = [Value::Symbols(Symbols::parse("ab").unwrap())];
    /// let t = [Value::tuple(vec![Value::str("a"), Value::str("b")])];
    /// assert_eq!(Value::first_difference(&s, &t).unwrap(), None);
    /// ```
    pub fn first_difference<'a>(
        a: &'a [Value],
        b: &'a [Value],
    ) -> Result<Option<(&'a Value, &'a Value)>> {
        for (x, y) in a.iter().zip(b) {
            if x != y && !x.py_eq(y)? {
                return Ok(Some((x, y)));
            }
        }
        Ok(None)
    }

    /// Whether this is a `Vector` or `Symbols`, whose operators follow their own classes (`ops::collection_binary`).
    ///
    /// ```
    /// use icepool_core::value::Vector;
    /// use icepool_core::Value;
    /// assert!(Value::Vector(Vector::new(vec![])).is_collection());
    /// assert!(!Value::tuple(vec![]).is_collection());
    /// ```
    pub fn is_collection(&self) -> bool {
        matches!(self, Value::Vector(_) | Value::Symbols(_))
    }

    /// Python's `max(a, b)`: `b` only if `b > a`, so the first of equal values wins.
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `max(1, True)` is `1`: equal, so the first wins.
    /// assert!(matches!(Value::py_max(Value::from(1), Value::Bool(true)).unwrap(), Value::Int(_)));
    /// // `max(1, 'a')` raises TypeError.
    /// assert!(Value::py_max(Value::from(1), Value::str("a")).is_err());
    /// ```
    pub fn py_max(a: Value, b: Value) -> crate::Result<Value> {
        Ok(if ops::compare(BinOp::Gt, &b, &a)? {
            b
        } else {
            a
        })
    }

    /// Python's `min(a, b)`: `b` only if `b < a`.
    ///
    /// ```
    /// use icepool_core::Value;
    /// // `min(True, 1)` is `True`.
    /// assert!(matches!(Value::py_min(Value::Bool(true), Value::from(1)).unwrap(), Value::Bool(true)));
    /// assert_eq!(Value::py_min(Value::Float(2.5), Value::from(2)).unwrap(), Value::from(2));
    /// ```
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
///
/// It recurses on tuples and vectors, so it keeps a small frame: everything else is `scalar_eq`'s (`recursion.rs`).
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
            }
            (Value::Vector(a), Value::Vector(b)) => a == b,
            _ => scalar_eq(self, other),
        }
    }
}

#[inline(never)]
fn scalar_eq(this: &Value, other: &Value) -> bool {
    if let Some(h) = crate::host::host_of(this, other) {
        return h.eq(this, other);
    }
    match (this, other) {
        (Value::None, Value::None) => true,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Complex(ar, ai), Value::Complex(br, bi)) => ar == br && ai == bi,
        // As a dict sees them: a vector (two are `eq`'s) or a `Symbols` only ever equals its own kind. (Python's
        // `==` also makes `Symbols('ab') == 'ab'`, but their hashes differ, so a dict never merges them; `eq_op`
        // is the operator.)
        (Value::Symbols(a), Value::Symbols(b)) => a == b,
        (Value::Vector(_) | Value::Symbols(_), _) | (_, Value::Vector(_) | Value::Symbols(_)) => {
            false
        }
        // A complex number equals a real one when its imaginary part is zero.
        (Value::Complex(re, im), x) | (x, Value::Complex(re, im)) => {
            *im == 0.0 && x.as_num().is_some_and(|n| n == Num::Float(*re))
        }
        _ => match (this.as_num(), other.as_num()) {
            (Some(a), Some(b)) => a.partial_cmp(&b) == Some(Ordering::Equal),
            _ => false,
        },
    }
}

/// Python's `==` is an equivalence on everything except float NaN, which Python also treats as unequal to itself
/// (bar object identity, which the port does not model; see docs/NOTES.md).
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
            // the two do not merge as they would in a Python dict (see docs/NOTES.md).
            Value::Host(h) => {
                state.write_u8(0);
                state.write_i64(h.hash_value());
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

/// Python's `repr`.
/// What a value holds that may hold host objects: tuples, vectors, symbols, and host objects themselves.
impl Traverse for Value {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        match self {
            Value::Tuple(items) => owners.arc(items),
            Value::Vector(v) => v.traverse(owners),
            Value::Symbols(s) => s.traverse(owners),
            Value::Host(h) => owners.host(&h.0, h.0.as_any()),
            Value::None
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Fraction(_)
            | Value::Str(_)
            | Value::Complex(..) => {}
        }
    }
}

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
            // Nested too deep for `recursion.rs`, where Python's `repr` raises: an ellipsis (this is for messages).
            Value::Vector(_) | Value::Tuple(_) if recursion::level().is_err() => write!(f, "..."),
            Value::Vector(v) => {
                let _level = recursion::level();
                match v.repr_with(|x| Ok(x.to_string()), |x| Ok(x.to_string())) {
                    Ok(s) => write!(f, "{s}"),
                    Err(_) => write!(f, "Vector(?)"),
                }
            }
            Value::Symbols(s) => match s.py_repr() {
                Ok(r) => write!(f, "{r}"),
                Err(_) => write!(f, "Symbols(?)"),
            },
            Value::Tuple(t) => {
                let _level = recursion::level();
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

    #[test]
    fn type_names_are_pythons() {
        let names: Vec<&str> = [
            Value::None,
            Value::Bool(true),
            Value::from(1),
            Value::Float(0.5),
            Value::str("a"),
            Value::tuple(vec![]),
            Value::Complex(0.0, 1.0),
            Value::Vector(Vector::new(vec![])),
        ]
        .iter()
        .map(Value::type_name)
        .collect();
        assert_eq!(
            names,
            ["NoneType", "bool", "int", "float", "str", "tuple", "complex", "Vector"]
        );
    }

    #[test]
    fn collections_hash_by_their_contents() {
        // Not needed for correctness (unequal values may collide), but dice of vectors or symbols would degrade
        // to linear lookups if every one hashed alike.
        let v =
            |xs: &[i64]| Value::Vector(Vector::new(xs.iter().map(|&x| Value::from(x)).collect()));
        assert_eq!(h(&v(&[1, 2])), h(&v(&[1, 2])));
        assert_ne!(h(&v(&[1, 2])), h(&v(&[2, 1])));
        assert_ne!(h(&v(&[0])), h(&Value::from(0)));
        let s = |x: &str| Value::Symbols(Symbols::convert(&Value::str(x)).unwrap());
        assert_eq!(h(&s("ab")), h(&s("ba")));
        assert_ne!(h(&s("ab")), h(&s("abb")));
        assert_ne!(h(&s("")), h(&Value::from(0)));
    }

    #[test]
    fn dict_keys_reject_none_and_unhashable_vectors() {
        // `Counts` raises these itself; sorting usually raises the same error type first (`None < None`, the
        // truth value of a `VectorWithTruthOnly`), so fixtures cannot see this check.
        assert!(Value::None.py_check_key().is_err());
        let truth_only = Value::Vector(Vector::truth_only(Value::Bool(true)));
        assert!(truth_only.py_check_key().is_err());
        assert!(Value::tuple(vec![Value::from(1), truth_only])
            .py_check_key()
            .is_err());
        assert!(Value::tuple(vec![Value::from(1)]).py_check_key().is_ok());
    }

    #[test]
    fn a_vector_compared_with_a_scalar_raises_the_vectors_error() {
        // `Vector([1]) < 2` is a vector without a truth value: its own TypeError, not "'<' not supported".
        let e = Value::Vector(Vector::new(vec![Value::from(1)]))
            .py_lt(&Value::from(2))
            .unwrap_err();
        assert!(e.to_string().contains("truth value"), "{e}");
    }

    #[test]
    fn zero_numbers_are_falsy() {
        assert!(!Value::Float(0.0).truthy().unwrap() && Value::Float(-0.5).truthy().unwrap());
        assert!(
            !Value::Fraction(RBig::ZERO).truthy().unwrap()
                && Value::Fraction(RBig::ONE).truthy().unwrap()
        );
    }
}
