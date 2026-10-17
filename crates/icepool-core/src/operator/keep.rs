//! Port of icepool `operator/keep.py`: `MultisetKeep`, an expression to keep some of the lowest or highest
//! elements of a multiset.
//!
//! This is less capable than the `KeepGenerator` version: it does not know how many elements it is selecting from
//! unless the child's size is inferrable, so it must be anchored at the starting end.

use std::sync::Arc;

use super::Statelet;
use crate::error::{Error, Result};
use crate::keep_tuple::list_len;
use crate::pool::Count;
use crate::Int;

/// The `index` of `MultisetKeep`: a `slice` (whose bounds are `int`s or `None`) or a sequence of `int`s with
/// `...` (`None` here) at one end.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MultisetKeepIndex {
    Slice {
        start: Option<i64>,
        stop: Option<i64>,
        /// Whether a step was given (never supported).
        step: bool,
    },
    Seq(Vec<Option<Int>>),
    /// Anything else (neither a slice nor a sequence), by its Python type name: only rejected when evaluated.
    Invalid(Arc<str>),
}

/// `(x,) * n` for a Python int `n` (empty if `n <= 0`). Python builds these tuples in memory; the port refuses
/// sizes Python would struggle with (`list_len`).
fn repeat(x: i64, n: impl Into<Int>) -> Result<Vec<Int>> {
    Ok(vec![Int::from(x); list_len(&n.into())?])
}

/// The length of `xs[:n]` for a Python int `n` (negative counts from the end).
fn prefix_len(len: usize, n: &Int) -> usize {
    if n.is_negative() {
        len.saturating_sub((-n).clamp_usize())
    } else {
        n.clamp_usize().min(len)
    }
}

fn sum(xs: &[Int]) -> Int {
    xs.iter().sum()
}

const ASCENDING: i64 = 1;
const DESCENDING: i64 = -1;
const ANY: i64 = 0;

/// `MultisetKeep._initial_state`.
///
/// Returns `((keep_tuple, keep_more), size)`: a keep_tuple that pops from the left regardless of order, and
/// whether items are kept after running out of the keep_tuple.
pub(super) fn initial_state(
    index: &MultisetKeepIndex,
    order: i64,
    child_size: Option<Int>,
) -> Result<(Statelet, Option<Count>)> {
    let (keep_tuple, keep_more, required_order) = match index {
        MultisetKeepIndex::Slice { step: true, .. } => {
            return Err(Error::InvalidArgument("step is not supported.".into()))
        }
        &MultisetKeepIndex::Slice {
            start,
            stop,
            step: false,
        } => match (start, stop) {
            (None, None) => (Vec::new(), true, ANY),
            // [:+b] keeps from the bottom.
            (None, Some(stop)) if stop >= 0 => (repeat(1, stop)?, false, ASCENDING),
            // [:-b] drops from the top.
            (None, Some(stop)) => (repeat(0, -stop)?, true, DESCENDING),
            // [-a:] keeps from the top.
            (Some(start), None) if start < 0 => (repeat(1, -start)?, false, DESCENDING),
            // [a:] drops from the bottom.
            (Some(start), None) => (repeat(0, start)?, true, ASCENDING),
            // [a:b]
            (Some(start), Some(stop)) if start >= 0 && stop >= 0 => {
                let mut t = repeat(0, start)?;
                t.extend(repeat(1, stop - start)?);
                (t, false, ASCENDING)
            }
            // [-a:-b]
            (Some(start), Some(stop)) if start < 0 && stop < 0 => {
                let mut t = repeat(0, -stop)?;
                t.extend(repeat(1, stop - start)?);
                (t, false, DESCENDING)
            }
            (Some(start), Some(stop)) => {
                let Some(child_size) = &child_size else {
                    return Err(Error::InvalidArgument(
                        "If both start and stop are provided, they must be both negative or both \
                         non-negative, or the size must be an inferrable constant."
                            .into(),
                    ));
                };
                let mut ascending_keep = repeat(0, child_size.clone())?;
                // `[0] * child_size` was built, so a size beyond `i64` is a negative one (a negative multiplier
                // of a large size): saturating it changes neither the empty range nor the `IndexError`.
                let child_size = child_size.to_i64().unwrap_or(i64::MIN);
                let range = if start >= 0 {
                    start..child_size.saturating_add(stop)
                } else {
                    child_size.saturating_add(start)..stop
                };
                let len = ascending_keep.len() as i64;
                for i in range {
                    // `ascending_keep[i] = 1`, negative indexes counting from the end.
                    let j = if i < 0 { i + len } else { i };
                    if !(0..len).contains(&j) {
                        return Err(Error::IndexOutOfRange(
                            "list assignment index out of range".into(),
                        ));
                    }
                    ascending_keep[j as usize] = Int::ONE;
                }
                if order > 0 {
                    (ascending_keep, false, order)
                } else {
                    ascending_keep.reverse();
                    (ascending_keep, false, order)
                }
            }
        },
        MultisetKeepIndex::Seq(index) => {
            let invalid = || {
                Error::InvalidArgument(
                    "If a sequence is provided, either the first or last element (but not both) must be an \
                     Ellipsis (...)"
                        .into(),
                )
            };
            let (keep_tuple, required_order) = match (index.first(), index.last()) {
                (None, _) | (_, None) => {
                    return Err(Error::IndexOutOfRange("tuple index out of range".into()))
                }
                (Some(None), _) => (
                    index[1..].iter().rev().cloned().collect::<Vec<_>>(),
                    DESCENDING,
                ),
                (_, Some(None)) => (index[..index.len() - 1].to_vec(), ASCENDING),
                _ => return Err(invalid()),
            };
            if keep_tuple.contains(&None) {
                return Err(invalid());
            }
            (
                keep_tuple.into_iter().flatten().collect(),
                false,
                required_order,
            )
        }
        MultisetKeepIndex::Invalid(type_name) => {
            return Err(Error::WrongType(format!(
                "Invalid type <class '{type_name}'> for index."
            )))
        }
    };

    let len = keep_tuple.len() as i64;
    if required_order == order {
        let Some(child_size) = child_size else {
            return Ok((statelet(keep_tuple, keep_more), None));
        };
        let more = (&child_size - len).max(Int::ZERO);
        let mut size = sum(&keep_tuple[..prefix_len(keep_tuple.len(), &child_size)]);
        if keep_more {
            size += more;
        }
        Ok((statelet(keep_tuple, keep_more), Some(Count::Int(size))))
    } else {
        let Some(child_size) = child_size else {
            return Err(Error::UnsupportedOrder(
                "Could not find supported order for keep operator without inferrable constant size.".into(),
            ));
        };
        let more = (&child_size - len).max(Int::ZERO);
        let reversed_start = (Int::from(len) - &child_size)
            .max(Int::ZERO)
            .min(len.into());
        let reversed_start = reversed_start.to_usize().expect("at most the length");
        // `(keep_more,) * more`: bools, but they only ever get summed.
        let mut reversed_tuple = repeat(keep_more as i64, more)?;
        reversed_tuple.extend(keep_tuple[reversed_start..].iter().rev().cloned());
        let size = sum(&reversed_tuple);
        Ok((statelet(reversed_tuple, false), Some(Count::Int(size))))
    }
}

fn statelet(keep_tuple: Vec<Int>, keep_more: bool) -> Statelet {
    Statelet::Keep {
        keep_tuple: Arc::from(keep_tuple),
        keep_more,
    }
}

/// `MultisetKeep._next_state`. Negative incoming counts are treated as zero counts.
pub(super) fn next_state(state: &Statelet, child_count: &Int) -> Result<(Statelet, Count)> {
    let Statelet::Keep {
        keep_tuple,
        keep_more,
    } = state
    else {
        unreachable!("a keep node's statelet")
    };
    let child_count = child_count.max(&Int::ZERO);
    let k = prefix_len(keep_tuple.len(), child_count);
    let mut count = sum(&keep_tuple[..k]);
    if *keep_more {
        count += (child_count - Int::from(keep_tuple.len())).max(Int::ZERO);
    }
    Ok((
        Statelet::Keep {
            keep_tuple: Arc::from(&keep_tuple[k..]),
            keep_more: *keep_more,
        },
        Count::Int(count),
    ))
}
