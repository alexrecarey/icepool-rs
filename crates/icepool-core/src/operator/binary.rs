//! Port of icepool `operator/binary.py`: operators merging the counts of any number of expressions.

use super::{int, tuple_error, Statelet};
use crate::error::{Error, Result};
use crate::pool::Count;
use crate::Int;

/// The `MultisetBinaryOperator` subclasses. Given more than two children, the counts are `reduce`d.
///
/// ```
/// use icepool_core::expression::BinaryOperator;
///
/// assert_eq!(BinaryOperator::AdditiveUnion.symbol(), "+");
/// // Both differences print as `-`.
/// assert_eq!(BinaryOperator::DifferenceKeepNegative.symbol(), "-");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    /// `MultisetIntersection`, `&`: `min(left, right)`.
    Intersection,
    /// `MultisetDifferenceDropNegative`, `-`: `max(left - right, 0)`.
    DifferenceDropNegative,
    /// `MultisetDifferenceKeepNegative`: `left - right`.
    DifferenceKeepNegative,
    /// `MultisetUnion`, `|`: `max(left, right)`.
    Union,
    /// `MultisetAdditiveUnion`, `+`: `left + right`.
    AdditiveUnion,
    /// `MultisetSymmetricDifference`, `^`: `abs(left - right)`.
    SymmetricDifference,
}

impl BinaryOperator {
    /// `symbol()`: how `__str__` joins the children.
    ///
    /// ```
    /// use icepool_core::expression::BinaryOperator;
    /// assert_eq!(BinaryOperator::Intersection.symbol(), "&");
    /// assert_eq!(BinaryOperator::SymmetricDifference.symbol(), "^");
    /// ```
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOperator::Intersection => "&",
            BinaryOperator::DifferenceDropNegative | BinaryOperator::DifferenceKeepNegative => "-",
            BinaryOperator::Union => "|",
            BinaryOperator::AdditiveUnion => "+",
            BinaryOperator::SymmetricDifference => "^",
        }
    }

    /// `merge_counts`: merge counts produced by the left and right expression.
    fn merge_counts(self, left: &Int, right: &Int) -> Int {
        match self {
            BinaryOperator::Intersection => left.min(right).clone(),
            BinaryOperator::DifferenceDropNegative => (left - right).max(Int::ZERO),
            BinaryOperator::DifferenceKeepNegative => left - right,
            BinaryOperator::Union => left.max(right).clone(),
            BinaryOperator::AdditiveUnion => left + right,
            BinaryOperator::SymmetricDifference => (left - right).abs(),
        }
    }

    /// `reduce(self.merge_counts, child_counts)`.
    pub(super) fn next_state(self, child_counts: &[Int]) -> Result<Int> {
        let Some((first, rest)) = child_counts.split_first() else {
            return Err(Error::WrongType(
                "reduce() of empty iterable with no initial value".into(),
            ));
        };
        Ok(rest
            .iter()
            .fold(first.clone(), |a, b| self.merge_counts(&a, b)))
    }

    /// `merge_counts` when a count is a tuple: Python's `min` / `max` compare two tuples lexicographically (as
    /// `[i64]` does) and `+` concatenates them; everything else, and any tuple with an `int`, is a TypeError.
    fn merge_tuple_counts(self, left: Count, right: &Count) -> Result<Count> {
        let (Count::Tuple(l), Count::Tuple(r)) = (&left, right) else {
            return Err(tuple_error());
        };
        Ok(match self {
            // `min(left, right)` keeps `left` unless `right < left`; `max` unless `right > left`.
            BinaryOperator::Intersection if r < l => right.clone(),
            BinaryOperator::Union if r > l => right.clone(),
            BinaryOperator::Intersection | BinaryOperator::Union => left,
            BinaryOperator::AdditiveUnion => {
                Count::Tuple(l.iter().chain(r.iter()).cloned().collect())
            }
            _ => return Err(tuple_error()),
        })
    }

    /// `reduce(self.merge_counts, child_counts)` over counts that may be tuples (a single count is returned
    /// as it is).
    pub(super) fn reduce(self, child_counts: &[Count]) -> Result<Count> {
        let Some((first, rest)) = child_counts.split_first() else {
            return Err(Error::WrongType(
                "reduce() of empty iterable with no initial value".into(),
            ));
        };
        rest.iter().try_fold(first.clone(), |a, b| match (&a, b) {
            (Count::Int(a), Count::Int(b)) => Ok(Count::Int(self.merge_counts(a, b))),
            _ => self.merge_tuple_counts(a, b),
        })
    }

    /// Only the additive union knows its size: `sum(child_sizes)`, if they all have one (a tuple size is the
    /// TypeError of `0 + (...)`).
    pub(super) fn initial_state(
        self,
        child_sizes: &[Option<Count>],
    ) -> Result<(Statelet, Option<Count>)> {
        if self != BinaryOperator::AdditiveUnion || child_sizes.iter().any(Option::is_none) {
            return Ok((Statelet::None, None));
        }
        let size = child_sizes
            .iter()
            .flatten()
            .try_fold(Int::ZERO, |a, b| Ok::<_, crate::Error>(a + int(b)?))?;
        Ok((Statelet::None, Some(Count::Int(size))))
    }
}
