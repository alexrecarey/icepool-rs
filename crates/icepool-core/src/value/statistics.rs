//! `PyStatistics`: the probabilities, quantiles and statistics of `Die<Value>` (population/base.py: `probability`
//! to `quantile`, `mean` to `entropy`, the Kolmogorov-Smirnov and Cramér-von Mises statistics, `covariance` and
//! `correlation`). They do arithmetic on the outcomes, so they are written with Python's own operators and
//! builtins (`sum`, `math.log`...): a `Fraction` where Python gives one, floats rounded exactly as CPython rounds
//! them, and Python's exceptions. The typed, exact statistics are in `crate::statistics`; what only needs the order
//! of the outcomes is generic, in `crate::population`.

use dashu_int::IBig;
use dashu_ratio::RBig;

use crate::error::{Error, Result};
use crate::function::sorted_union;
use crate::population::{bisect, int, Comparison};
use crate::value::builtins::{getitem, math_log, math_sqrt, py_sum, try_fraction, Index};
use crate::value::{BinOp, PyDie, UnOp};
use crate::{Die, Quantity, Value};

/// The statistics of population/base.py for `Die<Value>`, with Python's types: a `Fraction` where Python gives one,
/// a `float` for float outcomes, `percent` probabilities, and Python's exceptions. The typed, exact versions are
/// inherent methods of `Die<O: Outcome + ToRatio>` (`mean`, `variance`, `probability`, ...): Rust picks the
/// inherent method for a typed die and this trait's for a `Die<Value>`.
pub trait PyStatistics {
    /// `try_fraction(sum(outcome * quantity ...), denominator)`: a `Fraction` for rational outcomes, a `float`
    /// for float ones.
    fn mean(&self) -> Result<Value>;

    /// This is the population variance, not the sample variance.
    fn variance(&self) -> Result<Value>;

    /// `math.sqrt(self.variance())`.
    fn standard_deviation(&self) -> Result<Value>;

    /// `sum(p * (outcome - mean)**k ...) / sd**k`.
    fn standardized_moment(&self, k: &Value) -> Result<Value>;

    /// `standardized_moment(3)`.
    fn skewness(&self) -> Result<Value>;

    /// `standardized_moment(4) - 3.0`.
    fn excess_kurtosis(&self) -> Result<Value>;

    /// The entropy of a random sample from this population: `-sum(p * math.log(p, base) for p in
    /// self.probabilities() if p > 0.0)`. An empty die sums nothing (the int `0`, and `base` is never looked at).
    fn entropy(&self, base: &Value) -> Result<Value>;

    /// Kolmogorov–Smirnov statistic. The maximum absolute difference between CDFs.
    fn kolmogorov_smirnov(&self, other: &Die<Value>) -> Result<Value>;

    /// Cramér-von Mises statistic. The sum-of-squares difference between CDFs.
    fn cramer_von_mises(&self, other: &Die<Value>) -> Result<Value>;

    /// `try_fraction(sum((outcome[i] - mean_i) * (outcome[j] - mean_j) * quantity ...), denominator)`, the means
    /// those of `marginals[i]` and `marginals[j]`.
    fn covariance(&self, i: &Index, j: &Index) -> Result<Value>;

    /// `covariance(i, j) / (sd_i * sd_j)`.
    fn correlation(&self, i: &Index, j: &Index) -> Result<Value>;

    /// The total probability of outcomes fitting a comparison: a `Fraction`, or with `percent` a percentage
    /// (`result * 100.0`, a `float`).
    fn probability(&self, comparison: Comparison, outcome: &Value, percent: bool) -> Result<Value>;

    /// The total probabilities fitting the comparison for each outcome in sorted order (`'<='` gives the CDF).
    /// With `percent`, each is `100.0 * x`.
    fn probabilities(&self, comparison: Comparison, percent: bool) -> Result<Vec<Value>>;

    /// The probability of the outcomes `which` selected: `100.0 * numerator / denominator` with `percent`, else
    /// `Fraction(numerator, denominator)`.
    fn probability_where(&self, selected: &[bool], percent: bool) -> Result<Value>;

    /// The outcome `n / d` of the way through the CDF, taking the lesser in case of a tie.
    fn quantile_low(&self, n: &Value, d: &Value) -> Result<Value>;

    /// The outcome `n / d` of the way through the CDF, taking the greater in case of a tie.
    fn quantile_high(&self, n: &Value, d: &Value) -> Result<Value>;

    /// The outcome `n / d` of the way through the CDF, taking the mean in case of a tie. This will fail if the
    /// outcomes do not support addition and division.
    fn quantile(&self, n: &Value, d: &Value) -> Result<Value>;
}

impl PyStatistics for Die<Value> {
    fn mean(&self) -> Result<Value> {
        let total = py_sum(
            self.items()
                .iter()
                .map(|(o, q)| o.binary(BinOp::Mul, &int(q))),
            Value::from(0),
        )?;
        try_fraction(&total, &self.den())
    }

    fn variance(&self) -> Result<Value> {
        let mean = self.mean()?;
        let squares = py_sum(
            self.items()
                .iter()
                .map(|(o, q)| int(q).binary(BinOp::Mul, &o.binary(BinOp::Pow, &Value::from(2))?)),
            Value::from(0),
        )?;
        let mean_of_squares = try_fraction(&squares, &self.den())?;
        mean_of_squares.binary(BinOp::Sub, &mean.binary(BinOp::Mul, &mean)?)
    }

    fn standard_deviation(&self) -> Result<Value> {
        math_sqrt(&self.variance()?)
    }

    fn standardized_moment(&self, k: &Value) -> Result<Value> {
        let sd = self.standard_deviation()?;
        let mean = self.mean()?;
        let probabilities = self.probabilities(Comparison::Eq, false)?;
        let ev = py_sum(
            self.outcomes().zip(probabilities).map(|(o, p)| {
                p.binary(
                    BinOp::Mul,
                    &o.binary(BinOp::Sub, &mean)?.binary(BinOp::Pow, k)?,
                )
            }),
            Value::from(0),
        )?;
        ev.binary(BinOp::TrueDiv, &sd.binary(BinOp::Pow, k)?)
    }

    fn skewness(&self) -> Result<Value> {
        self.standardized_moment(&Value::from(3))
    }

    fn excess_kurtosis(&self) -> Result<Value> {
        self.standardized_moment(&Value::from(4))?
            .binary(BinOp::Sub, &Value::Float(3.0))
    }

    fn entropy(&self, base: &Value) -> Result<Value> {
        let zero = Value::Float(0.0);
        let terms = self
            .probabilities(Comparison::Eq, false)?
            .into_iter()
            .filter_map(
                |p| match p.binary(BinOp::Gt, &zero).and_then(|x| x.truthy()) {
                    Ok(false) => None,
                    Ok(true) => {
                        Some(math_log(&p, Some(base)).and_then(|l| p.binary(BinOp::Mul, &l)))
                    }
                    Err(e) => Some(Err(e)),
                },
            );
        py_sum(terms, Value::from(0))?.unary(UnOp::Neg)
    }

    fn kolmogorov_smirnov(&self, other: &Die<Value>) -> Result<Value> {
        let mut max: Option<Value> = None;
        for d in self.cdf_differences(other)? {
            let d = d?.unary(UnOp::Abs)?;
            // `max` keeps the first of equal items: it replaces only on `item > max`.
            max = match max {
                Some(m) if !d.binary(BinOp::Gt, &m)?.truthy()? => Some(m),
                _ => Some(d),
            };
        }
        max.ok_or_else(|| Error::InvalidArgument("max() iterable argument is empty".into()))
    }

    fn cramer_von_mises(&self, other: &Die<Value>) -> Result<Value> {
        let squares = self
            .cdf_differences(other)?
            .map(|d| d?.binary(BinOp::Pow, &Value::from(2)));
        py_sum(squares, Value::Fraction(RBig::ZERO))
    }

    fn covariance(&self, i: &Index, j: &Index) -> Result<Value> {
        let mean_i = self.marginal(i)?.mean()?;
        let mean_j = self.marginal(j)?.mean()?;
        let total = py_sum(
            self.items().iter().map(|(o, q)| {
                getitem(o, i)?
                    .binary(BinOp::Sub, &mean_i)?
                    .binary(BinOp::Mul, &getitem(o, j)?.binary(BinOp::Sub, &mean_j)?)?
                    .binary(BinOp::Mul, &int(q))
            }),
            Value::from(0),
        )?;
        try_fraction(&total, &self.den())
    }

    fn correlation(&self, i: &Index, j: &Index) -> Result<Value> {
        let sd_i = self.marginal(i)?.standard_deviation()?;
        let sd_j = self.marginal(j)?.standard_deviation()?;
        self.covariance(i, j)?
            .binary(BinOp::TrueDiv, &sd_i.binary(BinOp::Mul, &sd_j)?)
    }

    fn probability(&self, comparison: Comparison, outcome: &Value, percent: bool) -> Result<Value> {
        let result = self.fraction(self.try_quantity(comparison, outcome)?)?;
        if percent {
            result.binary(BinOp::Mul, &Value::Float(100.0))
        } else {
            Ok(result)
        }
    }

    fn probabilities(&self, comparison: Comparison, percent: bool) -> Result<Vec<Value>> {
        self.quantities_by(comparison)
            .into_iter()
            .map(|q| {
                let x = self.fraction(q)?;
                if percent {
                    Value::Float(100.0).binary(BinOp::Mul, &x)
                } else {
                    Ok(x)
                }
            })
            .collect()
    }

    fn probability_where(&self, selected: &[bool], percent: bool) -> Result<Value> {
        let numerator = int(&self.quantity_where(selected));
        if percent {
            Value::Float(100.0)
                .binary(BinOp::Mul, &numerator)?
                .binary(BinOp::TrueDiv, &Value::Int(self.den()))
        } else {
            try_fraction(&numerator, &self.den())
        }
    }

    fn quantile_low(&self, n: &Value, d: &Value) -> Result<Value> {
        let cdf = self.quantities_by(Comparison::Le);
        // (n * self.denominator() + d - 1) // d
        let target = n
            .binary(BinOp::Mul, &Value::Int(self.den()))?
            .binary(BinOp::Add, d)?
            .binary(BinOp::Sub, &Value::from(1))?
            .binary(BinOp::FloorDiv, d)?;
        let index = bisect(&cdf, |q| int(q).binary(BinOp::Lt, &target)?.truthy())?;
        self.outcome_at(index)
    }

    fn quantile_high(&self, n: &Value, d: &Value) -> Result<Value> {
        let cdf = self.quantities_by(Comparison::Le);
        // n * self.denominator() // d
        let target = n
            .binary(BinOp::Mul, &Value::Int(self.den()))?
            .binary(BinOp::FloorDiv, d)?;
        let index = bisect(&cdf, |q| -> Result<bool> {
            Ok(!target.binary(BinOp::Lt, &int(q))?.truthy()?)
        })?;
        self.outcome_at(index)
    }

    fn quantile(&self, n: &Value, d: &Value) -> Result<Value> {
        let low = self.quantile_low(n, d)?;
        let high = self.quantile_high(n, d)?;
        low.binary(BinOp::Add, &high)?
            .binary(BinOp::TrueDiv, &Value::from(2))
    }
}

impl Die<Value> {
    /// `self.probability('<=', o) - other.probability('<=', o)` for each `o` in `sorted_union(self, other)`.
    fn cdf_differences<'a>(
        &'a self,
        other: &'a Die<Value>,
    ) -> Result<impl Iterator<Item = Result<Value>> + 'a> {
        let outcomes = sorted_union([self.outcomes(), other.outcomes()])?;
        Ok(outcomes.into_iter().map(move |o| {
            self.probability(Comparison::Le, &o, false)?
                .binary(BinOp::Sub, &other.probability(Comparison::Le, &o, false)?)
        }))
    }

    /// The denominator as a Python `int`.
    pub(crate) fn den(&self) -> IBig {
        self.denominator().into()
    }

    /// `Fraction(quantity, self.denominator())`: `ZeroDivisionError` for an empty die.
    fn fraction(&self, quantity: Quantity) -> Result<Value> {
        try_fraction(&int(&quantity), &self.den())
    }

    /// `self.outcomes()[index]`, or the max outcome past the end (`IndexError` if there is none).
    fn outcome_at(&self, index: usize) -> Result<Value> {
        match self.items().get(index) {
            Some((o, _)) => Ok(o.clone()),
            None => self.max_outcome().cloned(),
        }
    }
}
