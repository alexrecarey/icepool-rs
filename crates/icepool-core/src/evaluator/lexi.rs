//! Port of icepool `evaluator/lexi.py`: lexicographic comparison of two multisets (`leximin` / `leximax`).

use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::Int;
use crate::Value;

/// `LexiComparisonEvaluator` with its `evaluate()` keyword arguments `sort_order` and `lexi_tuple`
/// (`compute_lexi_tuple_with_extra`).
pub struct LexiComparisonEvaluator {
    pub sort_order: Order,
    /// `(left_first, left_extra, tie, right_extra, right_first)`.
    pub lexi_tuple: (i64, i64, i64, i64, i64),
}

/// `(forward_order, extra, first, left_lead)`.
///
/// Positive on first/extra means the left side is earlier. First means the first differing element was paired
/// with a later element from the other side.
type LexiState = (bool, i64, i64, Int);

/// `initial_state(self, order, outcomes, left_size, right_size, ...)`: exactly two inputs.
fn two<T: Clone>(xs: &[T], method: &str) -> Result<(T, T)> {
    match xs {
        [a, b] => Ok((a.clone(), b.clone())),
        _ => Err(Error::Type(format!(
            "LexiComparisonEvaluator.{method}() takes two multisets, {} given",
            xs.len()
        ))),
    }
}

impl MultisetEvaluator<Value> for LexiComparisonEvaluator {
    type State = LexiState;
    type Final = Value;

    fn initial_state(&self, order: Order, _: &[Value], sizes: &[Option<Int>]) -> Result<LexiState> {
        let (left_size, right_size) = two(sizes, "initial_state")?;
        if order == self.sort_order {
            return Ok((true, 0, 0, Int::ZERO));
        }
        let (Some(left_size), Some(right_size)) = (left_size, right_size) else {
            return Err(Error::UnsupportedOrder(
                "Reverse order not supported unless sizes of both operands are inferrable.".into(),
            ));
        };
        let extra = match left_size.cmp(&right_size) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => 0,
        };
        Ok((false, extra, 0, right_size - left_size))
    }

    fn next_state(
        &self,
        state: &LexiState,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<LexiState>> {
        let (forward_order, mut extra, mut first, ref left_lead) = *state;
        let mut left_lead = left_lead.clone();
        let (left, right) = two(counts, "next_state")?;
        let (left, right) = (left.max(Int::ZERO), right.max(Int::ZERO));
        if forward_order {
            if extra == 0 {
                if left < right {
                    extra = -1;
                } else if right < left {
                    extra = 1;
                }
            } else if extra == 1 && right.is_positive() {
                first = 1;
            } else if extra == -1 && left.is_positive() {
                first = -1;
            }
        } else {
            left_lead += &left - &right;
            if left_lead.is_positive() && left.is_positive() {
                first = -1;
            } else if left_lead.is_negative() && right.is_positive() {
                first = 1;
            }
        }
        Ok(Some((forward_order, extra, first, left_lead)))
    }

    fn final_outcome(
        &self,
        final_state: &LexiState,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let (_, extra, first, _) = final_state;
        let (extra, first) = (*extra, *first);
        let (left_first, left_extra, tie, right_extra, right_first) = self.lexi_tuple;
        let result = if first > 0 {
            left_first
        } else if first < 0 {
            right_first
        } else if extra > 0 {
            left_extra
        } else if extra < 0 {
            right_extra
        } else {
            tie
        };
        Ok(Final::Outcome(Value::from(result)))
    }
}
