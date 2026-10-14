//! Port of icepool `population/keep.py`: the lowest / highest of several dice, the cases that need no pool.
//!
//! `_sum_slice` (the dispatch to these, to sums, or to a pool) lives with the facade, which owns `+`.

use crate::die::{from_cumulative, Die};
use crate::error::Result;
use crate::function::sorted_union;
use crate::outcome::Outcome;
use crate::Quantity;

/// Python's `min(xs)`: the first of the least elements, comparing `item < current`.
fn py_min<'a, O: Outcome>(xs: impl IntoIterator<Item = &'a O>) -> Result<&'a O> {
    let mut xs = xs.into_iter();
    let mut best = xs.next().expect("at least one die");
    for x in xs {
        if x.py_lt(best)? {
            best = x;
        }
    }
    Ok(best)
}

/// Python's `max(xs)`: the first of the greatest elements, comparing `item > current`.
fn py_max<'a, O: Outcome>(xs: impl IntoIterator<Item = &'a O>) -> Result<&'a O> {
    let mut xs = xs.into_iter();
    let mut best = xs.next().expect("at least one die");
    for x in xs {
        if x.py_gt(best)? {
            best = x;
        }
    }
    Ok(best)
}

/// Roll all the dice and take the lowest single one (`_lowest_single`, at least one die, none empty).
///
/// The maximum outcome is equal to the least maximum outcome among all input dice.
pub fn lowest_single<O: Outcome>(dice: &[Die<O>]) -> Result<Die<O>> {
    let maxes = dice
        .iter()
        .map(Die::max_outcome)
        .collect::<Result<Vec<_>>>()?;
    let max_outcome = py_min(maxes)?.clone();
    let dice = dice
        .iter()
        .map(|die| die.clip(None, Some(&max_outcome)))
        .collect::<Result<Vec<_>>>()?;
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut quantities_ge = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut product = Quantity::ONE;
        for die in &dice {
            product *= die.quantity_ge(outcome)?;
        }
        quantities_ge.push(product);
    }
    from_cumulative(outcomes, quantities_ge, true)
}

/// Roll all the dice and take the highest single one (`_highest_single`, at least one die, none empty).
///
/// The minimum outcome is equal to the greatest minimum outcome among all input dice.
pub fn highest_single<O: Outcome>(dice: &[Die<O>]) -> Result<Die<O>> {
    let mins = dice
        .iter()
        .map(Die::min_outcome)
        .collect::<Result<Vec<_>>>()?;
    let min_outcome = py_max(mins)?.clone();
    let dice = dice
        .iter()
        .map(|die| die.clip(Some(&min_outcome), None))
        .collect::<Result<Vec<_>>>()?;
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut quantities_le = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut product = Quantity::ONE;
        for die in &dice {
            product *= die.quantity_le(outcome)?;
        }
        quantities_le.push(product);
    }
    from_cumulative(outcomes, quantities_le, false)
}
