//! [`Int`]: an unbounded integer for multiset counts (keep tuples, the counts sources pop, what operators and
//! evaluators compute with).
//!
//! Python counts are `int`s, so `d6.pool([2**70, 1])` or `pool * 2**63` are ordinary. Almost every count is small,
//! though, and counts sit in the evaluator's memo keys and are added up once per pop: an `IBig` everywhere would
//! slow ordinary evaluations down. So an `Int` is an `i64` until checked arithmetic overflows, and only then an
//! `IBig` (behind an `Arc`, so a clone stays cheap). The representation is canonical (a value that fits `i64` is
//! always `Small`), which makes the derived `Eq` and `Hash` agree with the value.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};
use std::sync::Arc;

use dashu_int::IBig;

use crate::error::{Error, Result};

/// An unbounded integer count: an `i64` until arithmetic overflows it (see the module documentation).
///
/// ```
/// use icepool_core::Int;
/// let big = Int::from(i64::MAX) + Int::ONE;
/// assert_eq!(big.to_i64(), None);
/// assert_eq!(big.to_string(), "9223372036854775808");
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Int(Repr);

#[derive(Clone, PartialEq, Eq)]
enum Repr {
    Small(i64),
    /// Never a value that fits `i64`.
    Big(Arc<IBig>),
}

impl Int {
    /// Zero.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert!(Int::ZERO.is_zero());
    /// assert_eq!(Int::ZERO, Int::default());
    /// ```
    pub const ZERO: Int = Int(Repr::Small(0));
    /// One.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert_eq!(Int::ONE + Int::ONE, Int::from(2));
    /// ```
    pub const ONE: Int = Int(Repr::Small(1));

    /// `i` as an `Int`, in a `const` context.
    ///
    /// ```
    /// use icepool_core::Int;
    /// const THREE: Int = Int::small(3);
    /// assert_eq!(THREE, 3);
    /// ```
    pub const fn small(i: i64) -> Int {
        Int(Repr::Small(i))
    }

    /// The value as an `i64`, if it fits.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert_eq!(Int::from(-5).to_i64(), Some(-5));
    /// assert_eq!((Int::from(i64::MIN) - Int::ONE).to_i64(), None);
    /// ```
    pub fn to_i64(&self) -> Option<i64> {
        match self.0 {
            Repr::Small(i) => Some(i),
            Repr::Big(_) => None,
        }
    }

    /// The value as a `usize`, if it fits (so not if negative).
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert_eq!(Int::from(5).to_usize(), Some(5));
    /// assert_eq!(Int::from(-1).to_usize(), None);
    /// ```
    pub fn to_usize(&self) -> Option<usize> {
        self.to_i64().and_then(|i| usize::try_from(i).ok())
    }

    /// The value as a `usize`, `usize::MAX` if it is larger and 0 if negative: for `min` with a length, where
    /// anything past the length means the same.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert_eq!(Int::from(-3).clamp_usize(), 0);
    /// assert_eq!(Int::from(4).clamp_usize(), 4);
    /// assert_eq!((Int::from(i64::MAX) * Int::from(4)).clamp_usize(), usize::MAX);
    /// ```
    pub fn clamp_usize(&self) -> usize {
        if self.is_negative() {
            0
        } else {
            self.to_usize().unwrap_or(usize::MAX)
        }
    }

    /// The `n` of a sequence repeat `seq * n` as a length: `OverflowError` beyond 64 bits, whatever the sign
    /// (Python's "cannot fit 'int' into an index-sized integer", raised before looking at the sequence), and 0 for
    /// any other negative. A `u64` rather than a `usize`: Python's index-sized integer is 64 bits, also where the
    /// port's `usize` is not (wasm32), so callers bound the length before taking it as a `usize`.
    ///
    /// ```
    /// use icepool_core::{Error, Int};
    /// // Python: `[1] * -3 == []`; `[1] * 2**64` raises OverflowError.
    /// assert_eq!(Int::from(-3).to_len("repeat"), Ok(0));
    /// assert_eq!(Int::from(3).to_len("repeat"), Ok(3));
    /// let huge = Int::from(i64::MAX) * Int::from(2);
    /// assert!(matches!(huge.to_len("repeat"), Err(Error::Overflow(_))));
    /// ```
    pub fn to_len(&self, what: &str) -> Result<u64> {
        match self.0 {
            Repr::Small(i) => Ok(u64::try_from(i).unwrap_or(0)),
            Repr::Big(_) => Err(Error::Overflow(format!(
                "cannot fit 'int' into an index-sized integer ({what})"
            ))),
        }
    }

    /// The value as a big integer.
    ///
    /// ```
    /// use icepool_core::Int;
    /// let big = Int::from(i64::MAX) + Int::ONE;
    /// assert_eq!(big.to_ibig().to_string(), "9223372036854775808");
    /// assert_eq!(Int::from(big.to_ibig()), big); // and back
    /// ```
    pub fn to_ibig(&self) -> IBig {
        match &self.0 {
            Repr::Small(i) => IBig::from(*i),
            Repr::Big(b) => (**b).clone(),
        }
    }

    /// Whether this is zero.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert!(Int::ZERO.is_zero());
    /// assert!(!Int::ONE.is_zero());
    /// ```
    pub fn is_zero(&self) -> bool {
        matches!(self.0, Repr::Small(0))
    }

    /// Whether this is less than zero.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert!(Int::from(-1).is_negative());
    /// assert!(!Int::ZERO.is_negative());
    /// assert!((Int::from(i64::MIN) - Int::ONE).is_negative()); // big
    /// ```
    pub fn is_negative(&self) -> bool {
        match &self.0 {
            Repr::Small(i) => *i < 0,
            Repr::Big(b) => **b < IBig::ZERO,
        }
    }

    /// Whether this is greater than zero.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert!(Int::ONE.is_positive());
    /// assert!(!Int::ZERO.is_positive());
    /// assert!((Int::from(i64::MAX) + Int::ONE).is_positive()); // big
    /// ```
    pub fn is_positive(&self) -> bool {
        match &self.0 {
            Repr::Small(i) => *i > 0,
            Repr::Big(b) => **b > IBig::ZERO,
        }
    }

    /// The absolute value.
    ///
    /// ```
    /// use icepool_core::Int;
    /// assert_eq!(Int::from(-4).abs(), 4);
    /// // Python: `abs(-2**63) == 9223372036854775808`, past `i64`.
    /// assert_eq!(Int::from(i64::MIN).abs().to_string(), "9223372036854775808");
    /// ```
    pub fn abs(&self) -> Int {
        if self.is_negative() {
            -self
        } else {
            self.clone()
        }
    }

    /// Python's `divmod(self, other)`: the quotient rounds toward minus infinity, the remainder takes the
    /// divisor's sign. `ZeroDivisionError` for 0.
    ///
    /// ```
    /// use icepool_core::{Error, Int};
    /// // Python: `divmod(-7, 2) == (-4, 1)`, `divmod(7, -2) == (-4, -1)`.
    /// assert_eq!(Int::from(-7).floor_divmod(&Int::from(2)), Ok((Int::from(-4), Int::from(1))));
    /// assert_eq!(Int::from(7).floor_divmod(&Int::from(-2)), Ok((Int::from(-4), Int::from(-1))));
    /// assert!(matches!(Int::ONE.floor_divmod(&Int::ZERO), Err(Error::DivisionByZero(_))));
    /// ```
    pub fn floor_divmod(&self, other: &Int) -> Result<(Int, Int)> {
        if other.is_zero() {
            return Err(Error::DivisionByZero(
                "integer division or modulo by zero".into(),
            ));
        }
        if let (Repr::Small(a), Repr::Small(b)) = (&self.0, &other.0) {
            // `i64::MIN // -1` is the only quotient that overflows.
            if let (Some(q), Some(r)) = (a.checked_div(*b), a.checked_rem(*b)) {
                return Ok(if r != 0 && (r < 0) != (*b < 0) {
                    (Int::small(q - 1), Int::small(r + b))
                } else {
                    (Int::small(q), Int::small(r))
                });
            }
        }
        let (q, r) = crate::value::ops::floor_divmod(&self.to_ibig(), &other.to_ibig());
        Ok((q.into(), r.into()))
    }
}

/// A small value hashes as its bare `i64` (no enum tag): keep tuples and counts are part of the evaluator's memo
/// keys. A big value never equals a small one, so sharing hashes with them is harmless.
/// Zero.
impl Default for Int {
    fn default() -> Self {
        Int::ZERO
    }
}

impl Hash for Int {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            Repr::Small(i) => i.hash(state),
            Repr::Big(b) => b.hash(state),
        }
    }
}

impl From<i64> for Int {
    fn from(i: i64) -> Int {
        Int::small(i)
    }
}

impl From<i32> for Int {
    fn from(i: i32) -> Int {
        Int::small(i.into())
    }
}

impl From<usize> for Int {
    fn from(n: usize) -> Int {
        match i64::try_from(n) {
            Ok(i) => Int::small(i),
            Err(_) => IBig::from(n).into(),
        }
    }
}

impl From<IBig> for Int {
    fn from(b: IBig) -> Int {
        match i64::try_from(&b) {
            Ok(i) => Int::small(i),
            Err(_) => Int(Repr::Big(Arc::new(b))),
        }
    }
}

impl From<&IBig> for Int {
    fn from(b: &IBig) -> Int {
        match i64::try_from(b) {
            Ok(i) => Int::small(i),
            Err(_) => Int(Repr::Big(Arc::new(b.clone()))),
        }
    }
}

impl From<Int> for IBig {
    fn from(i: Int) -> IBig {
        i.to_ibig()
    }
}

impl From<&Int> for IBig {
    fn from(i: &Int) -> IBig {
        i.to_ibig()
    }
}

impl From<Int> for crate::Value {
    fn from(i: Int) -> crate::Value {
        crate::Value::Int(i.to_ibig())
    }
}

impl From<&Int> for crate::Value {
    fn from(i: &Int) -> crate::Value {
        crate::Value::Int(i.to_ibig())
    }
}

impl PartialEq<i64> for Int {
    fn eq(&self, other: &i64) -> bool {
        self.0 == Repr::Small(*other)
    }
}

impl PartialOrd<i64> for Int {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        Some(match &self.0 {
            Repr::Small(i) => i.cmp(other),
            // Beyond `i64` on one side or the other.
            Repr::Big(b) => {
                if **b < IBig::ZERO {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
        })
    }
}

impl Ord for Int {
    fn cmp(&self, other: &Int) -> Ordering {
        match (&self.0, &other.0) {
            (Repr::Small(a), Repr::Small(b)) => a.cmp(b),
            (Repr::Big(a), Repr::Big(b)) => a.cmp(b),
            (Repr::Small(_), Repr::Big(b)) => {
                if **b < IBig::ZERO {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Repr::Big(_), Repr::Small(_)) => other.cmp(self).reverse(),
        }
    }
}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Int) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Small(i) => write!(f, "{i}"),
            Repr::Big(b) => write!(f, "{b}"),
        }
    }
}

impl fmt::Debug for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The binary operators: checked `i64` arithmetic, else `IBig`. Every combination of owned and borrowed operands,
/// and `i64` on the right.
macro_rules! binary_op {
    ($trait:ident, $method:ident, $checked:ident, $assign_trait:ident, $assign:ident) => {
        impl $trait<&Int> for &Int {
            type Output = Int;
            fn $method(self, other: &Int) -> Int {
                if let (Repr::Small(a), Repr::Small(b)) = (&self.0, &other.0) {
                    if let Some(c) = a.$checked(*b) {
                        return Int::small(c);
                    }
                }
                self.to_ibig().$method(other.to_ibig()).into()
            }
        }
        impl $trait<Int> for Int {
            type Output = Int;
            fn $method(self, other: Int) -> Int {
                (&self).$method(&other)
            }
        }
        impl $trait<&Int> for Int {
            type Output = Int;
            fn $method(self, other: &Int) -> Int {
                (&self).$method(other)
            }
        }
        impl $trait<Int> for &Int {
            type Output = Int;
            fn $method(self, other: Int) -> Int {
                self.$method(&other)
            }
        }
        impl $trait<i64> for &Int {
            type Output = Int;
            fn $method(self, other: i64) -> Int {
                self.$method(&Int::small(other))
            }
        }
        impl $trait<i64> for Int {
            type Output = Int;
            fn $method(self, other: i64) -> Int {
                (&self).$method(&Int::small(other))
            }
        }
        impl $assign_trait<&Int> for Int {
            fn $assign(&mut self, other: &Int) {
                *self = (&*self).$method(other);
            }
        }
        impl $assign_trait<Int> for Int {
            fn $assign(&mut self, other: Int) {
                *self = (&*self).$method(&other);
            }
        }
        impl $assign_trait<i64> for Int {
            fn $assign(&mut self, other: i64) {
                *self = (&*self).$method(&Int::small(other));
            }
        }
    };
}

binary_op!(Add, add, checked_add, AddAssign, add_assign);
binary_op!(Sub, sub, checked_sub, SubAssign, sub_assign);
binary_op!(Mul, mul, checked_mul, MulAssign, mul_assign);

impl Neg for &Int {
    type Output = Int;
    fn neg(self) -> Int {
        match &self.0 {
            Repr::Small(i) => match i.checked_neg() {
                Some(n) => Int::small(n),
                None => (-IBig::from(*i)).into(),
            },
            Repr::Big(b) => (-(**b).clone()).into(),
        }
    }
}

impl Neg for Int {
    type Output = Int;
    fn neg(self) -> Int {
        -&self
    }
}

impl std::iter::Sum for Int {
    fn sum<I: Iterator<Item = Int>>(iter: I) -> Int {
        iter.fold(Int::ZERO, |a, b| a + b)
    }
}

impl<'a> std::iter::Sum<&'a Int> for Int {
    fn sum<I: Iterator<Item = &'a Int>>(iter: I) -> Int {
        iter.fold(Int::ZERO, |a, b| a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_goes_big_and_back() {
        let max = Int::from(i64::MAX);
        let big = &max + 1;
        assert_eq!(big.to_i64(), None);
        assert_eq!(big.to_string(), "9223372036854775808");
        // Back within `i64`, the representation is small again, so equality and hashing agree.
        assert_eq!(&big - 1, max);
        assert_eq!((&big - 1).to_i64(), Some(i64::MAX));
        assert_eq!(-Int::from(i64::MIN), big);
        // -2**63 is `i64::MIN`, small again; one less is not.
        assert_eq!((-&big).to_i64(), Some(i64::MIN));
        let below = -&big - 1;
        assert!(big > max && below < i64::MIN && big > 5 && below < -5);
        assert_eq!(&big * &big, Int::from(IBig::from(2).pow(126)));
    }

    #[test]
    fn floor_divmod_is_pythons() {
        let dm = |a: i64, b: i64| {
            let (q, r) = Int::from(a).floor_divmod(&Int::from(b)).unwrap();
            (q.to_i64().unwrap(), r.to_i64().unwrap())
        };
        assert_eq!(dm(7, 2), (3, 1));
        assert_eq!(dm(-7, 2), (-4, 1));
        assert_eq!(dm(7, -2), (-4, -1));
        // `i64::MIN // -1` overflows `i64`.
        let (q, r) = Int::from(i64::MIN).floor_divmod(&Int::from(-1)).unwrap();
        assert_eq!((q, r), (-Int::from(i64::MIN), Int::ZERO));
        assert!(Int::ONE.floor_divmod(&Int::ZERO).is_err());
    }

    #[test]
    fn small_values_hash_and_debug_as_their_i64() {
        use std::collections::hash_map::DefaultHasher;
        fn hash<T: Hash>(x: &T) -> u64 {
            let mut h = DefaultHasher::new();
            x.hash(&mut h);
            h.finish()
        }
        // No enum tag in the hash (the doc comment on `impl Hash`), and unequal values hash apart.
        assert_eq!(hash(&Int::small(5)), hash(&5i64));
        assert_ne!(hash(&Int::small(5)), hash(&Int::small(6)));
        let big = Int::from(i64::MAX) + 1;
        assert_eq!(hash(&big), hash(&big.to_ibig()));
        assert_eq!(format!("{:?}", Int::small(-3)), "-3");
        assert_eq!(format!("{big:?}"), "9223372036854775808");
    }
}
