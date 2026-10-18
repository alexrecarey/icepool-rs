//! The outcome traits: what the generic core needs from an outcome type.
//!
//! icepool requires outcomes to be hashable and totally orderable under Python's `<` (typing.py `Outcome`), and
//! merges outcomes that are `==` (and hash equal), e.g. `True` and `1`. A Rust type with a total order (`Ord`) has
//! all of that and its comparisons cannot fail; the dynamic [`crate::Value`] has Python's semantics, where `1 <
//! 'a'` raises `TypeError`. The generic algorithms are written once against [`OutcomeOrder`], whose refusal to
//! compare is an associated type: [`Infallible`] for every `Ord` type, [`Error`] for `Value`. The typed API asks
//! for [`Outcome`] (an `OutcomeOrder` that cannot fail) and returns plain values; see
//! docs/design/library-architecture.md section 1.

use std::convert::Infallible;
use std::fmt::Debug;
use std::hash::Hash;

use crate::error::{Error, Result};
use crate::{Int, Ratio};

mod sealed {
    /// `OutcomeOrder` has exactly two implementations: every `Ord` type and `Value`.
    pub trait Sealed {}
    impl<T: Ord> Sealed for T {}
    impl Sealed for crate::Value {}
}

/// What the core's algorithms need from an outcome: hashing, equality, and an order that may refuse.
///
/// Implemented for every `Ord + Hash + Clone + Debug + Send + Sync + 'static` type (a total order that cannot
/// fail) and for [`crate::Value`], whose order is Python's `<` and can raise. Sealed: these are the only two.
///
/// `Eq` and `Hash` are the equality of dict keys: values of different types may be equal (`True == 1 == 1.0`),
/// and `Hash` agrees with it. (Like Python, `Value`'s float NaN is not equal to itself; see dev/NOTES.md.)
///
/// ```
/// use icepool_core::{OutcomeOrder, Value};
/// // Every `Ord` type: a comparison that cannot fail.
/// assert_eq!(1i64.try_lt(&2), Ok(true));
/// // `Value` compares as Python: `1 < 'a'` raises TypeError.
/// let refused = Value::from(1).try_lt(&Value::str("a")).unwrap_err();
/// assert_eq!(refused.to_string(), "'<' not supported between instances of 'int' and 'str'");
/// ```
pub trait OutcomeOrder: Clone + Eq + Hash + Debug + Send + Sync + 'static + sealed::Sealed {
    /// Why two outcomes could not be compared: `Infallible` for `Ord` types, `Error` for `Value`.
    ///
    /// ```
    /// use icepool_core::{Error, OutcomeOrder, Value};
    /// use std::convert::Infallible;
    /// fn refusal<O: OutcomeOrder>(a: &O, b: &O) -> Option<Error> {
    ///     a.try_lt(b).err().map(Into::into)
    /// }
    /// let _: Option<Infallible> = 1i64.try_lt(&2).err(); // `Ord` types never refuse
    /// assert_eq!(refusal(&1i64, &2), None);
    /// assert!(matches!(refusal(&Value::from(1), &Value::str("a")), Some(Error::Incomparable { .. })));
    /// ```
    type Incomparable: Into<Error>;

    /// `self < other`.
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!(1i64.try_lt(&1), Ok(false));
    /// // Python: `True < 2` (a bool is an int).
    /// assert_eq!(Value::Bool(true).try_lt(&Value::from(2)), Ok(true));
    /// ```
    fn try_lt(&self, other: &Self) -> std::result::Result<bool, Self::Incomparable>;
    /// `self <= other` (`Die.clip`).
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!(1i64.try_le(&1), Ok(true));
    /// assert_eq!(Value::Float(1.5).try_le(&Value::from(1)), Ok(false));
    /// ```
    fn try_le(&self, other: &Self) -> std::result::Result<bool, Self::Incomparable>;
    /// `self > other` (`max()`). Not always `<` reflected: a Python class may define `__gt__` its own way.
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!("b".try_gt(&"a"), Ok(true));
    /// assert_eq!(Value::str("b").try_gt(&Value::str("a")), Ok(true));
    /// ```
    fn try_gt(&self, other: &Self) -> std::result::Result<bool, Self::Incomparable>;
    /// `self >= other`.
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!(2i64.try_ge(&3), Ok(false));
    /// assert!(Value::str("a").try_ge(&Value::from(1)).is_err()); // `'a' >= 1` raises
    /// ```
    fn try_ge(&self, other: &Self) -> std::result::Result<bool, Self::Incomparable>;
    /// The truth of `self == other` as a comparison of tuples asks it. Not dict-key equality (`Eq`), which also
    /// requires equal hashes: the two differ for a few Python types (`Symbols('a') == 'a'`, but their hashes
    /// differ).
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!(3i64.try_eq(&3), Ok(true));
    /// // Python: `1 == 1.0`, and `==` between unrelated types is `False`, never a raise.
    /// assert_eq!(Value::from(1).try_eq(&Value::Float(1.0)), Ok(true));
    /// assert_eq!(Value::from(1).try_eq(&Value::str("a")), Ok(false));
    /// ```
    fn try_eq(&self, other: &Self) -> std::result::Result<bool, Self::Incomparable>;
    /// Whether this may be a key at all: `Counts` rejects some keys outright (`None`, `Reroll`) with `TypeError`.
    /// `Ord` types accept everything.
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// assert_eq!(7i64.check_key(), Ok(()));
    /// assert!(Value::from(7).check_key().is_ok());
    /// // Python: `Die([None])` raises TypeError.
    /// assert!(Value::None.check_key().is_err());
    /// ```
    fn check_key(&self) -> std::result::Result<(), Self::Incomparable>;

    /// Python's `sorted(items)` with `lt` as `<`: a stable sort, stopping at the first comparison that refuses.
    /// `Ord` types use `slice::sort_by`; `Value` uses CPython's timsort (`value/sort.rs`), so that an
    /// inconsistent `<` (NaN, a host class) sorts, and raises, exactly as Python does.
    ///
    /// ```
    /// use icepool_core::{OutcomeOrder, Value};
    /// let sorted = i64::try_sort_by(vec![(3, 'a'), (1, 'b'), (3, 'c')], |x, y| x.0.try_lt(&y.0));
    /// assert_eq!(sorted, Ok(vec![(1, 'b'), (3, 'a'), (3, 'c')])); // stable
    /// // Python: `sorted([2, 'a'])` raises TypeError.
    /// let mixed = vec![Value::from(2), Value::str("a")];
    /// assert!(Value::try_sort_by(mixed, |x, y| x.try_lt(y)).is_err());
    /// ```
    fn try_sort_by<T>(
        items: Vec<T>,
        lt: impl Fn(&T, &T) -> std::result::Result<bool, Self::Incomparable>,
    ) -> std::result::Result<Vec<T>, Self::Incomparable>;
}

/// An outcome type with a total order: any `Ord` type. This is the bound the typed API uses; its comparisons
/// never fail, so methods on `Die<O: Outcome>` return plain values.
///
/// ```
/// use icepool_core::{d, Die, Outcome};
/// // The typed API's bound: any `Ord` type, so its comparisons are plain `Ord` ones.
/// fn top<O: Outcome>(die: &Die<O>) -> Option<&O> {
///     die.outcomes().max()
/// }
/// assert_eq!(top(&d(6)), Some(&6));
/// assert_eq!(top(&Die::from_iter(["a", "b"])), Some(&"b"));
/// ```
pub trait Outcome: OutcomeOrder<Incomparable = Infallible> + Ord {}

impl<T: OutcomeOrder<Incomparable = Infallible> + Ord> Outcome for T {}

impl<T: Ord + Hash + Clone + Debug + Send + Sync + 'static> OutcomeOrder for T {
    type Incomparable = Infallible;

    fn try_lt(&self, other: &Self) -> std::result::Result<bool, Infallible> {
        Ok(self < other)
    }

    fn try_le(&self, other: &Self) -> std::result::Result<bool, Infallible> {
        Ok(self <= other)
    }

    fn try_gt(&self, other: &Self) -> std::result::Result<bool, Infallible> {
        Ok(self > other)
    }

    fn try_ge(&self, other: &Self) -> std::result::Result<bool, Infallible> {
        Ok(self >= other)
    }

    fn try_eq(&self, other: &Self) -> std::result::Result<bool, Infallible> {
        Ok(self == other)
    }

    fn check_key(&self) -> std::result::Result<(), Infallible> {
        Ok(())
    }

    fn try_sort_by<U>(
        mut items: Vec<U>,
        lt: impl Fn(&U, &U) -> std::result::Result<bool, Infallible>,
    ) -> std::result::Result<Vec<U>, Infallible> {
        // A stable sort gives the same result as Python's for any consistent `<`.
        items.sort_by(|a, b| {
            if infallible(lt(a, b)) {
                std::cmp::Ordering::Less
            } else if infallible(lt(b, a)) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        Ok(items)
    }
}

/// The value of a result that cannot fail: how the typed API unwraps comparisons of an [`Outcome`].
pub fn infallible<T>(result: std::result::Result<T, Infallible>) -> T {
    match result {
        Ok(t) => t,
        Err(never) => match never {},
    }
}

/// Turns a comparison's refusal into the core's [`Error`], for `?` in generic code returning [`Result`] (it also
/// fails for reasons that are not comparisons). `?` cannot do it by itself: a `From` bound on an associated type
/// is not carried along to users of the trait.
pub(crate) trait ErrInto<T> {
    fn err_into(self) -> Result<T>;
}

impl<T, E: Into<Error>> ErrInto<T> for std::result::Result<T, E> {
    fn err_into(self) -> Result<T> {
        self.map_err(Into::into)
    }
}

// The comparisons generic code makes, as the core's `Result`.

/// `a < b`.
pub(crate) fn lt<O: OutcomeOrder>(a: &O, b: &O) -> Result<bool> {
    a.try_lt(b).err_into()
}

/// `a > b`.
pub(crate) fn gt<O: OutcomeOrder>(a: &O, b: &O) -> Result<bool> {
    a.try_gt(b).err_into()
}

/// `sorted(items)`, comparing with `lt` (see [`OutcomeOrder::try_sort_by`]).
pub(crate) fn sort_by<O: OutcomeOrder, T>(
    items: Vec<T>,
    lt: impl Fn(&T, &T) -> std::result::Result<bool, O::Incomparable>,
) -> Result<Vec<T>> {
    O::try_sort_by(items, lt).err_into()
}

/// `sorted(outcomes)`.
pub(crate) fn sorted<O: OutcomeOrder>(outcomes: Vec<O>) -> Result<Vec<O>> {
    sort_by::<O, _>(outcomes, O::try_lt)
}

/// Python's `min(xs)`: the first of the least elements, comparing `item < current`. `xs` is not empty.
pub(crate) fn min<'a, O: OutcomeOrder>(xs: impl IntoIterator<Item = &'a O>) -> Result<&'a O> {
    let mut xs = xs.into_iter();
    let mut best = xs.next().expect("min() of at least one outcome");
    for x in xs {
        if lt(x, best)? {
            best = x;
        }
    }
    Ok(best)
}

/// Python's `max(xs)`: the first of the greatest elements, comparing `item > current`. `xs` is not empty.
pub(crate) fn max<'a, O: OutcomeOrder>(xs: impl IntoIterator<Item = &'a O>) -> Result<&'a O> {
    let mut xs = xs.into_iter();
    let mut best = xs.next().expect("max() of at least one outcome");
    for x in xs {
        if gt(x, best)? {
            best = x;
        }
    }
    Ok(best)
}

/// The arithmetic the generic algorithms perform on outcomes (`n @ die`, sums, explode, `Again`, the sum
/// evaluator). Implemented with checked operations for the primitive integers (`Error::Overflow` where Rust would
/// overflow) and with Python's operators for `Value`. Tuples and arrays get none: Python's tuple `+` concatenates,
/// so no Rust meaning is the obvious one.
///
/// ```
/// use icepool_core::{Error, OutcomeArith};
/// assert_eq!(2i64.try_add(&3), Ok(5));
/// // Where Rust's own operator would overflow, an error.
/// assert_eq!(i8::MAX.try_add(&1), Err(Error::Overflow("attempt to add with overflow".into())));
/// ```
pub trait OutcomeArith: OutcomeOrder {
    /// `self + other`.
    ///
    /// ```
    /// use icepool_core::{OutcomeArith, Value};
    /// assert_eq!(2i64.try_add(&3), Ok(5));
    /// // Python: `'a' + 'b' == 'ab'`.
    /// assert_eq!(Value::str("a").try_add(&Value::str("b")).unwrap(), Value::str("ab"));
    /// ```
    fn try_add(&self, other: &Self) -> Result<Self>;
    /// `self - other`.
    ///
    /// ```
    /// use icepool_core::OutcomeArith;
    /// assert_eq!(2i64.try_sub(&3), Ok(-1));
    /// assert!(i64::MIN.try_sub(&1).is_err());
    /// ```
    fn try_sub(&self, other: &Self) -> Result<Self>;
    /// `-self`.
    ///
    /// ```
    /// use icepool_core::OutcomeArith;
    /// assert_eq!(4i64.try_neg(), Ok(-4));
    /// assert!(i32::MIN.try_neg().is_err());
    /// ```
    fn try_neg(&self) -> Result<Self>;
    /// `self * other`.
    ///
    /// ```
    /// use icepool_core::OutcomeArith;
    /// assert_eq!(4i64.try_mul(&-3), Ok(-12));
    /// assert!(i16::MAX.try_mul(&2).is_err());
    /// ```
    fn try_mul(&self, other: &Self) -> Result<Self>;
    /// `self * count`, an outcome times an integer count.
    ///
    /// ```
    /// use icepool_core::{Int, OutcomeArith, Value};
    /// assert_eq!(5i64.try_mul_count(&Int::from(3)), Ok(15));
    /// // A count past `i64` fits only times zero.
    /// let huge = Int::from(i64::MAX) * Int::from(4);
    /// assert_eq!(0i64.try_mul_count(&huge), Ok(0));
    /// assert!(1i64.try_mul_count(&huge).is_err());
    /// // Python: `'ab' * 2 == 'abab'`.
    /// assert_eq!(Value::str("ab").try_mul_count(&Int::from(2)).unwrap(), Value::str("abab"));
    /// ```
    fn try_mul_count(&self, count: &Int) -> Result<Self>;
    /// `self * 0` (`Population.zero`, which zeros an outcome by multiplying it by `0`).
    ///
    /// ```
    /// use icepool_core::{OutcomeArith, Value};
    /// assert_eq!(7i64.try_zero(), Ok(0));
    /// // Python: `'a' * 0 == ''`, `2.5 * 0 == 0.0`.
    /// assert_eq!(Value::str("a").try_zero().unwrap(), Value::str(""));
    /// assert_eq!(Value::Float(2.5).try_zero().unwrap().to_string(), "0.0");
    /// ```
    fn try_zero(&self) -> Result<Self>;
    /// `self // other`: Python's floor division, which rounds toward negative infinity (Rust's `/` truncates).
    /// `Error::DivisionByZero` for a zero divisor.
    ///
    /// ```
    /// use icepool_core::{Error, OutcomeArith};
    /// // Python: `-7 // 2 == -4`, `7 // -2 == -4`.
    /// assert_eq!((-7i64).try_floor_div(&2), Ok(-4));
    /// assert_eq!(7i64.try_floor_div(&-2), Ok(-4));
    /// assert!(matches!(1i64.try_floor_div(&0), Err(Error::DivisionByZero(_))));
    /// ```
    fn try_floor_div(&self, other: &Self) -> Result<Self>;
    /// `self ** other`. Python's `int ** negative int` is a `float`, which an integer outcome cannot hold: for the
    /// primitive integers a negative exponent is `Error::InvalidArgument`.
    ///
    /// ```
    /// use icepool_core::{OutcomeArith, Value};
    /// assert_eq!(2i64.try_pow(&10), Ok(1024));
    /// // Python: `2 ** -1 == 0.5`, a float: an `i64` outcome cannot hold it, a `Value` can.
    /// assert!(2i64.try_pow(&-1).is_err());
    /// assert_eq!(Value::from(2).try_pow(&Value::from(-1)).unwrap().to_string(), "0.5");
    /// ```
    fn try_pow(&self, other: &Self) -> Result<Self>;
}

/// "attempt to add with overflow", as Rust's own operators say in a debug build.
fn overflow<T>(what: &str) -> Result<T> {
    Err(Error::Overflow(format!("attempt to {what} with overflow")))
}

macro_rules! integer_arith {
    ($($t:ty),*) => {$(
        impl OutcomeArith for $t {
            fn try_add(&self, other: &Self) -> Result<Self> {
                self.checked_add(*other).map_or_else(|| overflow("add"), Ok)
            }

            fn try_sub(&self, other: &Self) -> Result<Self> {
                self.checked_sub(*other).map_or_else(|| overflow("subtract"), Ok)
            }

            fn try_neg(&self) -> Result<Self> {
                self.checked_neg().map_or_else(|| overflow("negate"), Ok)
            }

            fn try_mul(&self, other: &Self) -> Result<Self> {
                self.checked_mul(*other).map_or_else(|| overflow("multiply"), Ok)
            }

            fn try_mul_count(&self, count: &Int) -> Result<Self> {
                match <$t>::try_from(count.to_ibig()) {
                    Ok(count) => self.try_mul(&count),
                    // A count beyond the type only fits times zero.
                    Err(_) if *self == 0 => Ok(0),
                    Err(_) => overflow("multiply"),
                }
            }

            fn try_zero(&self) -> Result<Self> {
                Ok(0)
            }

            fn try_floor_div(&self, other: &Self) -> Result<Self> {
                if *other == 0 {
                    // CPython's message for `int // 0`.
                    return Err(Error::DivisionByZero("integer division or modulo by zero".into()));
                }
                // Only `MIN / -1` overflows.
                let Some(quotient) = self.checked_div(*other) else {
                    return overflow("divide");
                };
                // Truncation rounded toward zero; a remainder of the other sign than the divisor rounds down.
                let rounded_up = self % other != 0 && ((*self < 0) != (*other < 0));
                Ok(if rounded_up { quotient - 1 } else { quotient })
            }

            fn try_pow(&self, other: &Self) -> Result<Self> {
                if *other < 0 {
                    return Err(Error::InvalidArgument(
                        "negative exponent: Python's ** gives a float, which an integer outcome cannot hold".into(),
                    ));
                }
                match u32::try_from(*other) {
                    Ok(exponent) => self.checked_pow(exponent).map_or_else(|| overflow("multiply"), Ok),
                    // An exponent beyond `u32` only fits for a base of 0, 1 or -1.
                    Err(_) => match *self {
                        0 | 1 => Ok(*self),
                        -1 => Ok(if *other % 2 == 0 { 1 } else { -1 }),
                        _ => overflow("multiply"),
                    },
                }
            }
        }
    )*};
}

integer_arith!(i8, i16, i32, i64, i128, isize);

/// An outcome with an exact rational value: what the typed statistics (`Die::mean`, `Die::variance`) compute
/// with. Python's `int` dice (and `bool`, an `int` subclass) give `Fraction`s for these, which is what [`Ratio`]
/// is.
///
/// ```
/// use icepool_core::{Ratio, ToRatio};
/// assert_eq!(3i64.to_ratio(), Ratio::from(3));
/// assert_eq!(true.to_ratio(), Ratio::from(1));
/// ```
pub trait ToRatio {
    /// The outcome as an exact fraction.
    ///
    /// ```
    /// use icepool_core::{Ratio, ToRatio};
    /// assert_eq!((-4i64).to_ratio(), Ratio::from(-4));
    /// ```
    fn to_ratio(&self) -> Ratio;
}

macro_rules! integer_to_ratio {
    ($($t:ty),*) => {$(
        impl ToRatio for $t {
            fn to_ratio(&self) -> Ratio {
                Ratio::from(*self)
            }
        }
    )*};
}

integer_to_ratio!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

impl ToRatio for bool {
    fn to_ratio(&self) -> Ratio {
        Ratio::from(u8::from(*self))
    }
}

impl ToRatio for Ratio {
    fn to_ratio(&self) -> Ratio {
        self.clone()
    }
}

/// The counts and sizes the evaluators produce (a die of `Int` from `LargestCountEvaluator`) are Python `int`s.
///
/// ```
/// use icepool_core::{Int, Ratio, ToRatio};
/// assert_eq!(Int::from(-4).to_ratio(), Ratio::from(-4));
/// ```
impl ToRatio for Int {
    fn to_ratio(&self) -> Ratio {
        Ratio::from(self.to_ibig())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ord_types_compare_and_sort_without_failing() {
        assert!(infallible(1i64.try_lt(&2)));
        assert!(!infallible([1i64, 2].try_gt(&[1, 3])));
        let sorted = infallible(i64::try_sort_by(
            vec![(3, 'a'), (1, 'b'), (3, 'c')],
            |a, b| Ok(a.0 < b.0),
        ));
        assert_eq!(sorted, [(1, 'b'), (3, 'a'), (3, 'c')]);
    }

    #[test]
    fn integer_arithmetic_is_checked() {
        assert_eq!(2i64.try_add(&3), Ok(5));
        assert!(i64::MAX.try_add(&1).is_err());
        assert!(i64::MIN.try_neg().is_err());
        assert_eq!(3i8.try_mul_count(&Int::from(-2)), Ok(-6));
        assert!(3i8.try_mul_count(&Int::from(1000)).is_err());
        assert_eq!(0i8.try_mul_count(&Int::from(1000)), Ok(0));
        assert_eq!(7i32.try_zero(), Ok(0));
    }

    #[test]
    fn floor_div_and_pow_are_pythons() {
        // Python: -7 // 2 == -4, 7 // -2 == -4, -7 // -2 == 3, 6 // -3 == -2.
        for (a, b, q) in [(7i64, 2, 3), (-7, 2, -4), (7, -2, -4), (-7, -2, 3), (6, -3, -2), (0, -3, 0)] {
            assert_eq!(a.try_floor_div(&b), Ok(q), "{a} // {b}");
        }
        assert!(matches!(1i64.try_floor_div(&0), Err(Error::DivisionByZero(_))));
        assert!(i64::MIN.try_floor_div(&-1).is_err());
        assert_eq!(2i64.try_pow(&10), Ok(1024));
        assert_eq!((-3i8).try_pow(&3), Ok(-27));
        assert_eq!(0i64.try_pow(&0), Ok(1));
        assert!(2i8.try_pow(&7).is_err());
        assert!(matches!(2i64.try_pow(&-1), Err(Error::InvalidArgument(_))));
        // Exponents beyond u32.
        assert_eq!((-1i64).try_pow(&(1 << 40)), Ok(1));
        assert_eq!((-1i64).try_pow(&((1 << 40) + 1)), Ok(-1));
        assert_eq!(1i64.try_pow(&(1 << 40)), Ok(1));
        assert_eq!(0i64.try_pow(&(1 << 40)), Ok(0));
        assert!(2i64.try_pow(&(1 << 40)).is_err());
    }
}
