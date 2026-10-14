//! Port of icepool `operator/binary.py`: operators merging the counts of any number of expressions.

use super::{add, sub, Statelet};
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

    /// Only the additive union knows its size: the sum of its children's, if they all have one.
    pub(super) fn initial_state(
        self,
        child_sizes: &[Option<i64>],
    ) -> Result<(Statelet, Option<Count>)> {
        if self != BinaryOperator::AdditiveUnion || child_sizes.iter().any(Option::is_none) {
            return Ok((Statelet::None, None));
        }
        let size = child_sizes
            .iter()
            .flatten()
            .try_fold(0, |a, &b| add(a, b))?;
        Ok((Statelet::None, Some(Count::Int(size))))
    }
}
