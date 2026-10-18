//! Port of icepool `creation_args.py`: turning `Die(...)` arguments into `Counts`.

use crate::counts::{Counts, OrderedMerge};
use crate::die::Die;
use crate::error::{Error, Result};
use crate::math::weighted_lcm;
use crate::outcome::{ErrInto, OutcomeOrder};
use crate::Quantity;

/// One element of the `outcomes` passed to `Die(...)` or `Deck(...)`, after the facade has unpacked Python's dynamic
/// arguments.
///
/// Tuples containing dice are `tupleize`d into a `Die` by the facade before they get here (the core does not know
/// about tuple outcomes). (`Again` is a later addition: it becomes a variant here.)
///
/// ```
/// use icepool_core::value::DieArg;
/// use icepool_core::{d, Die, Quantity};
/// // `Die([1, d(2), Reroll])`
/// let args = vec![DieArg::Outcome(1), DieArg::Die(d(2)), DieArg::Reroll];
/// let die = Die::new(args, vec![Quantity::ONE; 3]).unwrap();
/// assert_eq!(die, Die::from_weights([(1, 3u32), (2, 1)]));
/// ```
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

impl<O: OutcomeOrder> DieArg<O> {
    fn denominator(&self) -> Quantity {
        match self {
            DieArg::Outcome(_) => Quantity::ONE,
            DieArg::Die(d) => d.denominator(),
            DieArg::Reroll => Quantity::ZERO,
        }
    }
}

/// `expand_args_for_die`: `Counts(merge_weights_lcm(subdatas, times).items())`. Each argument gets total weight
/// proportional to its element of `times`, the dice scaled to a common denominator first.
///
/// ```
/// use icepool_core::value::{expand_args_for_die, DieArg};
/// use icepool_core::{d, Quantity};
/// let q = Quantity::from;
/// // `Die([d(2), 5], times=[1, 2])`: d(2) has denominator 2, so 5 weighs 2 * 2.
/// let counts = expand_args_for_die(&[DieArg::Die(d(2)), DieArg::Outcome(5)], &[q(1u8), q(2u8)]).unwrap();
/// assert_eq!(counts.items(), &[(1, q(1u8)), (2, q(1u8)), (5, q(4u8))]);
/// ```
pub fn expand_args_for_die<O: OutcomeOrder>(
    args: &[DieArg<O>],
    times: &[Quantity],
) -> Result<Counts<O>> {
    sorted_counts(merge_weights_lcm(args, times)?)
}

/// `Counts(items)` as `Die` and `Deck` make it. Python's `Counts` sorts the items inside a `try` and replaces the
/// `TypeError` of incomparable keys with its own (a key that cannot be hashed fails after the sort, as it is).
fn sorted_counts<O: OutcomeOrder>(items: Vec<(O, Quantity)>) -> Result<Counts<O>> {
    Counts::new(items).err_into().map_err(|e| match e {
        Error::Incomparable { .. } => Error::WrongType(
            "Items do not appear to be sortable.\n\
             Tip: Sequences containing dice or decks are not sortable.\n\
             Use tupleize() or vectorize() to transform\n\
             a sequence of dice into a die with sequence outcomes\n\
             according to the Cartesian product."
                .into(),
        ),
        e => e,
    })
}

/// Merge for dice.
///
/// Every subdata gets total weight proportional to the corresponding element of `weights`. Outcomes are added up in
/// a dict, so equal outcomes from different arguments merge into the first one seen. A die's zero quantities
/// survive (they add 0), but an argument whose scale factor is 0 contributes nothing at all.
pub fn merge_weights_lcm<O: OutcomeOrder>(
    subdatas: &[DieArg<O>],
    weights: &[Quantity],
) -> Result<Vec<(O, Quantity)>> {
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
                // A mixture of many big dice is a long loop.
                crate::interrupt::poll()?;
                for (outcome, weight) in d.items() {
                    data.add(outcome.clone(), weight * &scale_factor);
                }
            }
            DieArg::Reroll => {}
        }
    }
    Ok(data.into_items())
}

/// `expand_args_for_deck`: `Counts(merge_duplicates(subdatas, times).items())`.
pub fn expand_args_for_deck<O: OutcomeOrder>(
    args: &[DieArg<O>],
    duplicates: &[Quantity],
) -> Result<Counts<O>> {
    sorted_counts(merge_duplicates(args, duplicates)?)
}

/// Merge for decks.
///
/// Every subdata gets dups equal to the corresponding element of duplicates times its original dups. Unlike
/// `merge_weights_lcm` nothing is rescaled: a die among the arguments contributes its quantities as card counts.
/// Zero dups (of an argument, or of an outcome of a die) contribute nothing, not even the key.
pub fn merge_duplicates<O: OutcomeOrder>(
    subdatas: &[DieArg<O>],
    duplicates: &[Quantity],
) -> Result<Vec<(O, Quantity)>> {
    let mut data = OrderedMerge::default();
    for (subdata, subdup) in subdatas.iter().zip(duplicates) {
        if subdup.is_zero() {
            continue;
        }
        match subdata {
            DieArg::Outcome(o) => data.add(o.clone(), subdup.clone()),
            DieArg::Die(d) => {
                // A mixture of many big dice is a long loop.
                crate::interrupt::poll()?;
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
    Ok(data.into_items())
}
