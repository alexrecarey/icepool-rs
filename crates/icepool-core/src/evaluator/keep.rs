//! Port of icepool `evaluator/keep.py`.

use super::basic::int;
use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::Value;

/// Produces the outcome at a given sorted index (`KeepEvaluator`).
///
/// The attached generator or expression must produce enough values to reach the sorted index; otherwise, this
/// raises `IndexError`. Negative incoming counts are treated as zero counts.
pub struct KeepEvaluator {
    /// The `index=` keyword. `None` (what `pool[int]` passes, after keeping the one roll) takes the first element
    /// seen.
    pub index: Option<i64>,
}

impl MultisetEvaluator<Value> for KeepEvaluator {
    /// `(result, remaining)`: the outcome found (or `None`), and how many elements are left to see before it.
    type State = (Value, i64);
    type Final = Value;

    fn initial_state(
        &self,
        order: Order,
        _: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<(Value, i64)> {
        let remaining = match self.index {
            None => 1,
            Some(index) if (order.value() > 0) != (index >= 0) => match sizes[0] {
                Some(size) if order.value() > 0 => size + index + 1,
                Some(size) => size - index,
                None => return Err(Error::UnsupportedOrder(String::new())),
            },
            Some(index) if index >= 0 => index + 1,
            Some(index) => -index,
        };
        Ok((Value::None, remaining))
    }

    fn next_state(
        &self,
        state: &(Value, i64),
        _: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<(Value, i64)>> {
        let (result, remaining) = state;
        let remaining = (remaining - counts[0].max(0)).max(0);
        let result = if remaining == 0 && *result == Value::None {
            outcome.clone()
        } else {
            result.clone()
        };
        Ok(Some((result, remaining)))
    }

    fn final_outcome(
        &self,
        final_state: &(Value, i64),
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        match final_state {
            (Value::None, remaining) => Err(Error::Index(format!(
                "Evaluation ended {} element(s) short of reaching the kept index.",
                int(*remaining)
            ))),
            (result, _) => Ok(Final::Outcome(result.clone())),
        }
    }
}
