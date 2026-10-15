//! Port of icepool `population/base.py`: quantities and probabilities (with comparators), `nearest`, quantiles,
//! the scalar and joint statistics, `marginals`, `group_by`, and quantity management.
//!
//! What only needs the order of the outcomes is generic over `Outcome`. The statistics do arithmetic on the
//! outcomes, so they are written for `Value` with Python's own operators and builtins (`value::builtins`): a
//! `Fraction` where Python gives one, floats rounded exactly as CPython rounds them, and Python's exceptions.
//!
//! Methods that make a new population (`multiply_quantities`, `pad_to_denominator`, ...) return the `dict` Python
//! passes to `Die(...)`, as `(outcome, quantity)` pairs in insertion order, or `None` where Python returns `self`:
//! the quantities may be any Python number, and `Die(dict)`'s checks belong to the caller (the program layer or
//! the facade).

use std::hash::Hash;

use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use crate::error::{value_error, Error, Result};
use crate::function::sorted_union;
use crate::outcome::Outcome;
use crate::value::builtins::{getitem, math_log, math_sqrt, py_len, py_sum, try_fraction, Index};
use crate::value::{BinOp, UnOp};
use crate::{Die, Quantity, Value};

/// The comparisons `quantity`, `quantities`, `probability`, `probabilities` and `nearest` accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    Ne,
    Le,
    Lt,
    Ge,
    Gt,
}

impl Comparison {
    /// The comparison a Python argument names (`match comparison: case '==': ...`); `ValueError` otherwise.
    pub fn parse(c: &Value) -> Result<Self> {
        Ok(match c {
            Value::Str(s) => match &**s {
                "==" => Comparison::Eq,
                "!=" => Comparison::Ne,
                "<=" => Comparison::Le,
                "<" => Comparison::Lt,
                ">=" => Comparison::Ge,
                ">" => Comparison::Gt,
                _ => return invalid(c),
            },
            _ => return invalid(c),
        })
    }
}

fn invalid<T>(c: &Value) -> Result<T> {
    value_error(format!("Invalid comparison {c}"))
}

/// `bisect`'s binary search: the first index whose element does not `go_right`, probing midpoints as CPython's
/// `_bisect` does (the comparisons may raise, and may not be a consistent order).
fn bisect<T>(a: &[T], go_right: impl Fn(&T) -> Result<bool>) -> Result<usize> {
    let (mut lo, mut hi) = (0, a.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if go_right(&a[mid])? {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// A quantity as a Python `int`.
fn int(q: &Quantity) -> Value {
    Value::Int(q.clone().into())
}

impl<O: Outcome> Die<O> {
    /// The quantities fitting the comparison for each outcome in sorted order: `'<='` gives the CDF.
    ///
    /// `'>='` accumulates `values()[:-1]` downwards from the denominator, so an empty die has one: `(0,)`.
    pub fn quantities(&self, comparison: Comparison) -> Vec<Quantity> {
        let values = || self.items().iter().map(|(_, q)| q);
        let den = self.denominator();
        let complement = |qs: Vec<Quantity>| qs.into_iter().map(|q| &den - q).collect();
        match comparison {
            Comparison::Eq => values().cloned().collect(),
            Comparison::Le => self.quantities_le(),
            Comparison::Ge => {
                let mut out = vec![den.clone()];
                for q in values().take(self.len().saturating_sub(1)) {
                    let next = out.last().expect("not empty") - q;
                    out.push(next);
                }
                out
            }
            Comparison::Ne => complement(values().cloned().collect()),
            Comparison::Lt => complement(self.quantities(Comparison::Ge)),
            Comparison::Gt => complement(self.quantities(Comparison::Le)),
        }
    }

    /// The index of the nearest outcome fitting the comparison (`nearest`), or `None`; `Ok(Some((i, true)))` if
    /// `outcome in self` answered (Python then returns `outcome` itself).
    fn nearest_index(&self, comparison: Comparison, outcome: &O) -> Result<Option<(usize, bool)>> {
        let items = self.items();
        // bisect_left compares `a[mid] < x`, bisect_right `x < a[mid]`.
        let left = || bisect(items, |(o, _)| o.py_lt(outcome));
        let right = || bisect(items, |(o, _)| Ok(!outcome.py_lt(o)?));
        let below = |i: usize| i.checked_sub(1).map(|i| (i, false));
        let above = |i: usize| (i < items.len()).then_some((i, false));
        Ok(match comparison {
            Comparison::Le => match self.position(outcome) {
                Some(i) => Some((i, true)),
                None => below(right()?),
            },
            Comparison::Lt => below(left()?),
            Comparison::Ge => match self.position(outcome) {
                Some(i) => Some((i, true)),
                None => above(left()?),
            },
            Comparison::Gt => above(right()?),
            Comparison::Eq | Comparison::Ne => return value_error("Invalid comparison"),
        })
    }

    /// The nearest outcome in this population fitting the comparison (`'<='`, `'<'`, `'>='`, `'>'`), or `None` if
    /// there is no such outcome. For `'<='` and `'>='`, an outcome that is in the population is returned as given.
    pub fn nearest(&self, comparison: Comparison, outcome: &O) -> Result<Option<O>> {
        Ok(match self.nearest_index(comparison, outcome)? {
            None => None,
            Some((_, true)) => Some(outcome.clone()),
            Some((i, false)) => Some(self.items()[i].0.clone()),
        })
    }

    /// The total quantity fitting a comparison to a single outcome.
    pub fn quantity(&self, comparison: Comparison, outcome: &O) -> Result<Quantity> {
        let get = || {
            self.position(outcome)
                .map_or(Quantity::ZERO, |i| self.items()[i].1.clone())
        };
        Ok(match comparison {
            Comparison::Eq => get(),
            Comparison::Ne => self.denominator() - get(),
            Comparison::Le | Comparison::Lt => match self.nearest_index(comparison, outcome)? {
                None => Quantity::ZERO,
                // `_cumulative_quantities[threshold]`.
                Some((i, _)) => self.cumulative_quantities()[i].clone(),
            },
            Comparison::Ge => self.denominator() - self.quantity(Comparison::Lt, outcome)?,
            Comparison::Gt => self.denominator() - self.quantity(Comparison::Le, outcome)?,
        })
    }

    /// The quantity of the outcomes `which` selected (`quantity_where`, the selection made by the caller).
    pub fn quantity_where(&self, selected: &[bool]) -> Quantity {
        self.items()
            .iter()
            .zip(selected)
            .filter(|(_, s)| **s)
            .map(|((_, q), _)| q)
            .sum()
    }

    /// The highest quantity of any single outcome (`max(self.quantities())`: `ValueError` for an empty die).
    pub fn modal_quantity(&self) -> Result<Quantity> {
        self.items()
            .iter()
            .map(|(_, q)| q)
            .max()
            .cloned()
            .ok_or_else(|| Error::Value("max() iterable argument is empty".into()))
    }

    /// The most common outcome(s), lowest to highest. An empty die has none (and never asks `modal_quantity`).
    pub fn mode(&self) -> Vec<O> {
        let Ok(modal) = self.modal_quantity() else {
            return vec![];
        };
        self.items()
            .iter()
            .filter(|(_, q)| *q == modal)
            .map(|(o, _)| o.clone())
            .collect()
    }

    /// `group_by(key_map)` given each outcome's key: for each distinct key (by `==`, in order of first
    /// appearance), the die of the outcomes with that key (`Die(dict)`, so zero quantities drop out).
    pub fn group_by<K: Clone + Eq + Hash>(&self, keys: Vec<K>) -> Result<Vec<(K, Die<O>)>> {
        let mut index: FxHashMap<K, usize> = FxHashMap::default();
        let mut groups: Vec<(K, Vec<(O, Quantity)>)> = Vec::new();
        for (key, (outcome, quantity)) in keys.into_iter().zip(self.items()) {
            let i = *index.entry(key.clone()).or_insert_with(|| {
                groups.push((key, vec![]));
                groups.len() - 1
            });
            groups[i].1.push((outcome.clone(), quantity.clone()));
        }
        groups
            .into_iter()
            .map(|(k, items)| Ok((k, Die::from_dict(items)?)))
            .collect()
    }
}

/// A population's `dict` for `Die(...)`: `(outcome, quantity)` pairs in insertion order.
pub type Data = Vec<(Value, Value)>;

impl Die<Value> {
    fn den(&self) -> IBig {
        self.denominator().into()
    }

    /// `Fraction(quantity, self.denominator())`: `ZeroDivisionError` for an empty die.
    fn fraction(&self, quantity: Quantity) -> Result<Value> {
        try_fraction(&int(&quantity), &self.den())
    }

    /// The total probability of outcomes fitting a comparison: a `Fraction`, or with `percent` a percentage
    /// (`result * 100.0`, a `float`).
    pub fn probability(
        &self,
        comparison: Comparison,
        outcome: &Value,
        percent: bool,
    ) -> Result<Value> {
        let result = self.fraction(self.quantity(comparison, outcome)?)?;
        if percent {
            result.binary(BinOp::Mul, &Value::Float(100.0))
        } else {
            Ok(result)
        }
    }

    /// The total probabilities fitting the comparison for each outcome in sorted order (`'<='` gives the CDF).
    /// With `percent`, each is `100.0 * x`.
    pub fn probabilities(&self, comparison: Comparison, percent: bool) -> Result<Vec<Value>> {
        self.quantities(comparison)
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

    /// The probability of the outcomes `which` selected: `100.0 * numerator / denominator` with `percent`, else
    /// `Fraction(numerator, denominator)`.
    pub fn probability_where(&self, selected: &[bool], percent: bool) -> Result<Value> {
        let numerator = int(&self.quantity_where(selected));
        if percent {
            Value::Float(100.0)
                .binary(BinOp::Mul, &numerator)?
                .binary(BinOp::TrueDiv, &Value::Int(self.den()))
        } else {
            try_fraction(&numerator, &self.den())
        }
    }

    /// The outcome `n / d` of the way through the CDF, taking the lesser in case of a tie.
    pub fn quantile_low(&self, n: &Value, d: &Value) -> Result<Value> {
        let cdf = self.quantities(Comparison::Le);
        // (n * self.denominator() + d - 1) // d
        let target = n
            .binary(BinOp::Mul, &Value::Int(self.den()))?
            .binary(BinOp::Add, d)?
            .binary(BinOp::Sub, &Value::from(1))?
            .binary(BinOp::FloorDiv, d)?;
        let index = bisect(&cdf, |q| int(q).binary(BinOp::Lt, &target)?.truthy())?;
        self.outcome_at(index)
    }

    /// The outcome `n / d` of the way through the CDF, taking the greater in case of a tie.
    pub fn quantile_high(&self, n: &Value, d: &Value) -> Result<Value> {
        let cdf = self.quantities(Comparison::Le);
        // n * self.denominator() // d
        let target = n
            .binary(BinOp::Mul, &Value::Int(self.den()))?
            .binary(BinOp::FloorDiv, d)?;
        let index = bisect(&cdf, |q| {
            Ok(!target.binary(BinOp::Lt, &int(q))?.truthy()?)
        })?;
        self.outcome_at(index)
    }

    /// `self.outcomes()[index]`, or the max outcome past the end (`IndexError` if there is none).
    fn outcome_at(&self, index: usize) -> Result<Value> {
        match self.items().get(index) {
            Some((o, _)) => Ok(o.clone()),
            None => self.max_outcome().cloned(),
        }
    }

    /// The outcome `n / d` of the way through the CDF, taking the mean in case of a tie. This will fail if the
    /// outcomes do not support addition and division.
    pub fn quantile(&self, n: &Value, d: &Value) -> Result<Value> {
        let low = self.quantile_low(n, d)?;
        let high = self.quantile_high(n, d)?;
        low.binary(BinOp::Add, &high)?
            .binary(BinOp::TrueDiv, &Value::from(2))
    }

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

    pub fn skewness(&self) -> Result<Value> {
        self.standardized_moment(&Value::from(3))
    }

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
        max.ok_or_else(|| Error::Value("max() iterable argument is empty".into()))
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

    /// `len(marginals)`: the minimum `len()` of all outcomes (`ValueError` for an empty die).
    pub fn marginals_len(&self) -> Result<usize> {
        let mut min: Option<usize> = None;
        for o in self.outcomes() {
            let n = py_len(o)?;
            min = Some(min.map_or(n, |m| m.min(n)));
        }
        min.ok_or_else(|| Error::Value("min() iterable argument is empty".into()))
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

    /// Each `(outcome, f(quantity))`, zero quantities included.
    fn map_quantities(&self, f: impl Fn(Value) -> Result<Value>) -> Result<Data> {
        self.items()
            .iter()
            .map(|(o, q)| Ok((o.clone(), f(int(q))?)))
            .collect()
    }

    /// Multiplies all quantities by an integer (`None`: `scale == 1`, Python returns the population itself).
    pub fn multiply_quantities(&self, scale: &Value) -> Result<Option<Data>> {
        if *scale == Value::from(1) {
            return Ok(None);
        }
        self.map_quantities(|q| q.binary(BinOp::Mul, scale))
            .map(Some)
    }

    /// Divides all quantities by an integer, rounding down; resulting zero quantities are dropped (only those at
    /// least `divisor` are kept). `None`: `divisor == 0`, Python returns the population itself.
    pub fn divide_quantities(&self, divisor: &Value) -> Result<Option<Data>> {
        if *divisor == Value::from(0) {
            return Ok(None);
        }
        let mut data = Data::new();
        for (o, q) in self.items() {
            let q = int(q);
            if q.binary(BinOp::Ge, divisor)?.truthy()? {
                data.push((o.clone(), q.binary(BinOp::FloorDiv, divisor)?));
            }
        }
        Ok(Some(data))
    }

    /// Modulus of all quantities with an integer.
    pub fn modulo_quantities(&self, divisor: &Value) -> Result<Data> {
        self.map_quantities(|q| q.binary(BinOp::Mod, divisor))
    }

    /// Changes the denominator to a target number by changing the quantity of a specified outcome. If that
    /// quantity becomes zero, the outcome is removed (`del data[outcome]`: `KeyError` if it was not there);
    /// `ValueError` if it would have to be negative.
    pub fn pad_to_denominator(&self, denominator: &Value, outcome: &Value) -> Result<Data> {
        let adjustment = denominator.binary(BinOp::Sub, &Value::Int(self.den()))?;
        let mut data = self.map_quantities(Ok)?;
        let position = data.iter().position(|(o, _)| o == outcome);
        let current = position.map_or(Value::from(0), |i| data[i].1.clone());
        let new_quantity = current.binary(BinOp::Add, &adjustment)?;
        if new_quantity.binary(BinOp::Gt, &Value::from(0))?.truthy()? {
            match position {
                Some(i) => data[i].1 = new_quantity,
                None => data.push((outcome.clone(), new_quantity)),
            }
        } else if new_quantity == Value::from(0) {
            match position {
                Some(i) => {
                    data.remove(i);
                }
                None => return Err(Error::Key(format!("{outcome}"))),
            }
        } else {
            return value_error(format!(
                "Padding to denominator of {denominator} would require a negative quantity of \
                 {new_quantity} for {outcome}"
            ));
        }
        Ok(data)
    }

    /// Multiplies all quantities to reach the target denominator; `ValueError` if this cannot be achieved using
    /// an integer scaling.
    pub fn multiply_to_denominator(&self, denominator: &Value) -> Result<Option<Data>> {
        let den = Value::Int(self.den());
        if denominator.binary(BinOp::Mod, &den)?.truthy()? {
            return value_error(
                "Target denominator is not an integer factor of the current denominator.",
            );
        }
        self.multiply_quantities(&denominator.binary(BinOp::FloorDiv, &den)?)
    }

    /// `self.outcomes()[bisect.bisect_right(self.quantities('<='), r)]`: the outcome `Population.sample` picks for
    /// `r = random.randrange(self.denominator())` (the draw stays with the caller, whose generator it is).
    pub fn sample_at(&self, r: &Quantity) -> Result<Value> {
        let index = self.quantities_le().partition_point(|q| q <= r);
        match self.items().get(index) {
            Some((outcome, _)) => Ok(outcome.clone()),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `self.hash_key <op> other.hash_key` for `<`, `<=`, `>`, `>=`: `(Die, tuple(items))` compared as Python
    /// compares tuples. The classes are the same object, so it comes down to the items: the first `(outcome,
    /// quantity)` pair that differs (by `==`) decides, by `op` on the outcomes if those differ (which may raise),
    /// else on the quantities; if one is a prefix of the other, by `op` on the lengths. Like Python's tuple
    /// comparison, it returns what the deciding comparison returned: not a bool for vector outcomes.
    pub fn hash_key_compare(&self, other: &Die<Value>, op: BinOp) -> Result<Value> {
        fn cmp<T: PartialOrd>(op: BinOp, a: T, b: T) -> bool {
            match op {
                BinOp::Lt => a < b,
                BinOp::Le => a <= b,
                BinOp::Gt => a > b,
                _ => a >= b,
            }
        }
        for ((oa, qa), (ob, qb)) in self.items().iter().zip(other.items()) {
            // Python's tuple comparison skips items that are `==` (or identical, which dict equality stands in
            // for): a `Symbols` with only zero counts is `== ()`, though it is no dict key equal to it.
            if oa != ob && !oa.eq_op(ob, false)?.truthy()? {
                return oa.binary(op, ob);
            }
            if qa != qb {
                return Ok(Value::Bool(cmp(op, qa, qb)));
            }
        }
        Ok(Value::Bool(cmp(op, self.len(), other.len())))
    }
}
