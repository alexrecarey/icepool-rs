//! Port of icepool `function.py` (free functions).

use crate::counts::Counts;
use crate::die::Die;
use crate::error::{value_error, Result};
use crate::outcome::Outcome;
use crate::Quantity;

/// A standard die, uniformly distributed from `1` to `sides` inclusive.
///
/// Don't confuse this with `Die::new`: `Die([6])` always rolls the integer 6, `d(6)` is a d6. (Python's
/// `TypeError` for a non-`int` `sides` is the facade's job.)
pub fn d<O: Outcome + From<i64>>(sides: i64) -> Result<Die<O>> {
    if sides < 1 {
        return value_error("sides must be at least 1.");
    }
    Ok(Die::from_counts(Counts::from_sorted(
        (1..=sides).map(|i| (O::from(i), Quantity::ONE)).collect(),
    )))
}

/// Merge sets into a sorted sequence (`sorted(set.union(*(set(arg) for arg in args)))`).
///
/// An outcome keeps the spelling of the first argument that has it (`set.union` adds later equal elements to the
/// copy of the first set as no-ops). Python sorts the set in hash order, the port in first-seen order: the same
/// result whenever the outcomes are totally ordered (see NOTES.md for the NaN / partially comparable gap).
pub fn sorted_union<'a, O, I>(args: impl IntoIterator<Item = I>) -> Result<Vec<O>>
where
    O: Outcome + 'a,
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
    crate::counts::py_sorted(union, |a, b| a.py_lt(b))
}
