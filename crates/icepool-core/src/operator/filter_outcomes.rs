//! Port of icepool `operator/filter_outcomes.py`: keeps the designated outcomes, dropping the rest, or vice
//! versa. This is similar to `intersection` or `difference`, except the target set is considered to have
//! unlimited multiplicity.

use std::sync::Arc;

use crate::error::Result;
use crate::Int;

/// A callable returning `True` iff the outcome should be kept (Python calls `bool()` on its result).
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::expression::{OutcomeFilter, OutcomePredicate};
/// use icepool_core::Result;
///
/// struct Even;
/// impl OutcomePredicate<i64> for Even {
///     fn test(&self, outcome: &i64) -> Result<bool> {
///         Ok(outcome % 2 == 0)
///     }
/// }
/// assert!(Even.test(&4).unwrap());
/// let _filter = OutcomeFilter::Function(Arc::new(Even));
/// ```
pub trait OutcomePredicate<O> {
    /// `bool(func(outcome))`.
    fn test(&self, outcome: &O) -> Result<bool>;
}

/// A predicate prints as its kind alone: what it tests is code.
impl<O> std::fmt::Debug for dyn OutcomePredicate<O> + Send + Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutcomePredicate")
    }
}

/// `MultisetFilterOutcomes`'s target: a callable, or a collection of outcomes (Python makes it a `frozenset`
/// and tests `outcome in target_set`).
///
/// ```
/// use icepool_core::evaluator::{Count, Order};
/// use icepool_core::expression::{Operator, OutcomeFilter, Statelet};
/// use icepool_core::Int;
///
/// // `keep_outcomes([2, 4])`: an outcome outside the set counts 0.
/// let filter = OutcomeFilter::Set(vec![2, 4].into());
/// let op = Operator::FilterOutcomes { filter, invert: false };
/// let three = [Count::Int(Int::from(3))];
/// let (_, count) = op.next_state(&Statelet::None, Order::Ascending, &5, &three).unwrap();
/// assert_eq!(count, Count::Int(Int::ZERO));
/// let (_, count) = op.next_state(&Statelet::None, Order::Ascending, &4, &three).unwrap();
/// assert_eq!(count, Count::Int(Int::from(3)));
/// ```
#[derive(Clone, Debug)]
pub enum OutcomeFilter<O> {
    /// A callable: the outcome is kept iff it returns true.
    Function(Arc<dyn OutcomePredicate<O> + Send + Sync>),
    /// A collection of outcomes: the outcome is kept iff it is one of them.
    Set(Arc<[O]>),
}

/// `MultisetFilterOutcomes._next_state` keeps the count if `bool(func(outcome)) != invert`, else it is 0.
pub(super) fn keeps<O: PartialEq>(
    filter: &OutcomeFilter<O>,
    invert: bool,
    outcome: &O,
) -> Result<bool> {
    let keep = match filter {
        OutcomeFilter::Function(f) => f.test(outcome)?,
        OutcomeFilter::Set(set) => set.contains(outcome),
    };
    Ok(keep != invert)
}

/// `MultisetFilterOutcomesBinary._next_state`: the source count if the target count is positive (`!= invert`).
/// This version has a variable target.
pub(super) fn binary_next_state(invert: bool, source_count: &Int, target_count: &Int) -> Int {
    if target_count.is_positive() != invert {
        source_count.clone()
    } else {
        Int::ZERO
    }
}
