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
}
