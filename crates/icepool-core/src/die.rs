//! Port of icepool `population/die.py`: sampling with replacement, quantities are weights.

use std::sync::Arc;

use crate::counts::Counts;
use crate::creation_args::{expand_args_for_die, DieArg};
use crate::error::{value_error, Result};
use crate::outcome::Outcome;
use crate::Quantity;

/// Dice are immutable; methods return new dice. Cloning is cheap (the items are shared).
///
/// It's also possible to have "empty" dice with no outcomes at all, though these have little use other than being
/// sentinel values.
///
/// Equality and hashing are structural, as icepool's `Die.equals` / `hash_key`: the same outcomes (by `==`) with
/// the same quantities. A 2:2 coin is not equal to a 1:1 coin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Die<O> {
    data: Arc<Counts<O>>,
}

impl<O: Outcome> Die<O> {
    /// `Die(outcomes, times)` after `itemize`: one quantity multiplier per element of `outcomes`.
    ///
    /// Outcomes with zero total weight are omitted; dice among the outcomes are mixed in, scaled by
    /// `weighted_lcm` so that the overall denominator is minimised while preserving the relative weighting.
    pub fn new(outcomes: Vec<DieArg<O>>, times: Vec<Quantity>) -> Result<Self> {
        if outcomes.len() != times.len() {
            return value_error(format!(
                "The number of times ({}) must equal the number of keys ({}).",
                times.len(),
                outcomes.len()
            ));
        }
        // A single die with weight 1 is returned as-is (zero quantities and all).
        if let ([DieArg::Die(die)], [t]) = (outcomes.as_slice(), times.as_slice()) {
            if t.is_one() {
                return Ok(die.clone());
            }
        }
        Ok(Self::from_counts(expand_args_for_die(&outcomes, &times)?))
    }

    /// `Die._new_raw`: wraps already-processed data.
    pub fn from_counts(data: Counts<O>) -> Self {
        Die {
            data: Arc::new(data),
        }
    }

    /// Every `(outcome, quantity)` in sorted order, zero quantities included.
    pub fn items(&self) -> &[(O, Quantity)] {
        self.data.items()
    }

    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = &O> {
        self.data.items().iter().map(|(o, _)| o)
    }

    /// The number of outcomes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The sum of all quantities.
    pub fn denominator(&self) -> Quantity {
        self.data.items().iter().map(|(_, q)| q).sum()
    }
}
