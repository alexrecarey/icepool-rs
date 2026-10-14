//! Port of icepool `operator/filter_outcomes.py`: keeps the designated outcomes, dropping the rest, or vice
//! versa. This is similar to `intersection` or `difference`, except the target set is considered to have
//! unlimited multiplicity.

use std::sync::Arc;

use crate::error::Result;

/// A callable returning `True` iff the outcome should be kept (Python calls `bool()` on its result).
pub trait OutcomePredicate<O> {
    fn test(&self, outcome: &O) -> Result<bool>;
}

/// `MultisetFilterOutcomes`'s target: a callable, or a collection of outcomes (Python makes it a `frozenset`
/// and tests `outcome in target_set`).
#[derive(Clone)]
pub enum OutcomeFilter<O> {
    Function(Arc<dyn OutcomePredicate<O> + Send + Sync>),
    Set(Arc<[O]>),
}

/// `MultisetFilterOutcomes._next_state`: the count if `bool(func(outcome)) != invert`, else 0.
pub(super) fn next_state<O: PartialEq>(
    filter: &OutcomeFilter<O>,
    invert: bool,
    outcome: &O,
    count: i64,
) -> Result<i64> {
    let keep = match filter {
        OutcomeFilter::Function(f) => f.test(outcome)?,
        OutcomeFilter::Set(set) => set.contains(outcome),
    };
    Ok(if keep != invert { count } else { 0 })
}

/// `MultisetFilterOutcomesBinary._next_state`: the source count if the target count is positive (`!= invert`).
/// This version has a variable target.
pub(super) fn binary_next_state(invert: bool, source_count: i64, target_count: i64) -> i64 {
    if (target_count > 0) != invert {
        source_count
    } else {
        0
    }
}
