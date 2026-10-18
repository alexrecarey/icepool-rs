//! Port of icepool `collection/counts.py`: an immutable map with sorted keys and integer values.

use rustc_hash::FxHashMap;

use crate::outcome::{self, OutcomeOrder};
use crate::Quantity;

/// Sorted `(outcome, quantity)` items, exactly as icepool's `Counts` keeps them. Zero quantities are kept.
///
/// For language bindings: the data of a die or deck (`Die::from_counts`), built once the binding has unpacked
/// Python's arguments.
///
/// ```
/// use icepool_core::value::Counts;
/// use icepool_core::{Die, Quantity};
/// let q = Quantity::from;
/// let counts = Counts::new(vec![(3, q(1u8)), (1, q(2u8)), (3, q(1u8))]).unwrap();
/// assert_eq!(counts.items(), &[(1, q(2u8)), (3, q(2u8))]);
/// assert_eq!(Die::from_counts(counts), Die::from_weights([(1, 2u32), (3, 2)]));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Counts<O> {
    items: Vec<(O, Quantity)>,
}

impl<O: OutcomeOrder> Counts<O> {
    /// `Counts(items)`: sorts the items (as Python sorts `(key, value)` tuples: by key, then value for keys that
    /// are `==`), then merges equal keys (as a dict does), keeping the first. Raises `TypeError` if the keys are not mutually comparable;
    /// a single key is compared with itself so that even a one-outcome die checks its outcome is orderable.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::{Quantity, Value};
    /// let q = Quantity::from;
    /// // Sorted, equal keys merged, zero quantities kept.
    /// let counts = Counts::new(vec![(2, q(0u8)), (1, q(1u8)), (2, q(3u8))]).unwrap();
    /// assert_eq!(counts.items(), [(1, q(1u8)), (2, q(3u8))]);
    /// // Python's `TypeError`: a str and an int do not compare.
    /// assert!(Counts::new(vec![(Value::str("a"), q(1u8)), (Value::from(1), q(1u8))]).is_err());
    /// ```
    pub fn new(items: Vec<(O, Quantity)>) -> std::result::Result<Self, O::Incomparable> {
        if let [(k, _)] = items.as_slice() {
            k.try_lt(k)?;
        }
        let sorted = O::try_sort_by(items, |(ka, qa), (kb, qb)| {
            if ka.try_eq(kb)? {
                Ok(qa < qb)
            } else {
                ka.try_lt(kb)
            }
        })?;
        let mut merged: Vec<(O, Quantity)> = Vec::with_capacity(sorted.len());
        for (k, q) in sorted {
            k.check_key()?;
            // Sorted, so equal keys are adjacent.
            match merged.last_mut() {
                Some((last, total)) if *last == k => *total += q,
                _ => merged.push((k, q)),
            }
        }
        Ok(Counts { items: merged })
    }

    /// Items already sorted and merged (e.g. by construction, as in `d(n)`). Checked in debug builds.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::Quantity;
    /// let items = vec![(1, Quantity::ONE), (2, Quantity::ONE)];
    /// assert_eq!(Counts::from_sorted(items.clone()), Counts::new(items).unwrap());
    /// ```
    pub fn from_sorted(items: Vec<(O, Quantity)>) -> Self {
        debug_assert!(items
            .windows(2)
            .all(|w| outcome::lt(&w[0].0, &w[1].0) == Ok(true)));
        Counts { items }
    }

    /// The `(outcome, quantity)` items, in ascending order of the outcomes.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::Quantity;
    /// let counts = Counts::new(vec![("b", Quantity::ONE), ("a", Quantity::ONE)]).unwrap();
    /// assert_eq!(counts.items(), [("a", Quantity::ONE), ("b", Quantity::ONE)]);
    /// ```
    pub fn items(&self) -> &[(O, Quantity)] {
        &self.items
    }

    /// The number of outcomes (zero quantities included).
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::Quantity;
    /// // Zero quantities count.
    /// assert_eq!(Counts::new(vec![(1, Quantity::ONE), (2, Quantity::ZERO)]).unwrap().len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no outcomes at all.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::Quantity;
    /// assert!(Counts::<i64>::new(vec![]).unwrap().is_empty());
    /// assert!(!Counts::new(vec![(1, Quantity::ZERO)]).unwrap().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `True` iff `self` contains at least one zero value.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::Quantity;
    /// assert!(Counts::new(vec![(1, Quantity::ONE), (2, Quantity::ZERO)]).unwrap().has_zero_values());
    /// assert!(!Counts::new(vec![(1, Quantity::ONE)]).unwrap().has_zero_values());
    /// ```
    pub fn has_zero_values(&self) -> bool {
        self.items.iter().any(|(_, q)| q.is_zero())
    }
}

/// A Python `defaultdict(int)` keyed by outcome: `add` is `data[outcome] += quantity`. Iteration is in insertion
/// order and an outcome equal to an earlier one (e.g. `1` after `True`) adds to it, keeping the earlier key.
///
/// ```
/// use icepool_core::value::OrderedMerge;
/// use icepool_core::{Quantity, Value};
/// let q = Quantity::from;
/// let mut merge = OrderedMerge::default();
/// merge.add(Value::Bool(true), q(1u8));
/// merge.add(Value::from(2), q(1u8));
/// merge.add(Value::from(1), q(3u8)); // `1 == True`: adds to `True`
/// let expected = vec![(Value::Bool(true), q(4u8)), (Value::from(2), q(1u8))];
/// assert_eq!(merge.into_items(), expected);
/// ```
#[derive(Debug)]
pub struct OrderedMerge<O> {
    index: FxHashMap<O, usize>,
    items: Vec<(O, Quantity)>,
}

impl<O: OutcomeOrder> Default for OrderedMerge<O> {
    fn default() -> Self {
        OrderedMerge {
            index: FxHashMap::default(),
            items: Vec::new(),
        }
    }
}

impl<O: OutcomeOrder> OrderedMerge<O> {
    /// `data[outcome] += quantity`.
    ///
    /// ```
    /// use icepool_core::value::OrderedMerge;
    /// use icepool_core::Quantity;
    /// let mut merge = OrderedMerge::default();
    /// merge.add(3, Quantity::ONE);
    /// merge.add(1, Quantity::ONE);
    /// merge.add(3, Quantity::ONE);
    /// assert_eq!(merge.into_items(), [(3, Quantity::from(2u8)), (1, Quantity::ONE)]);
    /// ```
    pub fn add(&mut self, outcome: O, quantity: Quantity) {
        match self.index.get(&outcome) {
            Some(&i) => self.items[i].1 += quantity,
            None => {
                self.index.insert(outcome.clone(), self.items.len());
                self.items.push((outcome, quantity));
            }
        }
    }

    /// The merged items, in the order their outcomes were first added.
    ///
    /// ```
    /// use icepool_core::value::OrderedMerge;
    /// use icepool_core::Quantity;
    /// let mut merge = OrderedMerge::default();
    /// merge.add("b", Quantity::ONE);
    /// merge.add("a", Quantity::ONE);
    /// // In order of first addition, not sorted.
    /// assert_eq!(merge.into_items(), [("b", Quantity::ONE), ("a", Quantity::ONE)]);
    /// ```
    pub fn into_items(self) -> Vec<(O, Quantity)> {
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    #[test]
    fn sorts_and_raises_like_python() {
        let q = |x: u8| Quantity::from(x);
        let c = Counts::new(vec![
            (Value::from(3), q(1)),
            (Value::Float(1.5), q(2)),
            (Value::Bool(true), q(1)),
        ])
        .unwrap();
        let keys: Vec<String> = c.items().iter().map(|(k, _)| k.to_string()).collect();
        assert_eq!(keys, ["True", "1.5", "3"]);
        assert!(Counts::new(vec![(Value::from(3), q(1)), (Value::str("a"), q(1))]).is_err());
        let sorted =
            crate::value::sort::timsort((0..50).rev().collect(), |a: &i32, b| Ok(a < b)).unwrap();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
    }
}
