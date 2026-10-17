//! Port of icepool `operator/pair.py`: `MultisetSortPair`, `MultisetSortPairWhile`, `MultisetMaxPairLate` and
//! `MultisetMaxPairEarly`, which pair the elements of the left multiset with those of the right.

use super::{int, Statelet};
use crate::error::{Error, Result};
use crate::lexi::compute_lexi_tuple_with_extra;
use crate::pool::Count;
use crate::Int;

const ZERO: Int = Int::ZERO;

fn two(child_sizes: &[Option<Count>]) -> (Option<&Count>, Option<&Count>) {
    (child_sizes[0].as_ref(), child_sizes[1].as_ref())
}

/// `countdown = right_size` stores the size as it is: a tuple one makes the next state raise (`TupleSize`).
fn stored(size: &Count, statelet: impl FnOnce(Int) -> Statelet) -> Statelet {
    match size {
        Count::Int(n) => statelet(n.clone()),
        Count::Tuple(t) => Statelet::TupleSize(t.clone()),
    }
}

fn order_error(comparison: &str, extra: &str) -> Error {
    Error::UninferrableSize(format!(
        "The size of the right operand must be inferrable to sort_pair({comparison}, \
         extra={extra})."
    ))
}

/// `MultisetSortPair._initial_state`.
pub(super) fn sort_pair_initial_state(
    comparison: &str,
    sort_order: i64,
    extra: &str,
    order: i64,
    child_sizes: &[Option<Count>],
) -> Result<(Statelet, Option<Count>)> {
    let (left_first, left_extra, tie, right_extra, right_first) =
        compute_lexi_tuple_with_extra(comparison, sort_order, extra)?;
    let (lexi_tuple, forward_order, left_lead, countdown);
    if order == sort_order {
        forward_order = true;
        // drop right_extra
        lexi_tuple = [left_first, left_extra, tie, right_first];
        left_lead = ZERO;
        if left_first == left_extra {
            countdown = None;
        } else {
            let (_, right_size) = two(child_sizes);
            let Some(right_size) = right_size else {
                return Err(order_error(comparison, extra));
            };
            // After `countdown` elements, the rest are considered to be extra.
            let statelet = stored(right_size, |right_size| Statelet::SortPair {
                lexi_tuple,
                forward_order,
                left_lead,
                countdown: Some(right_size),
            });
            return Ok((statelet, None));
        }
    } else {
        forward_order = false;
        // reverse, then drop the resulting right_extra
        lexi_tuple = [right_first, right_extra, tie, left_first];
        let (Some(left_size), Some(right_size)) = two(child_sizes) else {
            return Err(Error::UnsupportedOrder(
                "Reverse order not supported unless sizes of both operands are inferrable.".into(),
            ));
        };
        let (left_size, right_size) = (int(left_size)?, int(right_size)?);
        if left_first == left_extra {
            left_lead = right_size - left_size;
            countdown = None;
        } else {
            // The first `countdown` elements will be counted as extra.
            left_lead = ZERO;
            countdown = Some((left_size - right_size).max(ZERO));
        }
    }
    Ok((
        Statelet::SortPair {
            lexi_tuple,
            forward_order,
            left_lead,
            countdown,
        },
        None,
    ))
}

/// The pairing step shared by both sort_pair operators: how many of `left_count` elements pair with right
/// elements seen before (`right_first_count`), with right elements of this outcome (`tie_count`), and with right
/// elements seen later (`left_first_count`). Updates `left_lead`.
fn pair_counts(left_lead: &mut Int, left_count: &Int, right_count: &Int) -> (Int, Int, Int) {
    // The number of left elements that paired with previously-seen right elements.
    let right_first_count = (-&*left_lead).min(left_count.clone()).max(ZERO);
    let tie_count = if !left_lead.is_negative() {
        (right_count - &*left_lead)
            .min(left_count.clone())
            .max(ZERO)
    } else {
        (left_count + &*left_lead)
            .min(right_count.clone())
            .max(ZERO)
    };
    *left_lead += left_count - right_count;
    let left_first_count = left_lead.clone().min(left_count.clone()).max(ZERO);
    (right_first_count, tie_count, left_first_count)
}

/// `MultisetSortPair._next_state`.
pub(super) fn sort_pair_next_state(
    state: &Statelet,
    left_count: &Int,
    right_count: &Int,
) -> Result<(Statelet, Count)> {
    let Statelet::SortPair {
        lexi_tuple,
        forward_order,
        left_lead,
        countdown,
    } = state
    else {
        unreachable!("a sort_pair node's statelet")
    };
    let (lexi_tuple, forward_order) = (*lexi_tuple, *forward_order);
    let (mut left_lead, mut countdown) = (left_lead.clone(), countdown.clone());
    let [left_first, left_extra, tie, right_first] = lexi_tuple;
    let mut left_count = left_count.clone();
    let extra_count;
    match &mut countdown {
        None => {
            extra_count = ZERO;
            left_count = left_count.max(ZERO);
        }
        Some(countdown) => {
            if forward_order {
                extra_count = (&left_count - &*countdown).max(ZERO);
                left_count = left_count.min(countdown.clone());
                *countdown -= &left_count;
            } else {
                extra_count = left_count.clone().min(countdown.clone());
                left_count = (&left_count - &*countdown).max(ZERO);
                *countdown -= &extra_count;
            }
        }
    }
    let right_count = right_count.clone().max(ZERO);
    let (right_first_count, tie_count, left_first_count) =
        pair_counts(&mut left_lead, &left_count, &right_count);
    let count = right_first_count * right_first
        + tie_count * tie
        + left_first_count * left_first
        + extra_count * left_extra;
    Ok((
        Statelet::SortPair {
            lexi_tuple,
            forward_order,
            left_lead,
            countdown,
        },
        Count::Int(count),
    ))
}

/// `MultisetSortPairWhile._initial_state`.
pub(super) fn sort_pair_while_initial_state(
    comparison: &str,
    sort_order: i64,
    extra: &str,
    order: i64,
    child_sizes: &[Option<Count>],
) -> Result<(Statelet, Option<Count>)> {
    if order != sort_order {
        return Err(Error::UnsupportedOrder(
            "MultisetSortPairWhile must be evaluated in sort order.".into(),
        ));
    }
    let (left_first, left_extra, tie, _right_extra, right_first) =
        compute_lexi_tuple_with_extra(comparison, sort_order, extra)?;
    // drop right_extra
    let lexi_tuple = [left_first, left_extra, tie, right_first];
    let statelet = |countdown| Statelet::SortPairWhile {
        lexi_tuple: Some(lexi_tuple),
        left_lead: Some(ZERO),
        countdown,
    };
    if left_first == left_extra {
        return Ok((statelet(None), None));
    }
    let (_, right_size) = two(child_sizes);
    let Some(right_size) = right_size else {
        return Err(order_error(comparison, extra));
    };
    // After `countdown` elements, the rest are considered to be extra.
    Ok((stored(right_size, |n| statelet(Some(n))), None))
}

/// `MultisetSortPairWhile._next_state`. `keep`: elements are kept as long as the comparison holds, then the
/// remaining elements are dropped; otherwise dropped while it holds, then kept.
pub(super) fn sort_pair_while_next_state(
    keep: bool,
    state: &Statelet,
    left_count: &Int,
    right_count: &Int,
) -> Result<(Statelet, Count)> {
    let Statelet::SortPairWhile {
        lexi_tuple,
        left_lead,
        countdown,
    } = state
    else {
        unreachable!("a sort_pair_while node's statelet")
    };
    let (lexi_tuple, left_lead, mut countdown) =
        (*lexi_tuple, left_lead.clone(), countdown.clone());
    let mut left_count = left_count.clone();
    let extra_count;
    match &mut countdown {
        None => {
            extra_count = ZERO;
            left_count = left_count.max(ZERO);
        }
        Some(countdown) => {
            extra_count = (&left_count - &*countdown).max(ZERO);
            left_count = left_count.min(countdown.clone());
            *countdown -= &left_count;
        }
    }

    let (Some([left_first, left_extra, tie, right_first]), Some(mut left_lead)) =
        (lexi_tuple, left_lead)
    else {
        let count = if keep { ZERO } else { left_count };
        let state = Statelet::SortPairWhile {
            lexi_tuple: None,
            left_lead: None,
            countdown,
        };
        return Ok((state, Count::Int(count)));
    };
    let right_count = right_count.clone().max(ZERO);
    let (right_first_count, tie_count, left_first_count) =
        pair_counts(&mut left_lead, &left_count, &right_count);

    let (mut lexi_tuple, mut left_lead) = (lexi_tuple, Some(left_lead));
    let mut end = || {
        lexi_tuple = None;
        left_lead = None;
    };
    let mut count = if right_first_count.is_positive() && right_first == 0 {
        end();
        ZERO
    } else if tie_count.is_positive() && tie == 0 {
        end();
        right_first_count
    } else if left_first_count.is_positive() && left_first == 0 {
        end();
        right_first_count + tie_count
    } else if extra_count.is_positive() && left_extra == 0 {
        end();
        right_first_count + tie_count + left_first_count
    } else {
        &left_count + &extra_count
    };
    if !keep {
        count = left_count + extra_count - count;
    }
    Ok((
        Statelet::SortPairWhile {
            lexi_tuple,
            left_lead,
            countdown,
        },
        Count::Int(count),
    ))
}

fn max_pair_wrong_order(self_order: i64) -> Error {
    Error::UnsupportedOrder(if self_order > 0 {
        "max_pair methods with priority='low' must be evaluated in ascending order.".into()
    } else {
        "max_pair methods with priority='high' must be evaluated in descending order.".into()
    })
}

/// `MultisetMaxPairLate._initial_state`: `prev_pairable`, the number of previously-seen elements that are
/// eligible to be paired.
pub(super) fn max_pair_late_initial_state(
    self_order: i64,
    order: i64,
) -> Result<(Statelet, Option<Count>)> {
    if order == self_order {
        Ok((Statelet::MaxPairLate(ZERO), None))
    } else {
        Err(max_pair_wrong_order(self_order))
    }
}

/// `MultisetMaxPairLate._next_state`.
pub(super) fn max_pair_late_next_state(
    pair_equal: bool,
    keep: bool,
    state: &Statelet,
    left_count: &Int,
    right_count: &Int,
) -> Result<(Statelet, Count)> {
    let Statelet::MaxPairLate(prev_pairable) = state else {
        unreachable!("a max_pair node's statelet")
    };
    let left_count = left_count.clone().max(ZERO);
    let right_count = right_count.clone().max(ZERO);
    let new_pairs = if pair_equal {
        (prev_pairable + &right_count).min(left_count.clone())
    } else {
        prev_pairable.clone().min(left_count.clone())
    };
    let prev_pairable = prev_pairable + (right_count - &new_pairs);
    let count = if keep {
        new_pairs
    } else {
        left_count - new_pairs
    };
    Ok((Statelet::MaxPairLate(prev_pairable), Count::Int(count)))
}

/// `MultisetMaxPairEarly._initial_state`: `(right_unpaired, right_remaining)`, the number of remaining right
/// elements that haven't been paired yet and the number of remaining right elements overall.
pub(super) fn max_pair_early_initial_state(
    self_order: i64,
    order: i64,
    child_sizes: &[Option<Count>],
) -> Result<(Statelet, Option<Count>)> {
    let (_, right_size) = two(child_sizes);
    if order == self_order {
        let Some(right_size) = right_size else {
            return Err(Error::UnsupportedOrder(
                "The size of the right operand must be inferrable.".into(),
            ));
        };
        let statelet = stored(right_size, |n| Statelet::MaxPairEarly(n.clone(), n));
        Ok((statelet, None))
    } else {
        Err(max_pair_wrong_order(self_order))
    }
}

/// `MultisetMaxPairEarly._next_state`.
pub(super) fn max_pair_early_next_state(
    pair_equal: bool,
    keep: bool,
    state: &Statelet,
    left_count: &Int,
    right_count: &Int,
) -> Result<(Statelet, Count)> {
    let Statelet::MaxPairEarly(right_unpaired, right_remaining) = state else {
        unreachable!("a max_pair node's statelet")
    };
    let (mut right_unpaired, mut right_remaining) =
        (right_unpaired.clone(), right_remaining.clone());
    let left_count = left_count.clone().max(ZERO);
    let right_count = right_count.clone().max(ZERO);
    let mut count;
    if pair_equal {
        count = right_unpaired.clone().min(left_count.clone());
        right_unpaired -= &count;
        right_remaining -= &right_count;
        right_unpaired = right_unpaired.min(right_remaining.clone());
    } else {
        right_remaining -= &right_count;
        right_unpaired = right_unpaired.min(right_remaining.clone());
        count = right_unpaired.clone().min(left_count.clone());
        right_unpaired -= &count;
    }
    if !keep {
        count = left_count - count;
    }
    Ok((
        Statelet::MaxPairEarly(right_unpaired, right_remaining),
        Count::Int(count),
    ))
}
