//! Port of icepool `operator/versus.py`: `MultisetVersus`, which keeps elements of the left multiset by how they
//! compare against all (or any) elements of the right (`versus_all` / `versus_any`).

use super::Statelet;
use crate::error::{Error, Result};
use crate::pool::Count;
use crate::Int;

/// `MultisetVersus._initial_state`. State is `(lexi_tuple, countdown)`: whether left elements are kept if they
/// appear before, at the same time, and after right elements (when using the preferred order).
pub(super) fn initial_state(
    lexi_tuple: (i64, i64, i64),
    self_order: i64,
    order: i64,
    child_sizes: &[Option<Count>],
) -> Result<(Statelet, Option<Count>)> {
    let (lexi_tuple, countdown) = if order == self_order {
        (lexi_tuple, Int::ONE)
    } else {
        let right_size = match &child_sizes[1] {
            None => return Err(Error::UnsupportedOrder(
                "Reverse order not supported unless the size of the right operand is inferrable."
                    .into(),
            )),
            Some(Count::Int(n)) => n.clone(),
            // `countdown = right_size`, compared with a count by every next state.
            Some(Count::Tuple(t)) => return Ok((Statelet::TupleSize(t.clone()), None)),
        };
        ((lexi_tuple.2, lexi_tuple.1, lexi_tuple.0), right_size)
    };
    Ok((
        Statelet::Versus {
            lexi_tuple,
            countdown,
        },
        None,
    ))
}

/// `MultisetVersus._next_state`.
pub(super) fn next_state(
    state: &Statelet,
    left_count: &Int,
    right_count: &Int,
) -> Result<(Statelet, Count)> {
    let Statelet::Versus {
        lexi_tuple,
        countdown,
    } = state
    else {
        unreachable!("a versus node's statelet")
    };
    let lexi_tuple = *lexi_tuple;
    let (left_first, tie, right_first) = lexi_tuple;
    let right_count = right_count.clone().max(Int::ZERO);
    let count = if *countdown > right_count {
        left_count * left_first
    } else if countdown.is_positive() {
        left_count * tie
    } else {
        left_count * right_first
    };
    let countdown = (countdown - right_count).max(Int::ZERO);
    Ok((
        Statelet::Versus {
            lexi_tuple,
            countdown,
        },
        Count::Int(count),
    ))
}
