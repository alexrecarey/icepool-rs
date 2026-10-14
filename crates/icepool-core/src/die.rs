//! Port of icepool `population/die.py`: sampling with replacement, quantities are weights.

use std::sync::Arc;

use crate::counts::{Counts, OrderedMerge};
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

    /// Performs the unary operation on the outcomes (`Population._unary_operator`).
    ///
    /// This is used for the standard unary operators `-, +, abs, ~, round, trunc, floor, ceil` as well as the
    /// additional methods `zero, bool`. Outcomes that map to the same result add up their quantities.
    pub fn unary_operator(&self, op: impl Fn(&O) -> Result<O>) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            data.add(op(outcome)?, quantity.clone());
        }
        Self::from_mapping(data)
    }

    /// Performs the operation on pairs of outcomes (`Die.binary_operator`).
    ///
    /// By the time this is called, the other operand has already been converted to a `Die`. This is used for the
    /// standard binary operators `+, -, *, /, //, %, **, <<, >>, &, |, ^` and the standard binary comparators
    /// `<, <=, >=, >, ==, !=, cmp`. The `@` operator does NOT use this method directly.
    ///
    /// Every pair is visited (zero quantities included, so an `op` that raises on one raises here), `self` in the
    /// outer loop, as `itertools.product(self.items(), other.items())`.
    pub fn binary_operator(
        &self,
        other: &Die<O>,
        op: impl Fn(&O, &O) -> Result<O>,
    ) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome_self, quantity_self) in self.items() {
            for (outcome_other, quantity_other) in other.items() {
                data.add(
                    op(outcome_self, outcome_other)?,
                    quantity_self * quantity_other,
                );
            }
        }
        Self::from_mapping(data)
    }

    /// `Die(data)` for a dict of outcome -> quantity (`self._new_type(data)` in icepool). `itemize` and
    /// `merge_weights_lcm` see denominator 1 for every outcome, so `weighted_lcm` passes the quantities through
    /// unchanged except that zero ones drop out; then `Counts` sorts what is left.
    fn from_mapping(data: OrderedMerge<O>) -> Result<Self> {
        let items = data
            .into_items()
            .into_iter()
            .filter(|(_, q)| !q.is_zero())
            .collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// The sum of all quantities.
    pub fn denominator(&self) -> Quantity {
        self.data.items().iter().map(|(_, q)| q).sum()
    }
}
