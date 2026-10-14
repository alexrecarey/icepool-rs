//! Port of icepool `expand.py`: Cartesian products of dice.

use crate::Quantity;

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
