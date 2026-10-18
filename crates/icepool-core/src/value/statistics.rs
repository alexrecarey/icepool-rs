//! The scalar and joint statistics of `Die<Value>` (population/base.py: `mean` to `entropy`, the
//! Kolmogorov-Smirnov and Cramér-von Mises statistics, `marginals`, `covariance` and `correlation`). They do
//! arithmetic on the outcomes, so they are written with Python's own operators and builtins (`sum`, `len`,
//! `math.log`...): a `Fraction` where Python gives one, floats rounded exactly as CPython rounds them, and
//! Python's exceptions. The rest of the population methods live in `crate::population`.

use dashu_ratio::RBig;

use crate::error::{Error, Result};
use crate::function::sorted_union;
use crate::population::{check_public_attr, int, Comparison};
use crate::value::builtins::{
    getattr, getitem, math_log, math_sqrt, py_len, py_sum, try_fraction, Index,
};
use crate::value::{BinOp, UnOp};
use crate::{Die, Value};

impl Die<Value> {
    /// `try_fraction(sum(outcome * quantity ...), denominator)`: a `Fraction` for rational outcomes, a `float`
    /// for float ones.
    pub fn mean(&self) -> Result<Value> {
        let total = py_sum(
            self.items()
                .iter()
                .map(|(o, q)| o.binary(BinOp::Mul, &int(q))),
            Value::from(0),
        )?;
        try_fraction(&total, &self.den())
    }

    /// This is the population variance, not the sample variance.
    pub fn variance(&self) -> Result<Value> {
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

    /// `math.sqrt(self.variance())`.
    pub fn standard_deviation(&self) -> Result<Value> {
        math_sqrt(&self.variance()?)
    }

    /// `sum(p * (outcome - mean)**k ...) / sd**k`.
    pub fn standardized_moment(&self, k: &Value) -> Result<Value> {
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

    /// `standardized_moment(3)`.
    pub fn skewness(&self) -> Result<Value> {
        self.standardized_moment(&Value::from(3))
    }

    /// `standardized_moment(4) - 3.0`.
    pub fn excess_kurtosis(&self) -> Result<Value> {
        self.standardized_moment(&Value::from(4))?
            .binary(BinOp::Sub, &Value::Float(3.0))
    }

    /// The entropy of a random sample from this population: `-sum(p * math.log(p, base) for p in
    /// self.probabilities() if p > 0.0)`. An empty die sums nothing (the int `0`, and `base` is never looked at).
    pub fn entropy(&self, base: &Value) -> Result<Value> {
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

    /// Kolmogorov–Smirnov statistic. The maximum absolute difference between CDFs.
    pub fn kolmogorov_smirnov(&self, other: &Die<Value>) -> Result<Value> {
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

    /// Cramér-von Mises statistic. The sum-of-squares difference between CDFs.
    pub fn cramer_von_mises(&self, other: &Die<Value>) -> Result<Value> {
        let squares = self
            .cdf_differences(other)?
            .map(|d| d?.binary(BinOp::Pow, &Value::from(2)));
        py_sum(squares, Value::Fraction(RBig::ZERO))
    }

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

    /// `marginals[index]`: the `[]` operator applied to each outcome (`_unary_operator(operator.getitem, dims)`).
    pub fn marginal(&self, index: &Index) -> Result<Die<Value>> {
        self.unary_operator(|o| getitem(o, index))
    }

    /// `marginals.<name>` (`_Marginals.__getattr__`): `getattr(outcome, name)` applied to each outcome. Private
    /// names are refused before any outcome is looked at (`check_public_attr`).
    pub fn marginal_attr(&self, name: &str) -> Result<Die<Value>> {
        check_public_attr(name)?;
        self.unary_operator(|o| getattr(o, name))
    }

    /// `len(marginals)`: the minimum `len()` of all outcomes (`ValueError` for an empty die).
    pub fn marginals_len(&self) -> Result<usize> {
        let mut min: Option<usize> = None;
        for o in self.outcomes() {
            let n = py_len(o)?;
            min = Some(min.map_or(n, |m| m.min(n)));
        }
        min.ok_or_else(|| Error::InvalidArgument("min() iterable argument is empty".into()))
    }

    /// `try_fraction(sum((outcome[i] - mean_i) * (outcome[j] - mean_j) * quantity ...), denominator)`, the means
    /// those of `marginals[i]` and `marginals[j]`.
    pub fn covariance(&self, i: &Index, j: &Index) -> Result<Value> {
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

    /// `covariance(i, j) / (sd_i * sd_j)`.
    pub fn correlation(&self, i: &Index, j: &Index) -> Result<Value> {
        let sd_i = self.marginal(i)?.standard_deviation()?;
        let sd_j = self.marginal(j)?.standard_deviation()?;
        self.covariance(i, j)?
            .binary(BinOp::TrueDiv, &sd_i.binary(BinOp::Mul, &sd_j)?)
    }
}
