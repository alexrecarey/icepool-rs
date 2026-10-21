//! Port of icepool `function.py` (free functions). Those only `Value` outcomes have are in `value/function.rs`.

use crate::counts::Counts;
use crate::die::Die;
use crate::error::{invalid_argument, Result};
use crate::memory::try_collect;
use crate::outcome::{ErrInto, Outcome, OutcomeOrder};
use crate::Quantity;

/// A standard die, uniformly distributed from `1` to `sides` inclusive.
///
/// Don't confuse this with [`Die::single`]: `Die::single(6)` always rolls the integer 6, `d(6)` is a d6.
///
/// # Panics
///
/// If `sides` is less than 1 (Python raises `ValueError: sides must be at least 1.`), or if no machine could
/// hold the die. [`try_d`] is the fallible form.
///
/// ```
/// use icepool_core::{d, Die};
/// let d6: Die<i64> = d(6);
/// assert_eq!(d6.outcomes().copied().collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
/// assert_eq!(d6.denominator().to_string(), "6");
/// ```
pub fn d(sides: i64) -> Die<i64> {
    try_d(sides).unwrap_or_else(|e| panic!("{e}"))
}

/// A die uniformly distributed from `0` to `sides - 1` inclusive. Equal to `d(sides) - 1`.
///
/// # Panics
///
/// As [`d`]. [`try_z`] is the fallible form.
///
/// ```
/// use icepool_core::{d, z};
/// assert_eq!(z(10), d(10) - 1);
/// ```
pub fn z(sides: i64) -> Die<i64> {
    try_z(sides).unwrap_or_else(|e| panic!("{e}"))
}

/// [`d`] for any outcome type made from an `i64` (the bindings' `Value`), as a `Result`: `ValueError` if `sides`
/// is less than 1. (Python's `TypeError` for a non-`int` `sides` is the facade's job.)
///
/// ```
/// use icepool_core::{try_d, Die, Value};
/// let d6: Die<Value> = try_d(6).unwrap();
/// assert_eq!(d6.len(), 6);
/// assert_eq!(try_d::<i64>(0).unwrap_err().to_string(), "sides must be at least 1.");
/// ```
pub fn try_d<O: OutcomeOrder + From<i64>>(sides: i64) -> Result<Die<O>> {
    if sides < 1 {
        return invalid_argument("sides must be at least 1.");
    }
    // `range(1, sides + 1)` as a list: Python's `MemoryError` where no machine holds it (`memory`).
    let items = try_collect(
        Some(sides),
        (1..=sides).map(|i| (O::from(i), Quantity::ONE)),
        "d(sides)",
    )?;
    Ok(Die::from_counts(Counts::from_sorted(items)))
}

/// Merge sets into a sorted sequence (`sorted(set.union(*(set(arg) for arg in args)))`).
///
/// An outcome keeps the spelling of the first argument that has it (`set.union` adds later equal elements to the
/// copy of the first set as no-ops). Python sorts the set in hash order, the port in first-seen order: the same
/// result whenever the outcomes are totally ordered (see docs/NOTES.md for the NaN / partially comparable gap).
///
/// ```
/// use icepool_core::value::sorted_union;
/// let union = sorted_union([&[3, 1][..], &[2, 3][..]]).unwrap();
/// assert_eq!(union, [1, 2, 3]);
/// ```
pub fn sorted_union<'a, O, I>(args: impl IntoIterator<Item = I>) -> Result<Vec<O>>
where
    O: OutcomeOrder + 'a,
    I: IntoIterator<Item = &'a O>,
{
    let mut seen = rustc_hash::FxHashSet::default();
    let mut union = Vec::new();
    for arg in args {
        for outcome in arg {
            if seen.insert(outcome.clone()) {
                union.push(outcome.clone());
            }
        }
    }
    crate::outcome::sorted(union)
}

/// [`z`] for any outcome type made from an `i64`, as a `Result` (see [`try_d`]).
///
/// ```
/// use icepool_core::{try_z, Die};
/// let z6: Die<i64> = try_z(6).unwrap();
/// assert_eq!(z6.outcomes().next(), Some(&0));
/// ```
pub fn try_z<O: OutcomeOrder + From<i64>>(sides: i64) -> Result<Die<O>> {
    if sides < 1 {
        return invalid_argument("sides must be at least 1.");
    }
    let items = try_collect(
        Some(sides),
        (0..sides).map(|i| (O::from(i), Quantity::ONE)),
        "z(sides)",
    )?;
    Ok(Die::from_counts(Counts::from_sorted(items)))
}

/// Selects the highest chance of rolling >= each outcome among the dice (Python's `icepool.pointwise_max`).
///
/// Specifically, for each outcome, the chance of the result rolling >= to that outcome is the same as the highest
/// chance of rolling >= that outcome among the dice. Equivalently, any quantile in the result is the highest of
/// that quantile among the dice. This is useful for selecting from several possible moves where you are trying to
/// get >= a threshold that is known but could change depending on the situation.
///
/// The denominators are harmonized first (Python's `harmonize_denominators`), so the result's denominator is the
/// least common multiple of theirs.
///
/// # Panics
///
/// Only if the result could not be held in memory.
///
/// ```
/// use icepool_core::{d, pointwise_max, Die};
/// // A d8, or a d4 + 2: the d4 + 2 is likelier to roll at least 3 to 6, the d8 at least 7 or 8.
/// let best = pointwise_max(&[d(8), d(4) + 2]);
/// assert_eq!(best, Die::from_weights([(3, 2u32), (4, 2), (5, 1), (6, 1), (7, 1), (8, 1)]));
/// assert_eq!(best.denominator().to_string(), "8");
/// ```
pub fn pointwise_max<O: Outcome>(dice: &[Die<O>]) -> Die<O> {
    pointwise(&harmonize_denominators(dice), true).unwrap_or_else(|e| panic!("{e}"))
}

/// Selects the lowest chance of rolling >= each outcome among the dice (Python's `icepool.pointwise_min`).
///
/// Specifically, for each outcome, the chance of the result rolling >= to that outcome is the same as the lowest
/// chance of rolling >= that outcome among the dice. Equivalently, any quantile in the result is the lowest of
/// that quantile among the dice. This is useful for selecting from several possible moves where you are trying to
/// get >= a threshold that is known but could change depending on the situation.
///
/// # Panics
///
/// Only if the result could not be held in memory.
///
/// ```
/// use icepool_core::{d, pointwise_min, Die};
/// let worst = pointwise_min(&[d(8), d(4) + 2]);
/// assert_eq!(worst, Die::from_weights([(1, 1u32), (2, 1), (3, 1), (4, 1), (5, 2), (6, 2)]));
/// ```
pub fn pointwise_min<O: Outcome>(dice: &[Die<O>]) -> Die<O> {
    pointwise(&harmonize_denominators(dice), false).unwrap_or_else(|e| panic!("{e}"))
}

/// Python's `harmonize_denominators(dice)` without weights: each die's quantities scaled so that every
/// denominator is the least common multiple of the (positive) denominators. math.py `weighted_lcm` gives a die
/// of denominator 0 the scale factor 0, so it becomes the empty die (`Die` drops zero quantities).
fn harmonize_denominators<O: OutcomeOrder>(dice: &[Die<O>]) -> Vec<Die<O>> {
    // `math.lcm()` of nothing is 1.
    let lcm = dice
        .iter()
        .map(Die::denominator)
        .filter(|d| !d.is_zero())
        .fold(Quantity::ONE, |lcm, d| crate::math::lcm(&lcm, &d));
    dice.iter()
        .map(|die| {
            let denominator = die.denominator();
            let items = if denominator.is_zero() {
                Vec::new()
            } else {
                let scale = &lcm / denominator;
                die.items().iter().map(|(o, q)| (o.clone(), q * &scale)).collect()
            };
            Die::from_counts(Counts::from_sorted(items))
        })
        .collect()
}

/// `pointwise_max` (`max` true) / `pointwise_min` once the dice are harmonized (`harmonize_denominators`).
///
/// pointwise_max selects the highest chance of rolling >= each outcome among the arguments: for each outcome, the
/// chance of the result rolling >= that outcome is the same as the highest chance of rolling >= that outcome
/// among the arguments (equivalently, any quantile in the result is the highest of that quantile among the
/// arguments). So its cumulative quantity `<=` each outcome is the *least* of the dice's; pointwise_min takes the
/// greatest. Each die's cumulative quantities never decrease, so neither do their min / max.
///
/// ```
/// use icepool_core::value::pointwise;
/// use icepool_core::{d, Die};
/// // d4 and d6 harmonized to the denominator 12 first.
/// let d4 = Die::from_weights((1..=4).map(|i| (i, 3u32)));
/// let d6 = Die::from_weights((1..=6).map(|i| (i, 2u32)));
/// assert_eq!(pointwise(&[d4.clone(), d6.clone()], true).unwrap(), d6);
/// assert_eq!(pointwise(&[d4.clone(), d6], false).unwrap(), d4);
/// # let _ = d(1);
/// ```
pub fn pointwise<O: OutcomeOrder>(dice: &[Die<O>], max: bool) -> Result<Die<O>> {
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut cumulative = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut best: Option<Quantity> = None;
        for die in dice {
            let q = die.try_quantity_le(outcome).err_into()?;
            // `min(...)` / `max(...)` of ints: which of equal ones is kept does not matter.
            best = Some(match best {
                Some(b) if (max && q < b) || (!max && q > b) => q,
                Some(b) => b,
                None => q,
            });
        }
        cumulative.push(best.expect("an outcome comes from some die"));
    }
    crate::die::from_cumulative(outcomes, cumulative, false).err_into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Value};

    #[test]
    fn pointwise_harmonizes_as_python() {
        // Python's answers for pointwise_max / pointwise_min of these dice.
        let empty = Die::<i64>::from_iter([]);
        assert_eq!(pointwise_max(&[d(6), empty.clone()]), empty);
        assert_eq!(pointwise_min(&[d(6), empty]), d(6));
        let dice = [d(2), d(3) - 1, d(4) * 2];
        assert_eq!(pointwise_max(&dice), Die::from_weights([(2, 3u32), (4, 3), (6, 3), (8, 3)]));
        assert_eq!(pointwise_min(&dice), Die::from_weights([(0, 4u32), (1, 4), (2, 4)]));
        assert_eq!(pointwise_max(&dice).denominator(), Quantity::from(12u8));
    }

    #[test]
    fn huge_standard_dice_are_memory_errors() {
        // Fixtures only see that both sides raise: Python's type here is MemoryError, not an abort or a panic.
        for sides in [i64::MAX, 1 << 40] {
            assert!(
                matches!(try_d::<Value>(sides), Err(Error::TooLarge(_))),
                "{sides}"
            );
            assert!(
                matches!(try_z::<Value>(sides), Err(Error::TooLarge(_))),
                "{sides}"
            );
        }
        assert_eq!(try_d::<Value>(3).unwrap().len(), 3);
    }
}
