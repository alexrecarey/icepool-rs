//! Port of icepool `creation_args.py`: turning `Die(...)` arguments into `Counts`.

use crate::counts::{Counts, OrderedMerge};
use crate::die::Die;
use crate::error::Result;
use crate::math::weighted_lcm;
use crate::outcome::Outcome;
use crate::Quantity;

/// One element of the `outcomes` passed to `Die(...)` or `Deck(...)`, after the facade has unpacked Python's dynamic
/// arguments.
///
/// Tuples containing dice are `tupleize`d into a `Die` by the facade before they get here (the core does not know
/// about tuple outcomes). (`Again` is a later addition: it becomes a variant here.)
#[derive(Clone, Debug)]
pub enum DieArg<O> {
    /// A plain outcome: `expand_arg` makes it `{outcome: 1}`.
    Outcome(O),
    /// A die, which will be flattened into the result. The quantity assigned to it is shared among its outcomes.
    /// (Any `Population` is a mapping to `expand_arg`: a `Deck` argument comes as its cards, `Deck::population`.)
    Die(Die<O>),
    /// `icepool.Reroll` (or `Restart`): `expand_arg` makes it `{}`, so it drops itself from consideration.
    Reroll,
}

impl<O: Outcome> DieArg<O> {
    fn denominator(&self) -> Quantity {
        match self {
            DieArg::Outcome(_) => Quantity::ONE,
            DieArg::Die(d) => d.denominator(),
            DieArg::Reroll => Quantity::ZERO,
        }
    }
}

/// `expand_args_for_die`: `Counts(merge_weights_lcm(subdatas, times).items())`.
pub fn expand_args_for_die<O: Outcome>(
    args: &[DieArg<O>],
    times: &[Quantity],
) -> Result<Counts<O>> {
    Counts::new(merge_weights_lcm(args, times))
}

/// Merge for dice.
///
/// Every subdata gets total weight proportional to the corresponding element of `weights`. Outcomes are added up in
/// a dict, so equal outcomes from different arguments merge into the first one seen. A die's zero quantities
/// survive (they add 0), but an argument whose scale factor is 0 contributes nothing at all.
pub fn merge_weights_lcm<O: Outcome>(
    subdatas: &[DieArg<O>],
    weights: &[Quantity],
) -> Vec<(O, Quantity)> {
    let denominators: Vec<Quantity> = subdatas.iter().map(DieArg::denominator).collect();
    let scale_factors = weighted_lcm(&denominators, weights);
    let mut data = OrderedMerge::default();
    for (subdata, scale_factor) in subdatas.iter().zip(scale_factors) {
        if scale_factor.is_zero() {
            continue;
        }
        match subdata {
            DieArg::Outcome(o) => data.add(o.clone(), scale_factor),
            DieArg::Die(d) => {
                for (outcome, weight) in d.items() {
                    data.add(outcome.clone(), weight * &scale_factor);
                }
            }
            DieArg::Reroll => {}
        }
    }
    data.into_items()
}

/// `expand_args_for_deck`: `Counts(merge_duplicates(subdatas, times).items())`.
pub fn expand_args_for_deck<O: Outcome>(
    args: &[DieArg<O>],
    duplicates: &[Quantity],
) -> Result<Counts<O>> {
    Counts::new(merge_duplicates(args, duplicates))
}

/// Merge for decks.
///
/// Every subdata gets dups equal to the corresponding element of duplicates times its original dups. Unlike
/// `merge_weights_lcm` nothing is rescaled: a die among the arguments contributes its quantities as card counts.
/// Zero dups (of an argument, or of an outcome of a die) contribute nothing, not even the key.
pub fn merge_duplicates<O: Outcome>(
    subdatas: &[DieArg<O>],
    duplicates: &[Quantity],
) -> Vec<(O, Quantity)> {
    let mut data = OrderedMerge::default();
    for (subdata, subdup) in subdatas.iter().zip(duplicates) {
        if subdup.is_zero() {
            continue;
        }
        match subdata {
            DieArg::Outcome(o) => data.add(o.clone(), subdup.clone()),
            DieArg::Die(d) => {
                for (outcome, dup) in d.items() {
                    if dup.is_zero() {
                        continue;
                    }
                    data.add(outcome.clone(), dup * subdup);
                }
            }
            DieArg::Reroll => {}
        }
    }
    data.into_items()
}
