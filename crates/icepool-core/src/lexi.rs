//! Port of the lexicographic-tuple helpers of icepool `lexi.py` that the multiset operators use (`sort_pair`,
//! `versus_all` / `versus_any`, `max_pair_*`). The lexi evaluators themselves belong to a later task.

use crate::error::{Error, Result};

/// A comparison between two multisets as icepool names it (`'==', '!=', '<=', '<', '>=', '>'` or `'cmp'`).
/// Parsing happens where Python looks the string up, so an invalid one raises there.
///
/// ```
/// use icepool_core::evaluator::lexi::compute_lexi_tuple;
/// // `(left_first, tie, right_first)`: `<` holds if the left side has the first lower element.
/// assert_eq!(compute_lexi_tuple("<", 1).unwrap(), (1, 0, 0));
/// // Descending order swaps the two sides' results.
/// assert_eq!(compute_lexi_tuple("<", -1).unwrap(), (0, 0, 1));
/// assert!(compute_lexi_tuple("<>", 1).is_err());
/// ```
pub fn compute_lexi_tuple(comparison: &str, order: i64) -> Result<(i64, i64, i64)> {
    // A 3-tuple representing a lexicographic comparison between two multisets:
    // * left_first: The result if the left side had the first different element and it was paired with a later
    //   right side element.
    // * tie: The result if no elements were paired.
    // * right_first: As right_first but vice versa.
    let (mut left_first, tie, mut right_first) = match comparison {
        "==" => (0, 1, 0),
        "!=" => (1, 0, 1),
        "<=" => (1, 1, 0),
        "<" => (1, 0, 0),
        ">=" => (0, 1, 1),
        ">" => (0, 0, 1),
        "cmp" => (-1, 0, 1),
        _ => {
            return Err(Error::InvalidArgument(format!(
                "Invalid comparison {comparison}"
            )))
        }
    };
    if order < 0 {
        std::mem::swap(&mut left_first, &mut right_first);
    }
    Ok((left_first, tie, right_first))
}

/// As [`compute_lexi_tuple`], but also computes an order such that right_first is 0. Returns the tuple and the
/// order (`1` ascending, `-1` descending). Only the program runner calls it (the Python facade runs lexi.py's own).
///
/// ```
/// use icepool_core::evaluator::lexi::compute_lexi_tuple_with_zero_right_first;
/// assert_eq!(compute_lexi_tuple_with_zero_right_first("<=").unwrap(), ((1, 1, 0), 1));
/// // `>=` is `(0, 1, 1)` ascending: descending order puts the zero last.
/// assert_eq!(compute_lexi_tuple_with_zero_right_first(">=").unwrap(), ((1, 1, 0), -1));
/// ```
pub fn compute_lexi_tuple_with_zero_right_first(
    comparison: &str,
) -> Result<((i64, i64, i64), i64)> {
    let lexi_tuple = compute_lexi_tuple(comparison, 1)?;
    if lexi_tuple.2 == 0 {
        Ok((lexi_tuple, 1))
    } else {
        Ok(((lexi_tuple.2, lexi_tuple.1, lexi_tuple.0), -1))
    }
}

/// A 5-tuple representing a lexicographic comparison between two multisets, also specifying what to do when one
/// side has more elements than the other: `(left_first, left_extra, tie, right_extra, right_first)`.
///
/// * left_extra: The result if the left side had the first different element and it was never paired with a
///   right side element.
/// * right_extra: As left_extra but vice versa.
///
/// ```
/// use icepool_core::evaluator::lexi::compute_lexi_tuple_with_extra;
/// // `(left_first, left_extra, tie, right_extra, right_first)`. Ascending, extra elements count `high`: they come
/// // late, so an extra left element makes the left side greater.
/// assert_eq!(compute_lexi_tuple_with_extra("<", 1, "high").unwrap(), (1, 0, 0, 1, 0));
/// assert_eq!(compute_lexi_tuple_with_extra("<", 1, "drop").unwrap(), (1, 0, 0, 0, 0));
/// ```
pub fn compute_lexi_tuple_with_extra(
    comparison: &str,
    order: i64,
    extra: &str,
) -> Result<(i64, i64, i64, i64, i64)> {
    let (left_first, tie, right_first) = compute_lexi_tuple(comparison, order)?;
    let extra = match extra {
        "low" if order > 0 => "early",
        "low" => "late",
        "high" if order < 0 => "early",
        "high" => "late",
        extra => extra,
    };
    let (left_extra, right_extra) = match extra {
        "early" => (left_first, right_first),
        "late" => (right_first, left_first),
        "equal" => (tie, tie),
        "keep" | "continue" => (1, 1),
        "drop" | "break" => (0, 0),
        _ => return Err(Error::InvalidArgument(format!("Invalid extra {extra}"))),
    };
    Ok((left_first, left_extra, tie, right_extra, right_first))
}
