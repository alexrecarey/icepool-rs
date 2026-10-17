//! Port of icepool `evaluator/basic.py`: basic evaluators.
//!
//! The built-in evaluators work on the dynamic [`Value`]: their states are Python objects (`None` before the
//! first outcome, tuples, ints) and they use Python's operators on outcomes, so every outcome type behaves as in
//! Python.

use super::{Final, MultisetEvaluator};
use crate::error::Result;
use crate::order::Order;
use crate::value::sort::{sorted, sorted_reverse};
use crate::value::BinOp;
use crate::Int;
use crate::Value;

/// The default `final_outcome`: the final state itself (`None` is refused by the driver).
pub(crate) fn state_as_outcome(state: &Value) -> Final<Value> {
    match state {
        Value::None => Final::None,
        state => Final::Outcome(state.clone()),
    }
}

pub(crate) fn int(i: impl Into<Value>) -> Value {
    i.into()
}

/// All elements of the multiset (`ExpandEvaluator`).
///
/// This is expensive and not recommended unless there are few possibilities. Outcomes with negative count will
/// be treated as 0 count.
pub struct ExpandEvaluator {
    /// The `order=` keyword: `Order.Ascending` (by `==`, so `1` or `True` too) sorts ascending, anything else
    /// descending.
    pub ascending: bool,
}

impl MultisetEvaluator<Value> for ExpandEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        // `(state or ()) + (outcome, ) * count`
        let mut items: Vec<Value> = match state {
            Value::Tuple(t) => t.to_vec(),
            _ => Vec::new(),
        };
        let count = crate::keep_tuple::list_len(&counts[0])?;
        items.extend(std::iter::repeat_n(outcome.clone(), count));
        Ok(Some(Value::tuple(items)))
    }

    /// A tuple count: `(outcome, ) * count` is `TypeError` (a tuple times a tuple).
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        let state = if state.truthy()? {
            state.clone()
        } else {
            Value::tuple(Vec::new())
        };
        let more = Value::tuple(vec![outcome.clone()]).binary(BinOp::Mul, &counts[0])?;
        Ok(Some(state.binary(BinOp::Add, &more)?))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let items: Vec<Value> = match final_state {
            Value::Tuple(t) => t.to_vec(),
            _ => return Ok(Final::Outcome(Value::tuple(Vec::new()))),
        };
        // `sorted(final_state, reverse=reverse)`.
        let sorted = if self.ascending {
            sorted(items)?
        } else {
            sorted_reverse(items)?
        };
        Ok(Final::Outcome(Value::tuple(sorted)))
    }
}

/// Sums all outcomes (`SumEvaluator()` without a map).
pub struct SumEvaluator;

impl MultisetEvaluator<Value> for SumEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        Ok(Some(add_to_sum(state, outcome.clone(), &int(&counts[0]))?))
    }

    /// A tuple count: `count == 1` is `False`, an `int` outcome times it repeats the tuple, and `state + ...`
    /// concatenates.
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        Ok(Some(add_to_sum(state, outcome.clone(), &counts[0])?))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// `SumEvaluator.next_state` after the (optional) map: `outcome * count` added to the state.
fn add_to_sum(state: &Value, outcome: Value, count: &Value) -> Result<Value> {
    Ok(match state {
        Value::None => {
            if *count == int(1) {
                // The outcome does not need to support addition or multiplication in this case.
                outcome
            } else {
                outcome.binary(BinOp::Mul, count)?
            }
        }
        state => state.binary(BinOp::Add, &outcome.binary(BinOp::Mul, count)?)?,
    })
}

/// `SumEvaluator(map)`: outcomes are mapped by `map` just before summing. A callable is called with each outcome
/// (every outcome the evaluation sees, zero counts included); for a mapping, unmapped outcomes are left as-is
/// (`map_dict.get(outcome, outcome)`). The caller builds `map` accordingly.
pub struct MappedSumEvaluator<F> {
    /// The function applied to each outcome before it is summed.
    pub map: F,
}

impl<F: Fn(&Value) -> Result<Value>> MultisetEvaluator<Value> for MappedSumEvaluator<F> {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        Ok(Some(add_to_sum(state, (self.map)(outcome)?, &int(&counts[0]))?))
    }

    /// A tuple count, as for [`SumEvaluator`].
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        Ok(Some(add_to_sum(state, (self.map)(outcome)?, &counts[0])?))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// Returns the total number of elements, i.e. the sum of all counts (`SizeEvaluator`).
///
/// Usually not very interesting unless the counts are adjusted by `unique` etc.
pub struct SizeEvaluator;

impl MultisetEvaluator<Value> for SizeEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(int(0))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        self.next_state_values(state, order, outcome, &[int(&counts[0])])
    }

    /// `state + count`: `0 + tuple` is `TypeError`.
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        Ok(Some(state.binary(BinOp::Add, &counts[0])?))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        // `final_state or 0`
        Ok(Final::Outcome(if final_state.truthy()? {
            final_state.clone()
        } else {
            int(0)
        }))
    }
}

/// Returns the product of all counts (`ProductOfCountsEvaluator`).
pub struct ProductOfCountsEvaluator;

impl MultisetEvaluator<Value> for ProductOfCountsEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(int(1))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        self.next_state_values(state, order, outcome, &[int(&counts[0])])
    }

    /// `state * count`: `1 * tuple` is the tuple, then a tuple times a tuple is `TypeError`.
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        Ok(Some(state.binary(BinOp::Mul, &counts[0])?))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// Returns `True` iff the multiset contains only zero counts (`EmptyEvaluator`); with `any=True`, the opposite.
pub struct EmptyEvaluator {
    /// Whether the result is `any` (some count is positive) rather than `empty` (no count is).
    pub any: bool,
}

impl MultisetEvaluator<Value> for EmptyEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::Bool(true))
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        // `state and (count == 0)`
        Ok(Some(if state.truthy()? {
            Value::Bool(counts[0].is_zero())
        } else {
            state.clone()
        }))
    }

    /// A tuple count: `count == 0` is `False`.
    fn next_state_values(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>> {
        Ok(Some(if state.truthy()? {
            Value::Bool(counts[0] == int(0))
        } else {
            state.clone()
        }))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(if self.any {
            Final::Outcome(Value::Bool(!final_state.truthy()?))
        } else {
            state_as_outcome(final_state)
        })
    }
}
