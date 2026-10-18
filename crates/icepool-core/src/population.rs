//! Port of icepool `population/base.py`: quantities and probabilities (with comparators), `nearest`, quantiles,
//! the scalar and joint statistics, `marginals`, `group_by`, and quantity management.
//!
//! What only needs the order of the outcomes is generic over `OutcomeOrder`. The statistics do arithmetic on the
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

use crate::error::{invalid_argument, Error, Result};
use crate::function::sorted_union;
use crate::outcome::{infallible, ErrInto, Outcome, OutcomeOrder};
use crate::value::builtins::{
    getattr, getitem, math_log, math_sqrt, py_len, py_sum, try_fraction, Index,
};
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
    invalid_argument(format!("Invalid comparison {c}"))
}

/// `bisect`'s binary search: the first index whose element does not `go_right`, probing midpoints as CPython's
/// `_bisect` does (the comparisons may raise, and may not be a consistent order).
fn bisect<T, E>(
    a: &[T],
    go_right: impl Fn(&T) -> std::result::Result<bool, E>,
) -> std::result::Result<usize, E> {
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

/// `data[outcome] = quantity` in a `dict`'s items: an existing key keeps its place (and its spelling), a new one
/// goes last.
fn set_item(data: &mut Data, position: Option<usize>, outcome: &Value, quantity: Value) {
    match position {
        Some(i) => data[i].1 = quantity,
        None => data.push((outcome.clone(), quantity)),
    }
}

impl<O: OutcomeOrder> Die<O> {
    /// Python's `quantities(comparison)`: the quantities fitting the comparison for each outcome in sorted order:
    /// `'<='` gives the CDF. ([`Die::quantities`] is the plain iterator of the typed API.)
    ///
    /// `'>='` accumulates `values()[:-1]` downwards from the denominator, so an empty die has one: `(0,)`.
    pub fn quantities_by(&self, comparison: Comparison) -> Vec<Quantity> {
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
            Comparison::Lt => complement(self.quantities_by(Comparison::Ge)),
            Comparison::Gt => complement(self.quantities_by(Comparison::Le)),
        }
    }

    /// The index of the nearest outcome on the given side (`nearest`), or `None`; `Ok(Some((i, true)))` if
    /// `outcome in self` answered (Python then returns `outcome` itself).
    fn nearest_index(
        &self,
        nearest: Nearest,
        outcome: &O,
    ) -> std::result::Result<Option<(usize, bool)>, O::Incomparable> {
        let items = self.items();
        // bisect_left compares `a[mid] < x`, bisect_right `x < a[mid]`.
        let left = || bisect(items, |(o, _)| o.try_lt(outcome));
        let right = || bisect(items, |(o, _)| Ok(!outcome.try_lt(o)?));
        let below = |i: usize| i.checked_sub(1).map(|i| (i, false));
        let above = |i: usize| (i < items.len()).then_some((i, false));
        Ok(match nearest {
            Nearest::Le => match self.position(outcome) {
                Some(i) => Some((i, true)),
                None => below(right()?),
            },
            Nearest::Lt => below(left()?),
            Nearest::Ge => match self.position(outcome) {
                Some(i) => Some((i, true)),
                None => above(left()?),
            },
            Nearest::Gt => above(right()?),
        })
    }

    /// The nearest outcome in this population fitting the comparison (`'<='`, `'<'`, `'>='`, `'>'`), or `None` if
    /// there is no such outcome. For `'<='` and `'>='`, an outcome that is in the population is returned as given.
    /// `ValueError` for `'=='` and `'!='`.
    pub fn try_nearest(&self, comparison: Comparison, outcome: &O) -> Result<Option<O>> {
        let nearest = match comparison {
            Comparison::Le => Nearest::Le,
            Comparison::Lt => Nearest::Lt,
            Comparison::Ge => Nearest::Ge,
            Comparison::Gt => Nearest::Gt,
            Comparison::Eq | Comparison::Ne => return invalid_argument("Invalid comparison"),
        };
        Ok(match self.nearest_index(nearest, outcome).err_into()? {
            None => None,
            Some((_, true)) => Some(outcome.clone()),
            Some((i, false)) => Some(self.items()[i].0.clone()),
        })
    }

    /// The total quantity fitting a comparison to a single outcome.
    pub fn try_quantity(
        &self,
        comparison: Comparison,
        outcome: &O,
    ) -> std::result::Result<Quantity, O::Incomparable> {
        let get = || {
            self.position(outcome)
                .map_or(Quantity::ZERO, |i| self.items()[i].1.clone())
        };
        // `_cumulative_quantities[threshold]` of the nearest outcome below.
        let cumulative = |nearest| -> std::result::Result<Quantity, O::Incomparable> {
            Ok(match self.nearest_index(nearest, outcome)? {
                None => Quantity::ZERO,
                Some((i, _)) => self.cumulative_quantities()[i].clone(),
            })
        };
        Ok(match comparison {
            Comparison::Eq => get(),
            Comparison::Ne => self.denominator() - get(),
            Comparison::Le => cumulative(Nearest::Le)?,
            Comparison::Lt => cumulative(Nearest::Lt)?,
            Comparison::Ge => self.denominator() - cumulative(Nearest::Lt)?,
            Comparison::Gt => self.denominator() - cumulative(Nearest::Le)?,
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
            .ok_or_else(|| Error::InvalidArgument("max() iterable argument is empty".into()))
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
            .map(|(k, items)| Ok((k, Die::from_dict(items).err_into()?)))
            .collect()
    }
}

/// Which nearest outcome `nearest` looks for: `'<='`, `'<'`, `'>='` or `'>'`.
#[derive(Clone, Copy)]
enum Nearest {
    Le,
    Lt,
    Ge,
    Gt,
}

/// The typed API: an [`Outcome`]'s comparisons cannot fail, so neither can these.
impl<O: Outcome> Die<O> {
    /// The total quantity fitting a comparison to a single outcome.
    pub fn quantity(&self, comparison: Comparison, outcome: &O) -> Quantity {
        infallible(self.try_quantity(comparison, outcome))
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
        let result = self.fraction(self.try_quantity(comparison, outcome)?)?;
        if percent {
            result.binary(BinOp::Mul, &Value::Float(100.0))
        } else {
            Ok(result)
        }
    }

    /// The total probabilities fitting the comparison for each outcome in sorted order (`'<='` gives the CDF).
    /// With `percent`, each is `100.0 * x`.
    pub fn probabilities(&self, comparison: Comparison, percent: bool) -> Result<Vec<Value>> {
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

    /// The outcome `n / d` of the way through the CDF, taking the greater in case of a tie.
    pub fn quantile_high(&self, n: &Value, d: &Value) -> Result<Value> {
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
                None => return Err(Error::MissingKey(format!("{outcome}"))),
            }
        } else {
            return invalid_argument(format!(
                "Padding to denominator of {denominator} would require a negative quantity of \
                 {new_quantity} for {outcome}"
            ));
        }
        Ok(data)
    }

    /// `Population.append(outcome, quantity)` up to `self._new_type(data)`: the items of `data = Counter(self)`
    /// after `data[outcome] = max(data[outcome] + quantity, 0)`, an outcome not there yet added last (as the
    /// `dict` does). `quantity` may be negative, which removes quantity (but not below zero); it is any Python
    /// number, and `Die(data)` / `Deck(data)` (the caller's) drops the zero quantities.
    pub fn append(&self, outcome: &Value, quantity: &Value) -> Result<Data> {
        let mut data = self.map_quantities(Ok)?;
        // `data[outcome]`: a `Counter` gives 0 for a missing key.
        let position = data.iter().position(|(o, _)| o == outcome);
        let current = position.map_or(Value::from(0), |i| data[i].1.clone());
        let mut new_quantity = current.binary(BinOp::Add, quantity)?;
        // `max(x, 0)` keeps `x` unless `0 > x`.
        if Value::from(0).binary(BinOp::Gt, &new_quantity)?.truthy()? {
            new_quantity = Value::from(0);
        }
        set_item(&mut data, position, outcome, new_quantity);
        Ok(data)
    }

    /// `Population.remove(outcome, quantity)` up to `self._new_type(data)`. Without a quantity all of that
    /// outcome is removed (`data[outcome] = 0`); with one, it is `self.append(outcome, -quantity)`, so a negative
    /// quantity adds instead.
    pub fn remove(&self, outcome: &Value, quantity: Option<&Value>) -> Result<Data> {
        match quantity {
            Some(q) => self.append(outcome, &q.unary(UnOp::Neg)?),
            None => {
                let mut data = self.map_quantities(Ok)?;
                let position = data.iter().position(|(o, _)| o == outcome);
                set_item(&mut data, position, outcome, Value::from(0));
                Ok(data)
            }
        }
    }

    /// `Population.to_one_hot(outcomes)` given, for each outcome of this population, `outcomes.index(outcome)` if
    /// `outcome in outcomes` (else `None`; the caller asks its sequence, whose `in` and `index` are its own) and
    /// `len(outcomes)`: each outcome becomes a `Vector` of `len` bools, `True` at that index, and equal vectors
    /// add up their quantities (`data[Vector(value)] += quantity`). The index is a Python list index: a negative
    /// one counts from the end, and one out of range is `IndexError`. The caller builds `Die(data)` /
    /// `Deck(data)` of the result (its keys are distinct; zero quantities drop out there).
    pub fn one_hot_items(
        &self,
        len: usize,
        positions: &[Option<i64>],
    ) -> Result<Vec<(Value, Quantity)>> {
        let mut data: Vec<(Value, Quantity)> = Vec::new();
        let mut index: FxHashMap<Value, usize> = FxHashMap::default();
        for ((_, quantity), position) in self.items().iter().zip(positions) {
            // `[False] * len(outcomes)`: `len` may be that of a huge sequence (`range(2**40)`).
            let mut value =
                crate::memory::try_repeat(Value::Bool(false), Some(len), "[False] * len")?;
            if let Some(i) = *position {
                let i = if i < 0 { i + len as i64 } else { i };
                match usize::try_from(i).ok().filter(|&i| i < len) {
                    Some(i) => value[i] = Value::Bool(true),
                    None => {
                        return Err(Error::IndexOutOfRange(
                            "list assignment index out of range".into(),
                        ))
                    }
                }
            }
            let key = Value::Vector(crate::value::Vector::new(value));
            match index.get(&key) {
                Some(&i) => data[i].1 += quantity,
                None => {
                    index.insert(key.clone(), data.len());
                    data.push((key, quantity.clone()));
                }
            }
        }
        Ok(data)
    }

    /// Multiplies all quantities to reach the target denominator; `ValueError` if this cannot be achieved using
    /// an integer scaling.
    pub fn multiply_to_denominator(&self, denominator: &Value) -> Result<Option<Data>> {
        let den = Value::Int(self.den());
        if denominator.binary(BinOp::Mod, &den)?.truthy()? {
            return invalid_argument(
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
            None => Err(Error::IndexOutOfRange("tuple index out of range".into())),
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

/// The guard of `_Marginals.__getattr__` and `_GroupBy.__getattr__`: `if key[0] == '_': raise AttributeError(key)`
/// (and `key[0]` of an empty name raises IndexError).
pub fn check_public_attr(name: &str) -> Result<()> {
    match name.chars().next() {
        None => Err(Error::IndexOutOfRange("string index out of range".into())),
        Some('_') => Err(Error::MissingAttribute(name.to_string())),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_hot(positions: &[Option<i64>]) -> Result<Vec<(Value, Quantity)>> {
        let die = Die::from_dict(
            (1..=positions.len() as i64)
                .map(|o| (Value::from(o), Quantity::ONE))
                .collect(),
        )
        .unwrap();
        die.one_hot_items(3, positions)
    }

    fn hot(index: Option<usize>) -> Value {
        let mut v = vec![Value::Bool(false); 3];
        if let Some(i) = index {
            v[i] = Value::Bool(true);
        }
        Value::Vector(crate::value::Vector::new(v))
    }

    #[test]
    fn one_hot_positions_are_list_indices() {
        // A host sequence's `index` may answer anything: a negative index counts from the end, as Python's
        // `value[i] = True` does, and equal vectors merge.
        assert_eq!(
            one_hot(&[Some(0), Some(-1), Some(2), None]).unwrap(),
            vec![
                (hot(Some(0)), Quantity::ONE),
                (hot(Some(2)), Quantity::from(2u8)),
                (hot(None), Quantity::ONE),
            ]
        );
        assert_eq!(
            one_hot(&[Some(-3)]).unwrap(),
            vec![(hot(Some(0)), Quantity::ONE)]
        );
        for out_of_range in [3, -4] {
            assert!(matches!(
                one_hot(&[Some(out_of_range)]),
                Err(Error::IndexOutOfRange(_))
            ));
        }
    }

    // The statistics on dice of `Value`s. Each expected string is `repr()` of Python icepool's answer, which also
    // checks its type: `7` is an int, `7.0` a float, `Fraction(35, 6)` a Fraction.

    fn value_die(outcomes: impl IntoIterator<Item = i64>) -> Die<Value> {
        let mut data = crate::counts::OrderedMerge::default();
        for o in outcomes {
            data.add(Value::from(o), Quantity::ONE);
        }
        Die::from_dict(data.into_items()).unwrap()
    }

    fn d(sides: i64) -> Die<Value> {
        value_die(1..=sides)
    }

    fn two(sides: i64) -> Die<Value> {
        d(sides)
            .binary_operator(&d(sides), |a, b| a.binary(BinOp::Add, b))
            .unwrap()
    }

    fn py(v: Result<Value>) -> String {
        v.unwrap().to_string()
    }

    #[test]
    fn moments() {
        // `(d(6) + d(6)).mean()`, `.variance()`; `d(6).standard_deviation()`
        assert_eq!(py(two(6).mean()), "Fraction(7, 1)");
        assert_eq!(py(two(6).variance()), "Fraction(35, 6)");
        assert_eq!(py(d(6).standard_deviation()), "1.707825127659933");
        // `(d(6) + d(4)).skewness()`, `d(6).excess_kurtosis()`
        let d6_d4 = d(6)
            .binary_operator(&d(4), |a, b| a.binary(BinOp::Add, b))
            .unwrap();
        assert_eq!(py(d6_d4.skewness()), "0.0");
        assert_eq!(py(d(6).excess_kurtosis()), "-1.2685714285714282");
        // `d(4).entropy()` (base 2)
        assert_eq!(py(d(4).entropy(&Value::from(2))), "2.0");
    }

    #[test]
    fn modes_and_quantiles() {
        // `(d(4) + d(4)).modal_quantity()`, `.mode()`
        assert_eq!(two(4).modal_quantity().unwrap(), Quantity::from(4u8));
        assert_eq!(two(4).mode(), [Value::from(5)]);
        // `(d(6) + d(6)).quantile_low(1, 2)`, `.quantile_high(1, 2)`, `.median()`: the median averages the two.
        let (one, half) = (Value::from(1), Value::from(2));
        assert_eq!(py(two(6).quantile_low(&one, &half)), "7");
        assert_eq!(py(two(6).quantile_high(&one, &half)), "7");
        assert_eq!(py(two(6).quantile(&one, &half)), "7.0");
        // `d(6).quantile(1, 3)` is 2.5, between quantile_low 2 and quantile_high 3.
        let third = Value::from(3);
        assert_eq!(py(d(6).quantile_low(&one, &third)), "2");
        assert_eq!(py(d(6).quantile_high(&one, &third)), "3");
        assert_eq!(py(d(6).quantile(&one, &third)), "2.5");
    }

    #[test]
    fn probabilities_and_nearest() {
        // `d(6).probability('>=', 5)`, and with `percent=True`
        let five = Value::from(5);
        assert_eq!(
            py(d(6).probability(Comparison::Ge, &five, false)),
            "Fraction(1, 3)"
        );
        assert_eq!(
            py(d(6).probability(Comparison::Ge, &five, true)),
            "33.33333333333333"
        );
        // `d(6).nearest('<=', 0)`, `d(6).nearest('<=', 3.5)`, `d(6).nearest('>', 3)`
        let nearest = |c, x: Value| d(6).try_nearest(c, &x).unwrap();
        assert_eq!(nearest(Comparison::Le, Value::from(0)), None);
        assert_eq!(
            nearest(Comparison::Le, Value::Float(3.5)),
            Some(Value::from(3))
        );
        assert_eq!(
            nearest(Comparison::Gt, Value::from(3)),
            Some(Value::from(4))
        );
        assert!(matches!(
            d(6).try_nearest(Comparison::Eq, &Value::from(3)),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn distances_between_dice() {
        // `d(6).kolmogorov_smirnov(d(4))`, `d(6).cramer_von_mises(d(4))`
        assert_eq!(py(d(6).kolmogorov_smirnov(&d(4))), "Fraction(1, 3)");
        assert_eq!(py(d(6).cramer_von_mises(&d(4))), "Fraction(17, 72)");
    }

    #[test]
    fn group_by_keeps_first_appearance_order() {
        // `d(6).group_by(lambda x: x % 3)`: keys 1, 2, 0 in order of first appearance.
        let keys = (1..=6).map(|x| x % 3).collect();
        let groups = d(6).group_by(keys).unwrap();
        let keys: Vec<i64> = groups.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, [1, 2, 0]);
        assert_eq!(groups[2].1, value_die([3, 6]));
    }
}
