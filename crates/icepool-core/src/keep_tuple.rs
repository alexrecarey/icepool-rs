//! Port of the keep-tuple helpers of icepool `generator/keep.py` and the slice helpers of `population/keep.py`.
//!
//! A keep tuple has one count per die of a pool, in ascending order of the rolls: how many times that sorted
//! position is counted (0 drops it; it may be more than 1 or negative). Python ints are unbounded; the port keeps
//! counts in `i64` and says `Unsupported` where they would overflow (no program comes close).

use crate::error::{Error, Result};

/// A Python slice `start:stop` (step must be `None`: see `KeepIndex::Slice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub start: Option<i64>,
    pub stop: Option<i64>,
}

impl Slice {
    pub fn new(start: Option<i64>, stop: Option<i64>) -> Self {
        Slice { start, stop }
    }

    /// `canonical_slice`: produces `(start, stop)` with `0 <= start <= stop <= length` (when `length >= 0`), which
    /// is also how a Python list resolves a slice with a positive step. `length` may be negative (`Die.highest`
    /// with negative rolls asks): then the bounds come out as Python computes them.
    pub fn canonical(self, length: i64) -> (i64, i64) {
        let start = match self.start {
            None => 0,
            Some(s) if s < 0 => 0.max(length.saturating_add(s)),
            Some(s) => s.min(length),
        };
        let mut stop = match self.stop {
            None => length,
            Some(s) if s < 0 => 0.max(length.saturating_add(s)),
            Some(s) => s.min(length),
        };
        if start > stop {
            stop = start;
        }
        (start, stop)
    }

    /// The positions of a list of `len` elements that `list[start:stop]` selects.
    fn range(self, len: usize) -> std::ops::Range<usize> {
        let (start, stop) = self.canonical(len as i64);
        start as usize..stop as usize
    }
}

/// The argument of `KeepGenerator.keep` / `pool[index]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepIndex {
    /// An `int`: only the roll at that index counts, and the result is a `Die` rather than a generator.
    Int(i64),
    /// A `slice`: the selected dice are counted once each. `step` is whether a step was given (never supported).
    Slice { slice: Slice, step: bool },
    /// A sequence of one count per die, `None` standing for at most one `...` (`Ellipsis`).
    Seq(Vec<Option<i64>>),
}

fn overflow<T>() -> Result<T> {
    Err(Error::Unsupported("keep counts beyond 64 bits".into()))
}

/// `sum(counts)`.
pub fn sum_counts(counts: &[i64]) -> Result<i64> {
    counts
        .iter()
        .try_fold(0i64, |a, &b| a.checked_add(b))
        .map_or_else(overflow, Ok)
}

/// Expresses `index` as a keep_tuple.
///
/// Raises `IndexError` if more than one `Ellipsis` is used, if a sequence without `...` does not have `pool_size`
/// elements, for an `int` out of range, or for a slice with a step.
pub fn make_keep_tuple(pool_size: usize, index: &KeepIndex) -> Result<Vec<i64>> {
    let index_error = |msg: &str| Err(Error::Index(msg.into()));
    match index {
        KeepIndex::Int(i) => {
            let mut result = vec![0; pool_size];
            // `result[index] = 1`, negative indices counting from the end.
            let n = pool_size as i64;
            let pos = if *i < 0 { i.saturating_add(n) } else { *i };
            if !(0..n).contains(&pos) {
                return index_error("list assignment index out of range");
            }
            result[pos as usize] = 1;
            Ok(result)
        }
        KeepIndex::Slice { slice, step } => {
            if *step {
                return index_error("step is not supported for pool subscripting");
            }
            let mut result = vec![0; pool_size];
            for x in &mut result[slice.range(pool_size)] {
                *x = 1;
            }
            Ok(result)
        }
        KeepIndex::Seq(index) => {
            let mut split = None;
            for (i, x) in index.iter().enumerate() {
                if x.is_none() {
                    if split.is_none() {
                        split = Some(i);
                    } else {
                        return index_error(
                            "Cannot use more than one Ellipsis (...) for keep_tuple.",
                        );
                    }
                }
            }
            // The following code is designed to replace Ellipsis with actual zeros.
            let ints = |xs: &[Option<i64>]| -> Vec<i64> {
                xs.iter()
                    .map(|x| x.expect("only the split is ..."))
                    .collect()
            };
            let Some(split) = split else {
                if index.len() != pool_size {
                    return index_error("Length of index does not match pool size");
                }
                return Ok(ints(index));
            };
            let len = index.len();
            let extra_dice = pool_size as i64 - len as i64 + 1;
            let zeros = vec![0; extra_dice.max(0) as usize];
            if split == 0 {
                // Ellipsis on left.
                let index = ints(&index[1..]);
                if extra_dice < 0 {
                    Ok(index[(-extra_dice) as usize..].to_vec())
                } else {
                    Ok([zeros, index].concat())
                }
            } else if split == len - 1 {
                // Ellipsis on right.
                let index = ints(&index[..len - 1]);
                if extra_dice < 0 {
                    Ok(index[..(index.len() as i64 + extra_dice) as usize].to_vec())
                } else {
                    Ok([index, zeros].concat())
                }
            } else if extra_dice < 0 {
                // Ellipsis in center, with more counts than dice: the sum of the two one-sided `...`.
                let mut result = vec![0i64; pool_size];
                for i in 0..split.min(pool_size) {
                    result[i] = result[i]
                        .checked_add(index[i].expect("left of the split"))
                        .map_or_else(overflow, Ok)?;
                }
                // `for i in range(-1, max(reverse_split - 1, -pool_size - 1), -1)`, indexing from the end. With
                // enough dice this reaches the `...` itself: Python then raises adding it to an int.
                let reverse_split = split as i64 - len as i64;
                let stop = (reverse_split - 1).max(-(pool_size as i64) - 1);
                let mut i = -1;
                while i > stop {
                    let Some(x) = index[(len as i64 + i) as usize] else {
                        return Err(Error::Type(
                            "unsupported operand type(s) for +=: 'int' and 'ellipsis'".into(),
                        ));
                    };
                    let r = &mut result[(pool_size as i64 + i) as usize];
                    *r = r.checked_add(x).map_or_else(overflow, Ok)?;
                    i -= 1;
                }
                Ok(result)
            } else {
                // Ellipsis in center.
                Ok([ints(&index[..split]), zeros, ints(&index[split + 1..])].concat())
            }
        }
    }
}

/// Pops elements off the front of the keep tuple, returning the remaining tuple and the sum of the elements.
pub fn pop_min_from_keep_tuple(keep_tuple: &[i64], count: usize) -> Result<(Vec<i64>, i64)> {
    let k = count.min(keep_tuple.len());
    Ok((keep_tuple[k..].to_vec(), sum_counts(&keep_tuple[..k])?))
}

/// Pops elements off the back of the keep tuple, returning the remaining tuple and the sum of the elements.
pub fn pop_max_from_keep_tuple(keep_tuple: &[i64], count: usize) -> Result<(Vec<i64>, i64)> {
    // `keep_tuple[:-0]` would be empty, hence the special case.
    if count == 0 {
        return Ok((keep_tuple.to_vec(), 0));
    }
    let k = keep_tuple.len().saturating_sub(count);
    Ok((keep_tuple[..k].to_vec(), sum_counts(&keep_tuple[k..])?))
}

/// Applies a keep tuple on top of a base keep tuple (whose counts are not negative).
///
/// Each base position counted `x` times takes the next `x` counts of `apply` and counts their sum.
pub fn compose_keep_tuples(base: &[i64], apply: &[i64]) -> Result<Vec<i64>> {
    let mut apply = apply;
    let mut result = Vec::with_capacity(base.len());
    for &x in base {
        let x = usize::try_from(x).expect("compose_keep_tuples needs non-negative base counts");
        let k = x.min(apply.len());
        result.push(sum_counts(&apply[..k])?);
        apply = &apply[k..];
    }
    Ok(result)
}

/// `KeepGenerator.keep` on a keep tuple: the keep tuple for `index` relative to the elements counted so far,
/// composed on top of `keep_tuple`. If any of the current counts are negative, Python falls back to the
/// less-capable `MultisetExpression` version and the result is not a keep generator: not ported yet.
pub fn keep(keep_tuple: &[i64], index: &KeepIndex) -> Result<Vec<i64>> {
    if keep_tuple.iter().any(|&x| x < 0) {
        return Err(Error::Unsupported(
            "keep on a generator with negative counts (a MultisetKeep expression)".into(),
        ));
    }
    let keep_size = usize::try_from(sum_counts(keep_tuple)?).expect("no negative counts");
    let relative_keep_tuple = make_keep_tuple(keep_size, index)?;
    compose_keep_tuples(keep_tuple, &relative_keep_tuple)
}

/// `KeepGenerator.multiply_counts`: every count multiplied by `constant` (`-g` is `-1 * g`).
pub fn multiply_counts(keep_tuple: &[i64], constant: i64) -> Result<Vec<i64>> {
    keep_tuple
        .iter()
        .map(|n| n.checked_mul(constant))
        .collect::<Option<Vec<_>>>()
        .map_or_else(overflow, Ok)
}

fn check_non_negative(name: &str, x: Option<i64>) -> Result<()> {
    match x {
        Some(x) if x < 0 => Err(Error::Value(format!("{name}={x} cannot be negative."))),
        _ => Ok(()),
    }
}

/// Converts keep and drop lowest into a slice.
pub fn lowest_slice(keep: Option<i64>, drop: Option<i64>) -> Result<Slice> {
    check_non_negative("keep", keep)?;
    check_non_negative("drop", drop)?;
    Ok(match drop {
        None => Slice::new(Some(0), Some(keep.unwrap_or(1))),
        Some(drop) => Slice::new(Some(drop), keep.map(|keep| drop.saturating_add(keep))),
    })
}

/// Converts keep and drop highest into a slice.
pub fn highest_slice(keep: Option<i64>, drop: Option<i64>) -> Result<Slice> {
    check_non_negative("keep", keep)?;
    check_non_negative("drop", drop)?;
    Ok(match drop {
        None => match keep.unwrap_or(1) {
            keep if keep > 0 => Slice::new(Some(-keep), None),
            _ => Slice::new(Some(0), Some(0)),
        },
        Some(drop) => {
            let stop = if drop == 0 { None } else { Some(-drop) };
            match keep {
                None => Slice::new(None, stop),
                // Saturating is exact here: any start past -length means the same as -length.
                Some(keep) if keep > 0 => {
                    Slice::new(Some(keep.saturating_add(drop).saturating_neg()), stop)
                }
                Some(_) => Slice::new(Some(0), Some(0)),
            }
        }
    })
}

/// The `tie` argument of `middle`: what to do if `keep` is odd but the keep size is even, or vice versa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// The default: raises `IndexError`.
    Error,
    /// The higher of the two possible elements is taken.
    High,
    /// The lower of the two possible elements is taken.
    Low,
    /// Anything else: `ValueError`, but only once the tiebreaker is consulted.
    Invalid,
}

/// `KeepGenerator.middle` up to the slice it takes: keep `keep` of the middle of `keep_size` elements.
///
/// When `keep` exceeds `keep_size` with the same parity, `start` is negative and Python's slice then counts it from
/// the end (so not everything is kept, despite the docstring). Copied as is.
pub fn middle_slice(keep_size: i64, keep: i64, tie: Tie) -> Result<Slice> {
    if keep < 0 {
        return Err(Error::Value(format!("keep={keep} cannot be negative.")));
    }
    let start = if keep.rem_euclid(2) == keep_size.rem_euclid(2) {
        // The "good" case.
        (keep_size - keep).div_euclid(2)
    } else {
        // Need to consult the tiebreaker.
        match tie {
            Tie::Error => {
                return Err(Error::Index(format!(
                    "The middle {keep} of {keep_size} elements is ambiguous."
                )))
            }
            Tie::High => (keep_size + 1 - keep).div_euclid(2),
            Tie::Low => (keep_size - 1 - keep).div_euclid(2),
            Tie::Invalid => {
                return Err(Error::Value(
                    "Invalid value for tie. Expected 'error', 'low', or 'high'.".into(),
                ))
            }
        }
    };
    Ok(Slice::new(Some(start), Some(start + keep)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(xs: &[Option<i64>]) -> KeepIndex {
        KeepIndex::Seq(xs.to_vec())
    }

    #[test]
    fn ellipsis_forms_follow_the_docstring() {
        let (o, e) = (Some, None);
        // `g[1, ..., 1]` on five dice acts as `g[1, 0, 0, 0, 1]`; on two dice as `g[1, 1]`.
        assert_eq!(
            make_keep_tuple(5, &seq(&[o(1), e, o(1)])).unwrap(),
            [1, 0, 0, 0, 1]
        );
        assert_eq!(make_keep_tuple(2, &seq(&[o(1), e, o(1)])).unwrap(), [1, 1]);
        // `g[..., 1, 2, 3]` on two dice acts as `g[2, 3]`.
        assert_eq!(
            make_keep_tuple(2, &seq(&[e, o(1), o(2), o(3)])).unwrap(),
            [2, 3]
        );
        // `g[-1, ..., 1]` on a single die: the -1 and 1 cancel.
        assert_eq!(make_keep_tuple(1, &seq(&[o(-1), e, o(1)])).unwrap(), [0]);
        assert!(make_keep_tuple(2, &seq(&[e, o(1), e])).is_err());
        // Center `...`, too many counts, enough dice to reach the `...` itself: Python adds it to an int.
        assert!(matches!(
            make_keep_tuple(3, &seq(&[o(1), o(1), o(1), e, o(1)])),
            Err(Error::Type(_))
        ));
    }

    #[test]
    fn compose_and_pop() {
        // `[:2][-1]` is the second-lowest.
        let a = make_keep_tuple(
            4,
            &KeepIndex::Slice {
                slice: Slice::new(None, Some(2)),
                step: false,
            },
        )
        .unwrap();
        let b = make_keep_tuple(2, &KeepIndex::Int(-1)).unwrap();
        assert_eq!(compose_keep_tuples(&a, &b).unwrap(), [0, 1, 0, 0]);
        assert_eq!(pop_max_from_keep_tuple(&[1, 2, 3], 5).unwrap(), (vec![], 6));
        assert_eq!(
            pop_max_from_keep_tuple(&[1, 2, 3], 0).unwrap(),
            (vec![1, 2, 3], 0)
        );
        assert_eq!(
            pop_min_from_keep_tuple(&[1, 2, 3], 2).unwrap(),
            (vec![3], 3)
        );
    }
}
