//! Port of icepool `evaluator/basic.py`: basic evaluators.
//!
//! The built-in evaluators work on the dynamic [`Value`]: their states are Python objects (`None` before the
//! first outcome, tuples, ints) and they use Python's operators on outcomes, so every outcome type behaves as in
//! Python.

use super::{Final, MultisetEvaluator};
use crate::counts::py_sorted;
use crate::error::Result;
use crate::order::Order;
use crate::outcome::Outcome;
use crate::value::BinOp;
use crate::Value;

/// The default `final_outcome`: the final state itself (`None` is refused by the driver).
pub(crate) fn state_as_outcome(state: &Value) -> Final<Value> {
    match state {
        Value::None => Final::None,
        state => Final::Outcome(state.clone()),
    }
}

pub(crate) fn int(i: i64) -> Value {
    Value::from(i)
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

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        // `(state or ()) + (outcome, ) * count`
        let mut items: Vec<Value> = match state {
            Value::Tuple(t) => t.to_vec(),
            _ => Vec::new(),
        };
        let count = usize::try_from(counts[0]).unwrap_or(0);
        items.extend(std::iter::repeat_n(outcome.clone(), count));
        Ok(Some(Value::tuple(items)))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        let items: Vec<Value> = match final_state {
            Value::Tuple(t) => t.to_vec(),
            _ => return Ok(Final::Outcome(Value::tuple(Vec::new()))),
        };
        // `sorted(final_state, reverse=reverse)`: a reverse sort reverses, sorts stably and reverses back.
        let sorted = if self.ascending {
            py_sorted(items, |a, b| a.py_lt(b))?
        } else {
            let mut items = items;
            items.reverse();
            let mut sorted = py_sorted(items, |a, b| a.py_lt(b))?;
            sorted.reverse();
            sorted
        };
        Ok(Final::Outcome(Value::tuple(sorted)))
    }
}

/// Sums all outcomes (`SumEvaluator()` without a map).
pub struct SumEvaluator;

impl MultisetEvaluator<Value> for SumEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        let count = counts[0];
        Ok(Some(match state {
            Value::None => {
                if count == 1 {
                    // The outcome does not need to support addition or multiplication in this case.
                    outcome.clone()
                } else {
                    outcome.binary(BinOp::Mul, &int(count))?
                }
            }
            state => state.binary(BinOp::Add, &outcome.binary(BinOp::Mul, &int(count))?)?,
        }))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
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

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        Ok(int(0))
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        Ok(Some(state.binary(BinOp::Add, &int(counts[0]))?))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        // `final_state or 0`
        Ok(Final::Outcome(if final_state.truthy() {
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

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        Ok(int(1))
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        Ok(Some(state.binary(BinOp::Mul, &int(counts[0]))?))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// Returns `True` iff the multiset contains only zero counts (`EmptyEvaluator`); with `any=True`, the opposite.
pub struct EmptyEvaluator {
    pub any: bool,
}

impl MultisetEvaluator<Value> for EmptyEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        Ok(Value::Bool(true))
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        // `state and (count == 0)`
        Ok(Some(if state.truthy() {
            Value::Bool(counts[0] == 0)
        } else {
            state.clone()
        }))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        Ok(if self.any {
            Final::Outcome(Value::Bool(!final_state.truthy()))
        } else {
            state_as_outcome(final_state)
        })
    }
}
