//! Port of icepool `population/die.py`: sampling with replacement, quantities are weights.

use std::sync::Arc;

use crate::counts::{Counts, OrderedMerge};
use crate::creation_args::{expand_args_for_die, DieArg};
use crate::error::{value_error, Error, Result};
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
    pub(crate) fn from_mapping(data: OrderedMerge<O>) -> Result<Self> {
        Self::from_dict(data.into_items())
    }

    /// `Die(dict)` for items whose outcomes are already distinct.
    pub fn from_dict(items: Vec<(O, Quantity)>) -> Result<Self> {
        let items = items.into_iter().filter(|(_, q)| !q.is_zero()).collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// The sum of all quantities.
    pub fn denominator(&self) -> Quantity {
        self.data.items().iter().map(|(_, q)| q).sum()
    }

    /// The least outcome (`IndexError` for an empty die).
    pub fn min_outcome(&self) -> Result<&O> {
        match self.items().first() {
            Some((o, _)) => Ok(o),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// Rerolls the outcomes selected by `reroll` (one flag per outcome, as `items()`).
    ///
    /// `depth` is the maximum number of times to reroll; `None` (Python's `'inf'`) rerolls an unlimited number of
    /// times. If the reroll would never terminate, the result has no outcomes. The facade computes the selection
    /// (`_select_outcomes`, or `{self.min_outcome()}` when no outcomes are given) and checks that `depth` is not
    /// negative.
    pub fn reroll(&self, reroll: &[bool], depth: Option<u32>) -> Result<Self> {
        let items = self.items().iter().zip(reroll);
        let data = match depth {
            None => items
                .filter(|(_, &r)| !r)
                .map(|((o, q), _)| (o.clone(), q.clone()))
                .collect(),
            Some(depth) => {
                let total_reroll_quantity: Quantity =
                    items.clone().filter(|(_, &r)| r).map(|((_, q), _)| q).sum();
                let denominator = self.denominator();
                let total_stop_quantity = &denominator - &total_reroll_quantity;
                if total_stop_quantity.is_zero() {
                    return Err(Error::ZeroDivision(
                        "integer division or modulo by zero".into(),
                    ));
                }
                // A stopping outcome is reached after 0..=depth rerolls: the factor sums
                // den**(depth - k) * rerolled**k over those, i.e. (den**(depth+1) - rerolled**(depth+1)) / stop.
                let rerollable_factor = total_reroll_quantity.pow(depth as usize);
                let stop_factor = (denominator.pow(depth as usize + 1)
                    - &rerollable_factor * &total_reroll_quantity)
                    / total_stop_quantity;
                items
                    .map(|((o, q), &r)| {
                        let factor = if r { &rerollable_factor } else { &stop_factor };
                        (o.clone(), factor * q)
                    })
                    .collect()
            }
        };
        Self::from_dict(data)
    }

    /// Splits this die into one containing the selected outcomes and another containing the rest.
    ///
    /// The sum of the denominators of the results is equal to the denominator of this die (zero quantities drop
    /// out of both).
    pub fn split(&self, selected: &[bool]) -> Result<(Self, Self)> {
        let (sel, rest): (Vec<_>, Vec<_>) =
            self.items().iter().zip(selected).partition(|(_, &s)| s);
        let unflag =
            |v: Vec<(&(O, Quantity), &bool)>| v.into_iter().map(|(i, _)| i.clone()).collect();
        Ok((
            Self::from_dict(unflag(sel))?,
            Self::from_dict(unflag(rest))?,
        ))
    }

    /// Truncates the outcomes of this die to the given range.
    ///
    /// The endpoints are included in the result if applicable. If one of the arguments is not provided, that side
    /// will not be truncated. This effectively rerolls outcomes outside the given range. If instead you want to
    /// replace those outcomes with the nearest endpoint, use `clip()`.
    pub fn truncate(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Result<Self> {
        let items = self.items();
        // bisect_left / bisect_right, making exactly the comparisons Python's bisect makes.
        let start = match min_outcome {
            Some(x) => bisect(items, |o| o.py_lt(x))?,
            None => 0,
        };
        let stop = match max_outcome {
            Some(x) => bisect(items, |o| Ok(!x.py_lt(o)?))?,
            None => items.len(),
        };
        // `items()[start:stop]` is empty when start > stop.
        Self::from_dict(items[start..stop.max(start)].to_vec())
    }

    /// Clips the outcomes of this die to the given values.
    ///
    /// The endpoints are included in the result if applicable. If one of the arguments is not provided, that side
    /// will not be clipped. This is not the same as rerolling outcomes beyond this range; the outcome is simply
    /// adjusted to fit within the range. This will typically cause some quantity to bunch up at the endpoint(s).
    /// If you want to reroll outcomes beyond this range, use `truncate()`.
    pub fn clip(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            let new = match (min_outcome, max_outcome) {
                (Some(min), _) if outcome.py_le(min)? => min,
                (_, Some(max)) if outcome.py_ge(max)? => max,
                _ => outcome,
            };
            data.add(new.clone(), quantity.clone());
        }
        Self::from_mapping(data)
    }
}

/// Python's `bisect` loop over sorted items: the first index whose outcome does not satisfy `go_right` (which must
/// hold for a prefix of the items). `lo` moves right while `go_right(a[mid])`.
fn bisect<O>(items: &[(O, Quantity)], go_right: impl Fn(&O) -> Result<bool>) -> Result<usize> {
    let (mut lo, mut hi) = (0, items.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if go_right(&items[mid].0)? {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}
