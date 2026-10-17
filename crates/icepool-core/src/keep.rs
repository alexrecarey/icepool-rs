//! Port of icepool `population/keep.py`: the lowest / highest of several dice, the cases that need no pool.
//!
//! `_sum_slice` (the dispatch to these, to sums, or to a pool) lives with the facade, which owns `+`.

use crate::die::{from_cumulative, Die};
use crate::error::Result;
use crate::function::sorted_union;
use crate::outcome::{self, ErrInto, OutcomeOrder};
use crate::Quantity;

/// Roll all the dice and take the lowest single one (`_lowest_single`, at least one die, none empty).
///
/// The maximum outcome is equal to the least maximum outcome among all input dice.
pub fn lowest_single<O: OutcomeOrder>(dice: &[Die<O>]) -> Result<Die<O>> {
    let maxes = dice
        .iter()
        .map(Die::max_outcome)
        .collect::<Result<Vec<_>>>()?;
    let max_outcome = outcome::min(maxes)?.clone();
    let dice = dice
        .iter()
        .map(|die| die.try_clip(None, Some(&max_outcome)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .err_into()?;
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut quantities_ge = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut product = Quantity::ONE;
        for die in &dice {
            product *= die.try_quantity_ge(outcome).err_into()?;
        }
        quantities_ge.push(product);
    }
    from_cumulative(outcomes, quantities_ge, true).err_into()
}

/// Roll all the dice and take the highest single one (`_highest_single`, at least one die, none empty).
///
/// The minimum outcome is equal to the greatest minimum outcome among all input dice.
pub fn highest_single<O: OutcomeOrder>(dice: &[Die<O>]) -> Result<Die<O>> {
    let mins = dice
        .iter()
        .map(Die::min_outcome)
        .collect::<Result<Vec<_>>>()?;
    let min_outcome = outcome::max(mins)?.clone();
    let dice = dice
        .iter()
        .map(|die| die.try_clip(Some(&min_outcome), None))
        .collect::<std::result::Result<Vec<_>, _>>()
        .err_into()?;
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut quantities_le = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut product = Quantity::ONE;
        for die in &dice {
            product *= die.try_quantity_le(outcome).err_into()?;
        }
        quantities_le.push(product);
    }
    from_cumulative(outcomes, quantities_le, false).err_into()
}
