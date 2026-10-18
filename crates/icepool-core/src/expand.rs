//! Port of icepool `expand.py`: Cartesian products of dice, for any outcome type. Python's tuple and `Vector`
//! outcomes (whose keys Python hashes) are `value::expand`'s.

use crate::error::Result;
use crate::outcome::{infallible, Outcome, OutcomeOrder};
use crate::{Die, Quantity};

/// Yields the independent joint distribution of the arguments (`iter_cartesian_product`).
///
/// Each argument is given as its `(outcome, quantity)` items: a die's items (zero quantities included), or a
/// single `(arg, 1)` for an argument that is not a die, which is passed through as-is. Yields one outcome per
/// argument and the joint quantity, in `itertools.product` order (the last argument varies fastest). No arguments
/// yield a single empty product with quantity 1.
pub fn iter_cartesian_product<T: Clone>(
    args: &[Vec<(T, Quantity)>],
) -> impl Iterator<Item = (Vec<T>, Quantity)> + '_ {
    // An odometer over the argument indices; `None` once exhausted (immediately, if any argument is empty).
    let mut index: Option<Vec<usize>> = if args.iter().any(Vec::is_empty) {
        None
    } else {
        Some(vec![0; args.len()])
    };
    std::iter::from_fn(move || {
        let current = index.as_mut()?;
        let mut outcomes = Vec::with_capacity(args.len());
        let mut quantity = Quantity::ONE;
        for (arg, &i) in args.iter().zip(current.iter()) {
            outcomes.push(arg[i].0.clone());
            quantity *= &arg[i].1;
        }
        // Advance the last argument first; when every position wraps around, the product is done.
        let mut pos = args.len();
        loop {
            if pos == 0 {
                index = None;
                break;
            }
            pos -= 1;
            current[pos] += 1;
            if current[pos] < args[pos].len() {
                break;
            }
            current[pos] = 0;
        }
        Some((outcomes, quantity))
    })
}

/// How many joint outcomes `iter_cartesian_product(args)` yields, `None` past `usize`.
pub fn product_size<T>(args: &[Vec<T>]) -> Option<usize> {
    if args.iter().any(Vec::is_empty) {
        return Some(0);
    }
    args.iter()
        .try_fold(1usize, |n, arg| n.checked_mul(arg.len()))
}

/// The `data` dict of `cartesian_product(*args, outcome_type=outcome_type)`: `data[outcome_type(outcomes)] =
/// final_quantity` for each joint outcome of `args`, in `itertools.product` order. Assignment, not `+=`: equal keys
/// such as Python's `(1,)` and `(True,)` overwrite each other's quantity and keep the first key. The caller makes
/// `Die(data)` or `Deck(data)` of it (which drops zero quantities); `outcome_type` may refuse a key.
pub fn cartesian_product_items<T: Clone, P: OutcomeOrder>(
    args: &[Vec<(T, Quantity)>],
    outcome_type: impl Fn(Vec<T>) -> Result<P>,
) -> Result<Vec<(P, Quantity)>> {
    // The dict holds up to one item per joint outcome: refuse up front a product no memory holds (`MemoryError`),
    // rather than abort when growing it fails.
    crate::memory::check_len::<(P, Quantity), _>(product_size(args), "a cartesian product")?;
    let mut data: Vec<(P, Quantity)> = Vec::new();
    let mut index: rustc_hash::FxHashMap<P, usize> = rustc_hash::FxHashMap::default();
    for (outcomes, quantity) in iter_cartesian_product(args) {
        crate::interrupt::poll()?;
        let key = outcome_type(outcomes)?;
        match index.get(&key) {
            Some(&i) => data[i].1 = quantity,
            None => {
                index.insert(key.clone(), data.len());
                data.push((key, quantity));
            }
        }
    }
    Ok(data)
}

/// The typed `tupleize`: joint distributions of independent dice.
impl<O: Outcome> Die<O> {
    /// Python's `icepool.tupleize(self, other)`: the independent joint distribution of the two dice, as pairs.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let joint = d(2).tupleize(&Die::from_iter([true, false]));
    /// assert_eq!(joint.outcomes().cloned().collect::<Vec<_>>(), [(1, false), (1, true), (2, false), (2, true)]);
    /// ```
    pub fn tupleize<P: Outcome>(&self, other: &Die<P>) -> Die<(O, P)> {
        // Typed pairs of distinct outcomes are distinct keys, so the product has nothing to merge or overwrite.
        let pairs = self.items().iter().flat_map(|(a, qa)| {
            other
                .items()
                .iter()
                .map(move |(b, qb)| ((a.clone(), b.clone()), qa * qb))
        });
        infallible(Die::from_dict(pairs.collect()))
    }

    /// Python's `icepool.tupleize(*dice)` for dice of one outcome type: the independent joint distribution, one
    /// outcome of each die per `Vec` (Python's tuple).
    ///
    /// # Panics
    ///
    /// If the product has more outcomes than memory holds (Python's `MemoryError`).
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let joint = Die::cartesian_product(&[d(2), d(3)]);
    /// assert_eq!(joint.len(), 6);
    /// assert_eq!(joint.max_outcome().unwrap(), &vec![2, 3]);
    /// ```
    pub fn cartesian_product(dice: &[Die<O>]) -> Die<Vec<O>> {
        let args: Vec<_> = dice.iter().map(|die| die.items().to_vec()).collect();
        let items = cartesian_product_items(&args, Ok).expect("a product that fits in memory");
        infallible(Die::from_dict(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_order_and_quantities() {
        let q = |x: u8| Quantity::from(x);
        let a = vec![(1, q(1)), (2, q(2))];
        let b = vec![(10, q(3)), (20, q(0))];
        let got: Vec<_> = iter_cartesian_product(&[a, b]).collect();
        assert_eq!(
            got,
            vec![
                (vec![1, 10], q(3)),
                (vec![1, 20], q(0)),
                (vec![2, 10], q(6)),
                (vec![2, 20], q(0)),
            ]
        );
        // `icepool.tupleize(d2, d2)` keeps no zero quantities.
        let zero = crate::Die::from_dict(vec![(1, q(1)), (2, q(0))]).unwrap();
        assert_eq!(
            crate::Die::cartesian_product(&[zero.clone(), zero]).len(),
            1
        );
        assert_eq!(
            iter_cartesian_product::<i32>(&[]).collect::<Vec<_>>(),
            vec![(vec![], q(1))]
        );
        assert_eq!(
            iter_cartesian_product(&[vec![(1, q(1))], vec![]]).count(),
            0
        );
    }
}
