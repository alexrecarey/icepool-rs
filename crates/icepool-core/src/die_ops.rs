//! The idiomatic Rust surface of a typed [`Die`]: `std::ops` operators, comparison methods, `n @ die`, pools,
//! iteration, conversions and `Display` (design/library-architecture.md section 4).
//!
//! Every operator is Python's `Die.binary_operator` / `_unary_operator` on typed outcomes: each pair of outcomes
//! is combined, the quantities multiplied, and equal results add up. A plain outcome on either side stands for the
//! die that always rolls it, as Python converts the other operand with `implicit_convert_to_die`.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitXor, Div, Not, Rem, Shl, Shr};

use crate::counts::Counts;
use crate::die::Die;
use crate::format::{format, Formatted};
use crate::outcome::{Outcome, OutcomeArith};
use crate::pool::Pool;
use crate::value::evaluators::IntoValue;
use crate::Quantity;

/// What may stand on the other side of a die's operator or comparison: a die (owned or borrowed) or a plain
/// outcome, which is the die that always rolls it.
///
/// ```
/// use icepool_core::{d, Die, IntoDie};
/// assert_eq!(3.into_die(), Die::single(3));
/// assert_eq!((&d(6)).into_die(), d(6));
/// ```
pub trait IntoDie<O> {
    /// The die this operand stands for.
    fn into_die(self) -> Die<O>;
}

impl<O: Outcome> IntoDie<O> for O {
    fn into_die(self) -> Die<O> {
        Die::single(self)
    }
}

impl<O> IntoDie<O> for Die<O> {
    fn into_die(self) -> Die<O> {
        self
    }
}

impl<O: Clone> IntoDie<O> for &Die<O> {
    fn into_die(self) -> Die<O> {
        // Cheap: a die shares its items.
        self.clone()
    }
}

/// The checked arithmetic of [`OutcomeArith`], panicking on overflow as Rust's integer operators do in a debug
/// build ("attempt to add with overflow"), in release builds too: a wrong probability is worse than a panic.
fn checked<O>(result: crate::Result<O>) -> O {
    result.unwrap_or_else(|e| panic!("{}", e.message()))
}

/// `impl Trait<R: IntoDie<O>>` for `Die<O>` and `&Die<O>`, combining outcomes pairwise with the given expression.
macro_rules! binary_operators {
    ($($Trait:ident :: $method:ident where O: [$($bound:tt)*], |$a:ident, $b:ident| $body:expr;)*) => {$(
        impl<O: Outcome + $($bound)*, R: IntoDie<O>> std::ops::$Trait<R> for &Die<O> {
            type Output = Die<O>;
            fn $method(self, rhs: R) -> Die<O> {
                self.binary(&rhs.into_die(), |$a: &O, $b: &O| $body)
            }
        }

        impl<O: Outcome + $($bound)*, R: IntoDie<O>> std::ops::$Trait<R> for Die<O> {
            type Output = Die<O>;
            fn $method(self, rhs: R) -> Die<O> {
                std::ops::$Trait::$method(&self, rhs)
            }
        }
    )*};
}

binary_operators! {
    Add::add where O: [OutcomeArith], |a, b| checked(a.try_add(b));
    Sub::sub where O: [OutcomeArith], |a, b| checked(a.try_sub(b));
    Mul::mul where O: [OutcomeArith], |a, b| checked(a.try_mul(b));
    // The rest are `O`'s own operators: Rust's `/` and `%` truncate (Python's `//` floors), and like them panic
    // on division by zero.
    Div::div where O: [Div<Output = O>], |a, b| a.clone() / b.clone();
    Rem::rem where O: [Rem<Output = O>], |a, b| a.clone() % b.clone();
    BitAnd::bitand where O: [BitAnd<Output = O>], |a, b| a.clone() & b.clone();
    BitOr::bitor where O: [BitOr<Output = O>], |a, b| a.clone() | b.clone();
    BitXor::bitxor where O: [BitXor<Output = O>], |a, b| a.clone() ^ b.clone();
    Shl::shl where O: [Shl<Output = O>], |a, b| a.clone() << b.clone();
    Shr::shr where O: [Shr<Output = O>], |a, b| a.clone() >> b.clone();
}

/// `-die`: every outcome negated (checked, as `-` of `i64::MIN` overflows).
///
/// ```
/// use icepool_core::{d, Die};
/// assert_eq!(-d(2), Die::from_iter([-1, -2]));
/// ```
impl<O: Outcome + OutcomeArith> std::ops::Neg for &Die<O> {
    type Output = Die<O>;
    fn neg(self) -> Die<O> {
        self.map(|a| checked(a.try_neg()))
    }
}

impl<O: Outcome + OutcomeArith> std::ops::Neg for Die<O> {
    type Output = Die<O>;
    fn neg(self) -> Die<O> {
        -&self
    }
}

/// `!die`: every outcome negated by `O`'s `!` (logical for `bool`, bitwise for integers).
///
/// ```
/// use icepool_core::{d, Die};
/// let even = d(6).map(|&x| x % 2 == 0);
/// assert_eq!(!even, Die::from_weights([(false, 3u32), (true, 3)]));
/// ```
impl<O: Outcome + Not<Output = O>> Not for &Die<O> {
    type Output = Die<O>;
    fn not(self) -> Die<O> {
        self.map(|a| !a.clone())
    }
}

impl<O: Outcome + Not<Output = O>> Not for Die<O> {
    type Output = Die<O>;
    fn not(self) -> Die<O> {
        !&self
    }
}

/// `1 + die`, `10 - die`, ... for the primitive integers: the plain outcome is the left operand (Python's
/// reflected operators keep the order). The orphan rule needs one impl per type.
macro_rules! outcome_on_the_left {
    ($($t:ty),*) => {$(
        outcome_on_the_left!(@ops $t: Add add, Sub sub, Mul mul, Div div, Rem rem, BitAnd bitand, BitOr bitor,
            BitXor bitxor, Shl shl, Shr shr);
    )*};
    (@ops $t:ty: $($Trait:ident $method:ident),*) => {$(
        impl std::ops::$Trait<Die<$t>> for $t {
            type Output = Die<$t>;
            fn $method(self, rhs: Die<$t>) -> Die<$t> {
                std::ops::$Trait::$method(Die::single(self), rhs)
            }
        }

        impl std::ops::$Trait<&Die<$t>> for $t {
            type Output = Die<$t>;
            fn $method(self, rhs: &Die<$t>) -> Die<$t> {
                std::ops::$Trait::$method(Die::single(self), rhs)
            }
        }
    )*};
}

outcome_on_the_left!(i8, i16, i32, i64, i128, isize);

/// Comparisons are methods returning dice of `bool` (Python's `die < other` etc.). `==` and `!=` stay
/// `PartialEq`'s, which is Python's `die.equals(other)`; the dice-valued forms are [`Die::equal`] and
/// [`Die::not_equal`].
impl<O: Outcome> Die<O> {
    /// The die of `self < other`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// assert_eq!(d(6).lt(3), Die::from_weights([(false, 4u32), (true, 2)]));
    /// ```
    pub fn lt(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a < b)
    }

    /// The die of `self <= other`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// assert_eq!(d(6).le(3), Die::from_weights([(false, 3u32), (true, 3)]));
    /// ```
    pub fn le(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a <= b)
    }

    /// The die of `self > other`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// // Ties go to the defender: 15 of the 36 pairs are a win.
    /// assert_eq!(d(6).gt(d(6)), Die::from_weights([(false, 21u32), (true, 15)]));
    /// ```
    pub fn gt(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a > b)
    }

    /// The die of `self >= other`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let hits: Die<bool> = d(20).ge(11);
    /// assert_eq!(hits, Die::from_weights([(false, 10u32), (true, 10)]));
    /// ```
    pub fn ge(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a >= b)
    }

    /// The die of `self == other` (Python's `die == other` as a die; Rust's `==` compares the dice themselves).
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// assert_eq!(d(6).equal(d(6)), Die::from_weights([(false, 30u32), (true, 6)]));
    /// ```
    pub fn equal(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a == b)
    }

    /// The die of `self != other`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// assert_eq!(d(4).not_equal(1), Die::from_weights([(false, 1u32), (true, 3)]));
    /// ```
    pub fn not_equal(&self, other: impl IntoDie<O>) -> Die<bool> {
        self.binary(&other.into_die(), |a, b| a != b)
    }

    /// A pool of `rolls` of this die (Python's `die.pool(rolls)`), to keep some of them and evaluate.
    ///
    /// # Panics
    ///
    /// If the pool would hold more dice than the port supports ([`Pool::new`] is the fallible form).
    ///
    /// ```
    /// use icepool_core::d;
    /// let pool = d(6).pool(4);
    /// assert_eq!(pool.raw_size(), 4);
    /// ```
    pub fn pool(&self, rolls: usize) -> Pool<O> {
        Pool::new(vec![(self.clone(), rolls)]).unwrap_or_else(|e| panic!("{}", e.message()))
    }
}

impl<O: Outcome + OutcomeArith> Die<O> {
    /// The sum of `rolls` rolls of this die (Python's `rolls @ die`); a negative count rolls `-rolls` times and
    /// negates the sum.
    ///
    /// # Panics
    ///
    /// On overflow, as the operators do, or if the result could not be held in memory ([`Die::matmul`] is the
    /// fallible form).
    ///
    /// ```
    /// use icepool_core::d;
    /// let three_d6 = d(6).sum_rolls(3);
    /// assert_eq!(three_d6, &d(6) + &d(6) + &d(6));
    /// assert_eq!(three_d6.denominator().to_string(), "216");
    /// ```
    pub fn sum_rolls(&self, rolls: i64) -> Die<O> {
        checked(self.matmul(&[(rolls, Quantity::ONE)]))
    }
}

/// One of each outcome: equal outcomes add up (Python's `Die([...])` of plain outcomes).
///
/// ```
/// use icepool_core::Die;
/// let die = Die::from_iter([1, 2, 2, 3]);
/// assert_eq!(die, Die::from_weights([(1, 1u32), (2, 2), (3, 1)]));
/// ```
impl<O: Outcome> FromIterator<O> for Die<O> {
    fn from_iter<I: IntoIterator<Item = O>>(outcomes: I) -> Self {
        Die::from_weights(outcomes.into_iter().map(|o| (o, Quantity::ONE)))
    }
}

/// A die rolling each value of the range once (Python's `Die(range(a, b))`).
///
/// ```
/// use icepool_core::{d, Die};
/// assert_eq!(Die::from(1..7), d(6));
/// ```
impl<O: Outcome> From<std::ops::Range<O>> for Die<O>
where
    std::ops::Range<O>: Iterator<Item = O>,
{
    fn from(range: std::ops::Range<O>) -> Self {
        range.collect()
    }
}

/// A die rolling each value of the range once.
///
/// ```
/// use icepool_core::{d, Die};
/// assert_eq!(Die::from(1..=6), d(6));
/// ```
impl<O: Outcome> From<std::ops::RangeInclusive<O>> for Die<O>
where
    std::ops::RangeInclusive<O>: Iterator<Item = O>,
{
    fn from(range: std::ops::RangeInclusive<O>) -> Self {
        range.collect()
    }
}

/// Iterates over `(outcome, quantity)` in ascending order of outcome, as [`Die::items`].
///
/// ```
/// use icepool_core::d;
/// for (outcome, quantity) in &d(6) {
///     assert!((1..=6).contains(outcome));
///     assert_eq!(quantity.to_string(), "1");
/// }
/// ```
impl<'a, O> IntoIterator for &'a Die<O>
where
    O: crate::outcome::OutcomeOrder,
{
    type Item = &'a (O, Quantity);
    type IntoIter = std::slice::Iter<'a, (O, Quantity)>;

    fn into_iter(self) -> Self::IntoIter {
        self.items().iter()
    }
}

/// Python's `str(die)`: a markdown table of outcome, quantity and probability, as icepool prints the same die of
/// Python values (the outcomes are converted with [`IntoValue`], so the text is Python's exactly).
///
/// ```
/// use icepool_core::d;
/// let text = d(4).to_string();
/// assert!(text.starts_with("Die with denominator 4\n"));
/// assert!(text.contains("|       1 |        1 |  25.000000% |"));
/// ```
impl<O: Outcome + IntoValue> fmt::Display for Die<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Converting keeps the order: the integers, `bool` and their tuples sort alike as Python values.
        let items = self.items().iter().map(|(o, q)| (o.clone().into_value(), q.clone()));
        let die = Die::from_counts(Counts::from_sorted(items.collect()));
        match format(&die, "Die", "", false) {
            Ok(Formatted::Text(text)) => f.write_str(&text),
            // Python's own str() of ints, bools and tuples of them cannot fail.
            _ => Err(fmt::Error),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{d, Die};

    #[test]
    fn operators_take_dice_and_outcomes_on_either_side() {
        let two = &d(6) + &d(6);
        assert_eq!(two.denominator().to_string(), "36");
        assert_eq!(two.outcomes().copied().collect::<Vec<_>>(), (2..=12).collect::<Vec<_>>());
        assert_eq!(d(6) - 1, Die::from(0..=5));
        assert_eq!(7 - d(6), Die::from(1..=6));
        assert_eq!(2 * &d(3), Die::from_iter([2, 4, 6]));
        assert_eq!(d(6) % 2, Die::from_weights([(0, 3u32), (1, 3)]));
        assert_eq!(d(4) / 2, Die::from_iter([0, 1, 1, 2]));
        assert_eq!(1 << d(3), Die::from_iter([2, 4, 8]));
        let t = Die::from_iter([true, false]);
        assert_eq!(&t & &t, Die::from_weights([(false, 3u32), (true, 1)]));
        assert_eq!(&t | false, t);
    }

    #[test]
    #[should_panic(expected = "attempt to add with overflow")]
    fn overflow_panics_in_every_build() {
        let _ = d(6) + i64::MAX;
    }

    #[test]
    #[should_panic(expected = "sides must be at least 1.")]
    fn d0_panics_as_python_raises() {
        d(0);
    }

    #[test]
    fn display_is_pythons_str() {
        // `str(icepool.Die([True, False, False]))`, from the oracle.
        let text = Die::from_iter([true, false, false]).to_string();
        assert_eq!(
            text,
            "Die with denominator 3\n\n| Outcome | Quantity | Probability |\n|:--------|---------:|------------:|\n\
             | False   |        2 |  66.666667% |\n| True    |        1 |  33.333333% |\n\n"
        );
    }
}
