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
