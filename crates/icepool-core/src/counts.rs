//! Port of icepool `collection/counts.py`: an immutable map with sorted keys and integer values.

use rustc_hash::FxHashMap;

use crate::error::Result;
use crate::outcome::Outcome;
use crate::Quantity;

/// Sorted `(outcome, quantity)` items, exactly as icepool's `Counts` keeps them. Zero quantities are kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Counts<O> {
    items: Vec<(O, Quantity)>,
}

impl<O: Outcome> Counts<O> {
    /// `Counts(items)`: sorts the items (as Python sorts `(key, value)` tuples: by key, then value for equal keys),
    /// then merges equal keys, keeping the first. Raises `TypeError` if the keys are not mutually comparable;
    /// a single key is compared with itself so that even a one-outcome die checks its outcome is orderable.
    pub fn new(items: Vec<(O, Quantity)>) -> Result<Self> {
        if let [(k, _)] = items.as_slice() {
            k.py_lt(k)?;
        }
        let sorted = py_sorted(
            items,
            |(ka, qa), (kb, qb)| if ka == kb { Ok(qa < qb) } else { ka.py_lt(kb) },
        )?;
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
    pub fn from_sorted(items: Vec<(O, Quantity)>) -> Self {
        debug_assert!(items.windows(2).all(|w| w[0].0.py_lt(&w[1].0) == Ok(true)));
        Counts { items }
    }

    pub fn items(&self) -> &[(O, Quantity)] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `True` iff `self` contains at least one zero value.
    pub fn has_zero_values(&self) -> bool {
        self.items.iter().any(|(_, q)| q.is_zero())
    }
}

/// Python's `sorted()`: a stable sort that uses only `<` and stops at the first comparison that raises.
///
/// Python's timsort and this merge sort compare different pairs, which can only matter for inputs that are not
/// totally ordered (float NaN); for those the order may differ from Python's (see NOTES.md).
pub fn py_sorted<T>(items: Vec<T>, lt: impl Fn(&T, &T) -> Result<bool>) -> Result<Vec<T>> {
    let n = items.len();
    // Most inputs (d(n), results of arithmetic over sorted dice) are nearly or fully sorted already.
    let mut already = true;
    for i in 1..n {
        if lt(&items[i], &items[i - 1])? {
            already = false;
            break;
        }
    }
    if already {
        return Ok(items);
    }
    // Bottom-up merge sort over indices; on ties the left run wins, which keeps it stable.
    let mut idx: Vec<usize> = (0..n).collect();
    let mut tmp = idx.clone();
    let mut width = 1;
    while width < n {
        for start in (0..n).step_by(2 * width) {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut l, mut r) = (start, mid);
            for slot in &mut tmp[start..end] {
                let take_right = l >= mid || (r < end && lt(&items[idx[r]], &items[idx[l]])?);
                if take_right {
                    *slot = idx[r];
                    r += 1;
                } else {
                    *slot = idx[l];
                    l += 1;
                }
            }
        }
        std::mem::swap(&mut idx, &mut tmp);
        width *= 2;
    }
    let mut slots: Vec<Option<T>> = items.into_iter().map(Some).collect();
    Ok(idx
        .into_iter()
        .map(|i| slots[i].take().expect("each index once"))
        .collect())
}

/// A Python `defaultdict(int)` keyed by outcome: `add` is `data[outcome] += quantity`. Iteration is in insertion
/// order and an outcome equal to an earlier one (e.g. `1` after `True`) adds to it, keeping the earlier key.
#[derive(Debug)]
pub struct OrderedMerge<O> {
    index: FxHashMap<O, usize>,
    items: Vec<(O, Quantity)>,
}

impl<O: Outcome> Default for OrderedMerge<O> {
    fn default() -> Self {
        OrderedMerge {
            index: FxHashMap::default(),
            items: Vec::new(),
        }
    }
}

impl<O: Outcome> OrderedMerge<O> {
    pub fn add(&mut self, outcome: O, quantity: Quantity) {
        match self.index.get(&outcome) {
            Some(&i) => self.items[i].1 += quantity,
            None => {
                self.index.insert(outcome.clone(), self.items.len());
                self.items.push((outcome, quantity));
            }
        }
    }

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
        let sorted = py_sorted((0..50).rev().collect(), |a: &i32, b| Ok(a < b)).unwrap();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
    }
}
