//! Port of icepool `evaluator/keep.py`.

use super::basic::int;
use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::{Int, Value};

/// Produces the outcome at a given sorted index (`KeepEvaluator`).
///
/// The attached generator or expression must produce enough values to reach the sorted index; otherwise, this
/// raises `IndexError`. Negative incoming counts are treated as zero counts.
pub struct KeepEvaluator {
    /// The `index=` keyword. `None` (what `pool[int]` passes, after keeping the one roll) takes the first element
    /// seen.
    pub index: Option<Int>,
}

impl MultisetEvaluator<Value> for KeepEvaluator {
    /// `(result, remaining)`: the outcome found (or `None`), and how many elements are left to see before it.
    type State = (Value, Int);
    type Final = Value;

    fn initial_state(
        &self,
        order: Order,
        _: &[Value],
        sizes: &[Option<Int>],
    ) -> Result<(Value, Int)> {
        let remaining = match &self.index {
            None => Int::ONE,
            Some(index) if (order.value() > 0) != !index.is_negative() => match &sizes[0] {
                Some(size) if order.value() > 0 => size + index + 1,
                Some(size) => size - index,
                None => return Err(Error::UnsupportedOrder(String::new())),
            },
            Some(index) if !index.is_negative() => index + 1,
            Some(index) => -index,
        };
        Ok((Value::None, remaining))
    }

    fn next_state(
        &self,
        state: &(Value, Int),
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<(Value, Int)>> {
        let (result, remaining) = state;
        let remaining = (remaining - counts[0].clone().max(Int::ZERO)).max(Int::ZERO);
        let result = if remaining.is_zero() && *result == Value::None {
            outcome.clone()
        } else {
            result.clone()
        };
        Ok(Some((result, remaining)))
    }

    fn final_outcome(
        &self,
        final_state: &(Value, Int),
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        match final_state {
            (Value::None, remaining) => Err(Error::Index(format!(
                "Evaluation ended {} element(s) short of reaching the kept index.",
                int(remaining)
            ))),
            (result, _) => Ok(Final::Outcome(result.clone())),
        }
    }
}
