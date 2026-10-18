//! Port of icepool `evaluator/keep.py`.

use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::Int;

/// Produces the outcome at a given sorted index (`KeepEvaluator`).
///
/// The attached generator or expression must produce enough values to reach the sorted index; otherwise, this
/// raises `IndexError`. Negative incoming counts are treated as zero counts.
pub struct KeepEvaluator {
    /// The `index=` keyword. `None` (what `pool[int]` passes, after keeping the one roll) takes the first element
    /// seen.
    pub index: Option<Int>,
}

/// Not a `CountFold`: `initial_state` reads the size.
impl<O: crate::outcome::OutcomeOrder> MultisetEvaluator<O> for KeepEvaluator {
    /// `(result, remaining)`: the outcome found (or `None`), and how many elements are left to see before it.
    type State = (Option<O>, Int);
    type Final = O;

    fn initial_state(&self, order: Order, _: &[O], sizes: &[Option<Int>]) -> Result<Self::State> {
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
        Ok((None, remaining))
    }

    fn next_state(
        &self,
        state: &Self::State,
        _: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        let (result, remaining) = state;
        let remaining = (remaining - counts[0].clone().max(Int::ZERO)).max(Int::ZERO);
        let result = match result {
            None if remaining.is_zero() => Some(outcome.clone()),
            result => result.clone(),
        };
        Ok(Some((result, remaining)))
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[O],
        _: &[Option<Int>],
    ) -> Result<Final<O>> {
        match final_state {
            (None, remaining) => Err(Error::IndexOutOfRange(format!(
                "Evaluation ended {remaining} element(s) short of reaching the kept index."
            ))),
            (Some(result), _) => Ok(Final::Outcome(result.clone())),
        }
    }
}
