//! Port of icepool `operator/binary.py`: operators merging the counts of any number of expressions.

use super::{add, int, sub, tuple_error, Statelet};
use crate::error::{Error, Result};
use crate::pool::Count;

/// The `MultisetBinaryOperator` subclasses. Given more than two children, the counts are `reduce`d.
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
    /// `merge_counts`: merge counts produced by the left and right expression.
    fn merge_counts(self, left: i64, right: i64) -> Result<i64> {
        Ok(match self {
            BinaryOperator::Intersection => left.min(right),
            BinaryOperator::DifferenceDropNegative => sub(left, right)?.max(0),
            BinaryOperator::DifferenceKeepNegative => sub(left, right)?,
            BinaryOperator::Union => left.max(right),
            BinaryOperator::AdditiveUnion => add(left, right)?,
            BinaryOperator::SymmetricDifference => sub(left, right)?
                .checked_abs()
                .ok_or_else(super::overflow)?,
        })
    }

    /// `reduce(self.merge_counts, child_counts)`.
    pub(super) fn next_state(self, child_counts: &[i64]) -> Result<i64> {
        let Some((&first, rest)) = child_counts.split_first() else {
            return Err(Error::Type(
                "reduce() of empty iterable with no initial value".into(),
            ));
        };
        rest.iter().try_fold(first, |a, &b| self.merge_counts(a, b))
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
                Count::Tuple(l.iter().chain(r.iter()).copied().collect())
            }
            _ => return Err(tuple_error()),
        })
    }

    /// `reduce(self.merge_counts, child_counts)` over counts that may be tuples (a single count is returned
    /// as it is).
    pub(super) fn reduce(self, child_counts: &[Count]) -> Result<Count> {
        let Some((first, rest)) = child_counts.split_first() else {
            return Err(Error::Type(
                "reduce() of empty iterable with no initial value".into(),
            ));
        };
        rest.iter().try_fold(first.clone(), |a, b| match (&a, b) {
            (Count::Int(a), Count::Int(b)) => Ok(Count::Int(self.merge_counts(*a, *b)?)),
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
            .try_fold(0, |a, b| add(a, int(b)?))?;
        Ok((Statelet::None, Some(Count::Int(size))))
    }
}
