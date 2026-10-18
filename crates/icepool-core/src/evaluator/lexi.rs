//! Port of icepool `evaluator/lexi.py`: lexicographic comparison of two multisets (`leximin` / `leximax`).

use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::Int;

// icepool `lexi.py`'s helpers, which compute the `lexi_tuple` these evaluators (and the multiset operators) take.
pub use crate::lexi::{
    compute_lexi_tuple, compute_lexi_tuple_with_extra, compute_lexi_tuple_with_zero_right_first,
};

/// `LexiComparisonEvaluator` with its `evaluate()` keyword arguments `sort_order` and `lexi_tuple`
/// (`compute_lexi_tuple_with_extra`).
///
/// ```
/// use icepool_core::evaluator::lexi::{compute_lexi_tuple_with_extra, LexiComparisonEvaluator};
/// use icepool_core::evaluator::{evaluate, MultisetSource};
/// use icepool_core::{d, Die, Int, Order};
/// // Python's `d(6).pool(2).leximax('>', d(6).pool(2))`: does the left pair win, highest die first?
/// let evaluator = LexiComparisonEvaluator {
///     sort_order: Order::Descending,
///     lexi_tuple: compute_lexi_tuple_with_extra(">", Order::Descending.value(), "high").unwrap(),
/// };
/// let pair = MultisetSource::Pool(d(6).pool(2).source());
/// let result = Die::from_weights(evaluate(&evaluator, &[pair.clone(), pair]).unwrap());
/// assert_eq!(result, Die::from_weights([(Int::from(0), 681u32), (Int::from(1), 615)]));
/// ```
pub struct LexiComparisonEvaluator {
    /// The order in which the outcomes are compared.
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
pub(crate) fn two<T: Clone>(xs: &[T], method: &str) -> Result<(T, T)> {
    match xs {
        [a, b] => Ok((a.clone(), b.clone())),
        _ => Err(Error::WrongType(format!(
            "LexiComparisonEvaluator.{method}() takes two multisets, {} given",
            xs.len()
        ))),
    }
}

/// The comparison never looks at the outcomes themselves, only at the counts: any outcome type will do.
impl<O> MultisetEvaluator<O> for LexiComparisonEvaluator {
    type State = LexiState;
    /// One of `lexi_tuple`'s elements.
    type Final = Int;

    fn initial_state(&self, order: Order, _: &[O], sizes: &[Option<Int>]) -> Result<LexiState> {
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
        _: &O,
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
        _: &[O],
        _: &[Option<Int>],
    ) -> Result<Final<Int>> {
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
        Ok(Final::Outcome(Int::from(result)))
    }
}
