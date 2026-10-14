//! Port of icepool `math.py`.

use dashu_base::Gcd;

use crate::Quantity;

/// Computes a minimal scale factor for each denominator so that they are in the ratio given by the weights.
///
/// If a denominator or its corresponding weight is 0, the corresponding result is 0.
///
/// Mixing dice must use exactly this (not the plain product of denominators), or quantities differ from icepool's
/// even when the probabilities agree.
pub fn weighted_lcm(denominators: &[Quantity], weights: &[Quantity]) -> Vec<Quantity> {
    // math.lcm() of nothing is 1.
    let mut denominator_lcm = Quantity::ONE;
    for (d, w) in denominators.iter().zip(weights) {
        if !d.is_zero() && !w.is_zero() {
            let x = d / d.gcd(w);
            denominator_lcm = lcm(&denominator_lcm, &x);
        }
    }
    denominators
        .iter()
        .zip(weights)
        .map(|(d, w)| {
            if d.is_zero() {
                Quantity::ZERO
            } else {
                &denominator_lcm * w / d
            }
        })
        .collect()
}

/// A row of n+1 elements, where the kth element is equal to `math.comb(n, k) * b ** k`.
///
/// Built row by row from `(1,)` as icepool does (Pascal's rule with the `b` factor folded in). icepool caches the rows
/// per `b` globally; the port computes the row it is asked for (the pool popper asks once per die per pop).
pub fn comb_row(n: usize, b: &Quantity) -> Vec<Quantity> {
    let mut row = vec![Quantity::ONE];
    for _ in 0..n {
        let mut next = Vec::with_capacity(row.len() + 1);
        next.push(Quantity::ONE);
        next.extend(row.windows(2).map(|w| &w[1] + b * &w[0]));
        next.push(b * row.last().expect("rows are never empty"));
        row = next;
    }
    row
}

/// `icepool.math.comb(n, k)`: the binomial coefficient (`k <= n` wherever icepool calls it).
///
/// icepool reads it off its cached `comb_row(n, 1)`; the port multiplies it out, which stays exact because every
/// partial product `n (n-1) ... (n-i) / (i+1)!` is itself a binomial coefficient.
pub fn comb(n: &Quantity, k: usize) -> Quantity {
    let mut result = Quantity::ONE;
    for i in 0..k {
        if *n < Quantity::from(i + 1) {
            return Quantity::ZERO;
        }
        result = result * (n - i) / (i + 1);
    }
    result
}

/// `iter_hypergeom(deck, draws)`: iterates over the `(hand, weight)`s in the given `Deck`.
///
/// `deck` is the number of duplicates of each card in the `Deck`, `draws` the total number of cards to draw. Each
/// `hand` says how many of each card were drawn; `weight` is the weight of drawing that hand. In Python's order: the
/// first card's count varies slowest.
pub fn iter_hypergeom(deck: &[usize], draws: usize) -> Vec<(Vec<usize>, Quantity)> {
    fn go(
        deck: &[usize],
        draws: usize,
        prefix: &mut Vec<usize>,
        weight: &Quantity,
        out: &mut Vec<(Vec<usize>, Quantity)>,
    ) {
        let Some((&deck_count, tail)) = deck.split_first() else {
            out.push((prefix.clone(), weight.clone()));
            return;
        };
        let deck_size: usize = deck.iter().sum(); // assume deck is small?
        let min_count = (deck_count + draws).saturating_sub(deck_size);
        let max_count = deck_count.min(draws);
        for count in min_count..=max_count {
            let w = weight * comb(&Quantity::from(draws), count);
            prefix.push(count);
            go(tail, draws - count, prefix, &w, out);
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    go(deck, draws, &mut Vec::new(), &Quantity::ONE, &mut out);
    out
}

pub fn lcm(a: &Quantity, b: &Quantity) -> Quantity {
    if a.is_zero() || b.is_zero() {
        return Quantity::ZERO;
    }
    a / a.gcd(b) * b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(v: &[u32]) -> Vec<Quantity> {
        v.iter().map(|&x| Quantity::from(x)).collect()
    }

    #[test]
    fn scales_minimally() {
        // A d4 and a d6 at equal weight: lcm 12, so the d4 is scaled by 3 and the d6 by 2.
        assert_eq!(weighted_lcm(&q(&[4, 6]), &q(&[1, 1])), q(&[3, 2]));
        // A d6 at weight 2 against a single outcome at weight 1: 6 // gcd(6, 2) = 3, lcm(3, 1) = 3.
        assert_eq!(weighted_lcm(&q(&[6, 1]), &q(&[2, 1])), q(&[1, 3]));
        assert_eq!(weighted_lcm(&q(&[0, 1, 3]), &q(&[1, 0, 1])), q(&[0, 0, 1]));
    }

    #[test]
    fn comb_row_is_binomial_times_powers() {
        // comb(3, k) * 2**k
        assert_eq!(comb_row(3, &Quantity::from(2u8)), q(&[1, 6, 12, 8]));
        assert_eq!(comb_row(0, &Quantity::from(5u8)), q(&[1]));
        assert_eq!(comb_row(2, &Quantity::ZERO), q(&[1, 0, 0]));
    }

    #[test]
    fn comb_matches_pascal() {
        for n in 0..12u32 {
            let row = comb_row(n as usize, &Quantity::ONE);
            for (k, c) in row.iter().enumerate() {
                assert_eq!(comb(&Quantity::from(n), k), *c);
            }
            assert_eq!(comb(&Quantity::from(n), n as usize + 1), Quantity::ZERO);
        }
    }
}
