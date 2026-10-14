//! Port of icepool `creation_args.py`: turning `Die(...)` arguments into `Counts`.

use crate::counts::{Counts, OrderedMerge};
use crate::die::Die;
use crate::error::Result;
use crate::math::weighted_lcm;
use crate::outcome::Outcome;
use crate::Quantity;

/// One element of the `outcomes` passed to `Die(...)`, after the facade has unpacked Python's dynamic arguments.
///
/// (icepool's `Reroll`, `Again` and tuples containing dice are later additions: they become variants here.)
#[derive(Clone, Debug)]
pub enum DieArg<O> {
    /// A plain outcome: `expand_arg` makes it `{outcome: 1}`.
    Outcome(O),
    /// A die, which will be flattened into the result. The quantity assigned to it is shared among its outcomes.
    Die(Die<O>),
}

impl<O: Outcome> DieArg<O> {
    fn denominator(&self) -> Quantity {
        match self {
            DieArg::Outcome(_) => Quantity::ONE,
            DieArg::Die(d) => d.denominator(),
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
        }
    }
    data.into_items()
}
