//! The typed statistics of population/base.py: exact means, variances and probabilities of dice whose outcomes
//! are numbers ([`ToRatio`]), as Python computes them for `int` dice (`Fraction`s), and the standard deviation as
//! the float `math.sqrt` makes of the variance.
//!
//! The Python-typed versions, for any `Value` (a `float` mean of a float die, `percent`, Python's errors), are
//! `value::PyStatistics`.

use crate::outcome::{Outcome, ToRatio};
use crate::population::Comparison;
use crate::value::ratio_to_f64;
use crate::{Die, Quantity, Ratio};

impl<O: Outcome + ToRatio> Die<O> {
    /// `Fraction(quantity, denominator)`, the probability a quantity stands for.
    fn fraction(&self, quantity: &Quantity) -> Ratio {
        assert!(!self.is_empty(), "division by zero: the die is empty");
        Ratio::from_parts(quantity.clone().into(), self.denominator())
    }

    /// The mean outcome, exactly: `sum(outcome * quantity) / denominator`.
    ///
    /// # Panics
    ///
    /// If the die is empty (Python raises `ZeroDivisionError`).
    ///
    /// ```
    /// use icepool_core::{d, Ratio};
    /// assert_eq!(d(6).mean(), Ratio::from_parts(7.into(), 2u8.into()));
    /// ```
    pub fn mean(&self) -> Ratio {
        let total = self.items().iter().fold(Ratio::ZERO, |sum, (o, q)| {
            sum + o.to_ratio() * Ratio::from(q.clone())
        });
        total * self.fraction(&Quantity::ONE)
    }

    /// The population variance (not the sample variance), exactly: the mean of the squares minus the square of
    /// the mean.
    ///
    /// # Panics
    ///
    /// If the die is empty (Python raises `ZeroDivisionError`).
    ///
    /// ```
    /// use icepool_core::{d, Ratio};
    /// assert_eq!(d(6).variance(), Ratio::from_parts(35.into(), 12u8.into()));
    /// ```
    pub fn variance(&self) -> Ratio {
        let mean = self.mean();
        let squares = self.items().iter().fold(Ratio::ZERO, |sum, (o, q)| {
            sum + Ratio::from(q.clone()) * o.to_ratio().sqr()
        });
        squares * self.fraction(&Quantity::ONE) - mean.sqr()
    }

    /// `math.sqrt(variance)`: the variance rounded to the nearest float, then its square root (correctly rounded,
    /// so the same bits as Python's).
    ///
    /// # Panics
    ///
    /// If the die is empty, or the variance is beyond the float range (Python raises `ZeroDivisionError` and
    /// `OverflowError`).
    ///
    /// ```
    /// use icepool_core::d;
    /// assert_eq!(d(6).standard_deviation(), 1.707825127659933);
    /// ```
    pub fn standard_deviation(&self) -> f64 {
        let variance = self.variance();
        let variance = ratio_to_f64(variance.numerator(), &variance.denominator().clone().into())
            .expect("the variance of a die fits a float");
        variance.sqrt()
    }

    /// The total probability of the outcomes fitting a comparison to `outcome`, exactly.
    ///
    /// # Panics
    ///
    /// If the die is empty (Python raises `ZeroDivisionError`).
    ///
    /// ```
    /// use icepool_core::{d, Comparison, Ratio};
    /// assert_eq!(d(6).probability(Comparison::Le, &2), Ratio::from_parts(1.into(), 3u8.into()));
    /// assert_eq!(d(6).probability(Comparison::Eq, &7), Ratio::ZERO);
    /// ```
    pub fn probability(&self, comparison: Comparison, outcome: &O) -> Ratio {
        self.fraction(&self.quantity(comparison, outcome))
    }

    /// The probability fitting the comparison for each outcome in ascending order, exactly: `Comparison::Le`
    /// gives the CDF. Python's `probabilities(comparison)`.
    ///
    /// # Panics
    ///
    /// If the die is empty (Python raises `ZeroDivisionError`).
    ///
    /// ```
    /// use icepool_core::{d, Comparison, Ratio};
    /// let cdf = d(4).probabilities(Comparison::Le);
    /// assert_eq!(cdf.last(), Some(&Ratio::ONE));
    /// ```
    pub fn probabilities(&self, comparison: Comparison) -> Vec<Ratio> {
        self.quantities_by(comparison)
            .iter()
            .map(|q| self.fraction(q))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d;

    fn ratio(n: i64, d: u64) -> Ratio {
        Ratio::from_parts(n.into(), d.into())
    }

    #[test]
    fn statistics_as_python() {
        // `(d(6) + d(6)).mean()`, `.variance()`, `.standard_deviation()`: Fraction(7, 1), Fraction(35, 6),
        // 2.41522945769824.
        let two = &d(6) + &d(6);
        assert_eq!(two.mean(), ratio(7, 1));
        assert_eq!(two.variance(), ratio(35, 6));
        assert_eq!(two.standard_deviation(), 2.41522945769824);
        // `Die([-3, 0, 0, 4]).mean()`, `.variance()`: Fraction(1, 4), Fraction(99, 16).
        let skewed = Die::from_iter([-3i64, 0, 0, 4]);
        assert_eq!(skewed.mean(), ratio(1, 4));
        assert_eq!(skewed.variance(), ratio(99, 16));
        // `Die([True, False, False]).mean()`: Fraction(1, 3).
        assert_eq!(Die::from_iter([true, false, false]).mean(), ratio(1, 3));
        // A Fraction die: `Die([Fraction(1, 2), Fraction(1, 3)]).mean()` is Fraction(5, 12).
        assert_eq!(
            Die::from_iter([ratio(1, 2), ratio(1, 3)]).mean(),
            ratio(5, 12)
        );
    }

    #[test]
    fn probabilities_as_python() {
        // `d(6).probability('>', 4)`, `d(6).probabilities('>=')`.
        assert_eq!(d(6).probability(Comparison::Gt, &4), ratio(1, 3));
        assert_eq!(
            d(3).probabilities(Comparison::Ge),
            [ratio(1, 1), ratio(2, 3), ratio(1, 3)]
        );
        assert_eq!(d(3).probabilities(Comparison::Ne)[0], ratio(2, 3));
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn the_mean_of_an_empty_die_divides_by_zero() {
        Die::<i64>::from_iter([]).mean();
    }
}
