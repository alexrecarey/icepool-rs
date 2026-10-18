//! Port of icepool `evaluator/basic.py`: basic evaluators.
//!
//! Each evaluator is a [`CountFold`] over any outcome type, with typed states and final outcomes: sizes and
//! products of counts are [`Int`]s, `empty` a `bool`, `expand` a `Vec` of outcomes, a sum an outcome
//! ([`Summand`]). The `Value` layer (`value/evaluators.rs`) turns the finals into Python values and runs Python's
//! own arithmetic on tuple counts (a `MultiDeal` input), which have no typed meaning.

use std::marker::PhantomData;

use super::fold::{count_fold_evaluators, CountFold};
use super::Final;
use crate::error::Result;
use crate::order::Order;
use crate::outcome::{sorted, OutcomeArith, OutcomeOrder};
use crate::Int;

count_fold_evaluators!(
    ExpandEvaluator,
    SumEvaluator,
    MappedSumEvaluator<F, P>,
    SizeEvaluator,
    ProductOfCountsEvaluator,
    EmptyEvaluator,
);

/// All elements of the multiset (`ExpandEvaluator`).
///
/// This is expensive and not recommended unless there are few possibilities. Outcomes with negative count will
/// be treated as 0 count.
///
/// ```
/// use icepool_core::evaluator::basic::ExpandEvaluator;
/// use icepool_core::{d, Die};
/// // Python's `d(2).pool(2).expand()`.
/// let result: Die<Vec<i64>> = d(2).pool(2).evaluate(&ExpandEvaluator { ascending: true }).unwrap();
/// assert_eq!(result, Die::from_weights([(vec![1, 1], 1u32), (vec![1, 2], 2), (vec![2, 2], 1)]));
/// ```
#[derive(Debug)]
pub struct ExpandEvaluator {
    /// The `order=` keyword: `Order.Ascending` (by `==`, so `1` or `True` too) sorts ascending, anything else
    /// descending.
    pub ascending: bool,
}

impl<O: OutcomeOrder> CountFold<O> for ExpandEvaluator {
    /// The elements so far, in the order seen (Python's `state or ()`).
    type State = Vec<O>;
    type Final = Vec<O>;

    fn fold(&self, state: &Vec<O>, _: Order, outcome: &O, counts: &[Int]) -> Result<Vec<O>> {
        // `(state or ()) + (outcome, ) * count`
        let count = crate::keep_tuple::list_len(&counts[0])?;
        let mut state = state.clone();
        state.extend(std::iter::repeat_n(outcome.clone(), count));
        Ok(state)
    }

    fn finish(&self, state: &Vec<O>) -> Result<Final<Vec<O>>> {
        // `tuple(sorted(final_state, reverse=reverse))`
        let sorted = if self.ascending {
            sorted(state.clone())?
        } else {
            sorted_reverse(state.clone())?
        };
        Ok(Final::Outcome(sorted))
    }
}

/// `sorted(outcomes, reverse=True)`: a reverse sort reverses, sorts stably and reverses back (so equal outcomes
/// keep their order).
fn sorted_reverse<O: OutcomeOrder>(mut outcomes: Vec<O>) -> Result<Vec<O>> {
    outcomes.reverse();
    let mut outcomes = sorted(outcomes)?;
    outcomes.reverse();
    Ok(outcomes)
}

/// What a sum adds up: outcomes with arithmetic. Python's sum starts from `None` and asks `state is None` at every
/// step, so a sum that *is* `None` (a sum's map may return it) starts over; only Python's `Value` has such a value.
///
/// ```
/// use icepool_core::evaluator::basic::{SumEvaluator, Summand};
/// use icepool_core::{d, Die, Outcome};
/// // Any summand can be summed by `SumEvaluator`; a typed integer is never Python's `None`.
/// fn sum_of_two<O: Summand + Outcome>(die: &Die<O>) -> Die<O> {
///     die.pool(2).evaluate(&SumEvaluator).unwrap()
/// }
/// assert!(!3i64.is_none());
/// assert_eq!(sum_of_two(&d(4)), &d(4) + &d(4));
/// ```
pub trait Summand: OutcomeArith {
    /// Whether this is Python's `None`.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::Summand;
    /// use icepool_core::Value;
    /// // Only the dynamic `Value` has a `None`; a typed integer never is.
    /// assert!(Value::None.is_none());
    /// assert!(!Value::Int(0.into()).is_none());
    /// assert!(!0i64.is_none());
    /// ```
    fn is_none(&self) -> bool {
        false
    }
}

macro_rules! summand {
    ($($t:ty),*) => {$(
        impl Summand for $t {}
    )*};
}

summand!(i8, i16, i32, i64, i128, isize);

/// `SumEvaluator.next_state` after the (optional) map: `outcome * count` added to the state.
fn add_to_sum<P: Summand>(state: &Option<P>, outcome: P, count: &Int) -> Result<Option<P>> {
    let sum = match state {
        None if *count == Int::ONE => {
            // The outcome does not need to support addition or multiplication in this case.
            outcome
        }
        None => outcome.try_mul_count(count)?,
        Some(state) => state.try_add(&outcome.try_mul_count(count)?)?,
    };
    Ok(Some(sum).filter(|sum| !sum.is_none()))
}

/// The default `final_outcome`: the final state itself (`None` is refused by the driver).
fn state_as_outcome<P: Clone>(state: &Option<P>) -> Final<P> {
    state.clone().map_or(Final::None, Final::Outcome)
}

/// Sums all outcomes (`SumEvaluator()` without a map).
///
/// ```
/// use icepool_core::evaluator::basic::SumEvaluator;
/// use icepool_core::d;
/// // Python's `d(6).pool(2).sum()`.
/// assert_eq!(d(6).pool(2).evaluate(&SumEvaluator).unwrap(), &d(6) + &d(6));
/// ```
#[derive(Debug)]
pub struct SumEvaluator;

impl<O: Summand> CountFold<O> for SumEvaluator {
    /// `None` before the first outcome, then the sum.
    type State = Option<O>;
    type Final = O;

    fn fold(&self, state: &Option<O>, _: Order, outcome: &O, counts: &[Int]) -> Result<Option<O>> {
        add_to_sum(state, outcome.clone(), &counts[0])
    }

    fn finish(&self, state: &Option<O>) -> Result<Final<O>> {
        Ok(state_as_outcome(state))
    }
}

/// `SumEvaluator(map)`: outcomes are mapped by `map` just before summing. A callable is called with each outcome
/// (every outcome the evaluation sees, zero counts included); for a mapping, unmapped outcomes are left as-is
/// (`map_dict.get(outcome, outcome)`). The caller builds `map` accordingly.
///
/// ```
/// use icepool_core::evaluator::basic::MappedSumEvaluator;
/// use icepool_core::{d, Die, Result};
/// // Python's `d(6).pool(3).sum(map=lambda x: 1 if x >= 5 else 0)`: the number of 5s and 6s in 3d6.
/// let successes = MappedSumEvaluator::new(|&x: &i64| -> Result<i64> { Ok(i64::from(x >= 5)) });
/// let result = d(6).pool(3).evaluate(&successes).unwrap();
/// assert_eq!(result, Die::from_weights([(0, 64u32), (1, 96), (2, 48), (3, 8)]));
/// ```
#[derive(Debug)]
pub struct MappedSumEvaluator<F, P> {
    /// The function applied to each outcome before it is summed.
    pub map: F,
    summand: PhantomData<fn() -> P>,
}

impl<F, P> MappedSumEvaluator<F, P> {
    /// Sums the outcomes mapped by `map`.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::MappedSumEvaluator;
    /// use icepool_core::{d, Die, Result};
    /// // Python's `d(3).pool(2).sum(map={3: 0})`: an unmapped outcome is left as it is.
    /// let threes_count_zero = MappedSumEvaluator::new(|&x: &i64| -> Result<i64> { Ok(if x == 3 { 0 } else { x }) });
    /// let result = d(3).pool(2).evaluate(&threes_count_zero).unwrap();
    /// assert_eq!(result, Die::from_weights([(0, 1u32), (1, 2), (2, 3), (3, 2), (4, 1)]));
    /// ```
    pub fn new(map: F) -> Self {
        MappedSumEvaluator {
            map,
            summand: PhantomData,
        }
    }
}

impl<O, F: Fn(&O) -> Result<P>, P: Summand> CountFold<O> for MappedSumEvaluator<F, P> {
    /// `None` before the first outcome, then the sum.
    type State = Option<P>;
    type Final = P;

    fn fold(&self, state: &Option<P>, _: Order, outcome: &O, counts: &[Int]) -> Result<Option<P>> {
        add_to_sum(state, (self.map)(outcome)?, &counts[0])
    }

    fn finish(&self, state: &Option<P>) -> Result<Final<P>> {
        Ok(state_as_outcome(state))
    }
}

/// Returns the total number of elements, i.e. the sum of all counts (`SizeEvaluator`).
///
/// Usually not very interesting unless the counts are adjusted by `unique` etc.
///
/// ```
/// use icepool_core::evaluator::basic::SizeEvaluator;
/// use icepool_core::{d, Die, Int};
/// // Python's `d(6).pool(4).highest(2).size()`: only the kept dice count.
/// let result: Die<Int> = d(6).pool(4).highest(2).evaluate(&SizeEvaluator).unwrap();
/// assert_eq!(result, Die::from_weights([(Int::from(2), 1296u32)]));
/// ```
#[derive(Debug)]
pub struct SizeEvaluator;

impl<O> CountFold<O> for SizeEvaluator {
    /// Starts at `0` (`Int::default()`).
    type State = Int;
    type Final = Int;

    fn fold(&self, state: &Int, _: Order, _: &O, counts: &[Int]) -> Result<Int> {
        Ok(state + &counts[0])
    }

    fn finish(&self, state: &Int) -> Result<Final<Int>> {
        // `final_state or 0`: the same int.
        Ok(Final::Outcome(state.clone()))
    }
}

/// Returns the product of all counts (`ProductOfCountsEvaluator`).
///
/// ```
/// use icepool_core::evaluator::basic::ProductOfCountsEvaluator;
/// use icepool_core::{d, Die, Int};
/// // Python's `d(2).pool(3).product_of_counts()`: 0 unless both outcomes were rolled.
/// let result: Die<Int> = d(2).pool(3).evaluate(&ProductOfCountsEvaluator).unwrap();
/// assert_eq!(result, Die::from_weights([(Int::from(0), 2u32), (Int::from(2), 6)]));
/// ```
#[derive(Debug)]
pub struct ProductOfCountsEvaluator;

impl<O> CountFold<O> for ProductOfCountsEvaluator {
    type State = Int;
    type Final = Int;

    fn initial(&self, _: Order, _: &[O]) -> Result<Int> {
        Ok(Int::ONE)
    }

    fn fold(&self, state: &Int, _: Order, _: &O, counts: &[Int]) -> Result<Int> {
        Ok(state * &counts[0])
    }

    fn finish(&self, state: &Int) -> Result<Final<Int>> {
        Ok(Final::Outcome(state.clone()))
    }
}

/// Returns `True` iff the multiset contains only zero counts (`EmptyEvaluator`); with `any=True`, the opposite.
///
/// ```
/// use icepool_core::evaluator::basic::EmptyEvaluator;
/// use icepool_core::{d, Die};
/// // Python's `d(6).pool(2).highest(0).empty()` and `d(6).pool(2).any()`.
/// let empty = d(6).pool(2).highest(0).evaluate(&EmptyEvaluator { any: false }).unwrap();
/// assert_eq!(empty, Die::from_weights([(true, 36u32)]));
/// let any = d(6).pool(2).evaluate(&EmptyEvaluator { any: true }).unwrap();
/// assert_eq!(any, Die::from_weights([(true, 36u32)]));
/// ```
#[derive(Debug)]
pub struct EmptyEvaluator {
    /// Whether the result is `any` (some count is nonzero) rather than `empty` (every count is zero).
    pub any: bool,
}

impl EmptyEvaluator {
    /// `final_outcome`: `not final_state` for `any`, else the final state.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::EmptyEvaluator;
    /// // The final state says whether every count was zero.
    /// assert!(EmptyEvaluator { any: false }.final_of(true));
    /// assert!(!EmptyEvaluator { any: true }.final_of(true));
    /// assert!(EmptyEvaluator { any: true }.final_of(false));
    /// ```
    pub fn final_of(&self, empty: bool) -> bool {
        empty != self.any
    }
}

impl<O> CountFold<O> for EmptyEvaluator {
    /// Whether every count so far was zero.
    type State = bool;
    type Final = bool;

    fn initial(&self, _: Order, _: &[O]) -> Result<bool> {
        Ok(true)
    }

    fn fold(&self, state: &bool, _: Order, _: &O, counts: &[Int]) -> Result<bool> {
        // `state and (count == 0)`
        Ok(*state && counts[0].is_zero())
    }

    fn finish(&self, state: &bool) -> Result<Final<bool>> {
        Ok(Final::Outcome(self.final_of(*state)))
    }
}

#[cfg(test)]
mod tests {
    //! The expected dice are Python icepool's answers; the comment above each gives the Python.

    use super::*;
    use crate::evaluator::test_util::{evaluate_repr, expected, pool};
    use crate::value::evaluators::PyEvaluator;

    #[test]
    fn sum_and_size_count_only_kept_dice() {
        // `SumEvaluator().evaluate(d(3).pool(2).highest(1))`
        let highest = pool(3, &[0, 1]);
        let sum = evaluate_repr(&PyEvaluator(SumEvaluator), std::slice::from_ref(&highest));
        assert_eq!(sum, expected(&[("1", 1), ("2", 3), ("3", 5)]));
        // `SizeEvaluator().evaluate(d(3).pool(2).highest(1))`
        assert_eq!(
            evaluate_repr(&PyEvaluator(SizeEvaluator), &[highest]),
            expected(&[("1", 9)])
        );
    }

    #[test]
    fn product_of_counts_sees_zero_counts() {
        // `ProductOfCountsEvaluator().evaluate(d(2).pool(3))`: 0 unless both outcomes were rolled.
        let product = evaluate_repr(
            &PyEvaluator(ProductOfCountsEvaluator),
            &[pool(2, &[1, 1, 1])],
        );
        assert_eq!(product, expected(&[("0", 2), ("2", 6)]));
    }
}
