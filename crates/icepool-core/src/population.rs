//! Port of icepool `population/base.py`, the part that only needs the order of the outcomes: quantities (with
//! comparators), `nearest`, modes and `group_by`, generic over `OutcomeOrder`.
//!
//! The exact probabilities and statistics of typed dice are in `crate::statistics`; those of `Die<Value>` with
//! Python's types (`Fraction` or `float`, `percent`), the quantiles and the quantity management are the
//! extension traits `value::PyStatistics` and `value::PyDie`. Methods that make a new population
//! (`multiply_quantities`, `pad_to_denominator`, ...) return the `dict` Python passes to `Die(...)`
//! (`value::Data`), or `None` where Python returns `self`: the quantities may be any Python number, and
//! `Die(dict)`'s checks belong to the caller (the program layer or the facade).

use std::hash::Hash;

use rustc_hash::FxHashMap;

use crate::error::{invalid_argument, Error, Result};
use crate::outcome::{infallible, ErrInto, Outcome, OutcomeOrder};
use crate::{Die, Quantity, Value};

/// The comparisons `quantity`, `quantities`, `probability`, `probabilities` and `nearest` accept.
///
/// ```
/// use icepool_core::{d, Comparison, Quantity};
/// // Python's `d6.quantity('<=', 3)` and `d6.quantity('!=', 3)`.
/// assert_eq!(d(6).quantity(Comparison::Le, &3), Quantity::from(3u8));
/// assert_eq!(d(6).quantity(Comparison::Ne, &3), Quantity::from(5u8));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// `'=='`
    Eq,
    /// `'!='`
    Ne,
    /// `'<='`
    Le,
    /// `'<'`
    Lt,
    /// `'>='`
    Ge,
    /// `'>'`
    Gt,
}

impl Comparison {
    /// The comparison a Python argument names (`match comparison: case '==': ...`); `ValueError` otherwise.
    ///
    /// ```
    /// use icepool_core::{Comparison, Error, Value};
    /// assert_eq!(Comparison::parse(&Value::str(">=")).unwrap(), Comparison::Ge);
    /// // Python raises `ValueError`.
    /// assert!(matches!(Comparison::parse(&Value::str("=<")), Err(Error::InvalidArgument(_))));
    /// ```
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
pub(crate) fn bisect<T, E>(
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
pub(crate) fn int(q: &Quantity) -> Value {
    Value::Int(q.clone().into())
}

impl<O: OutcomeOrder> Die<O> {
    /// Python's `quantities(comparison)`: the quantities fitting the comparison for each outcome in sorted order:
    /// `'<='` gives the CDF. ([`Die::quantities`] is the plain iterator of the typed API.)
    ///
    /// `'>='` accumulates `values()[:-1]` downwards from the denominator, so an empty die has one: `(0,)`.
    ///
    /// ```
    /// use icepool_core::{Comparison, Die, Quantity};
    /// let die = Die::from_iter([1, 2, 2, 3]);
    /// let quantities = |c| die.quantities_by(c).iter().map(Quantity::to_string).collect::<Vec<_>>();
    /// // Python's `die.quantities('<=')` is `(1, 3, 4)`, `'>'` gives `(3, 1, 0)`.
    /// assert_eq!(quantities(Comparison::Le), ["1", "3", "4"]);
    /// assert_eq!(quantities(Comparison::Gt), ["3", "1", "0"]);
    /// // `Die([]).quantities('>=')` is `(0,)`.
    /// assert_eq!(Die::<i64>::from_iter([]).quantities_by(Comparison::Ge), [Quantity::ZERO]);
    /// ```
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
    ///
    /// ```
    /// use icepool_core::{d, Comparison};
    /// // Python's `d6.nearest('<', 4)` is 3, `d6.nearest('>', 6)` is `None`.
    /// assert_eq!(d(6).try_nearest(Comparison::Lt, &4).unwrap(), Some(3));
    /// assert_eq!(d(6).try_nearest(Comparison::Gt, &6).unwrap(), None);
    /// assert!(d(6).try_nearest(Comparison::Eq, &3).is_err()); // ValueError: Invalid comparison ==
    /// ```
    pub fn try_nearest(&self, comparison: Comparison, outcome: &O) -> Result<Option<O>> {
        let nearest = match comparison {
            Comparison::Le => Nearest::Le,
            Comparison::Lt => Nearest::Lt,
            Comparison::Ge => Nearest::Ge,
            Comparison::Gt => Nearest::Gt,
            // Python's message names the comparison.
            Comparison::Eq => return invalid_argument("Invalid comparison =="),
            Comparison::Ne => return invalid_argument("Invalid comparison !="),
        };
        Ok(match self.nearest_index(nearest, outcome).err_into()? {
            None => None,
            Some((_, true)) => Some(outcome.clone()),
            Some((i, false)) => Some(self.items()[i].0.clone()),
        })
    }

    /// The total quantity fitting a comparison to a single outcome.
    ///
    /// ```
    /// use icepool_core::{d, Comparison, Quantity};
    /// // Python's `d6.quantity('>', 4)` is 2. An `Outcome`'s comparisons cannot fail.
    /// assert_eq!(d(6).try_quantity(Comparison::Gt, &4), Ok(Quantity::from(2u8)));
    /// ```
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
    ///
    /// ```
    /// use icepool_core::{Die, Quantity};
    /// // Python's `Die([1, 2, 2, 3]).quantity_where(lambda x: x > 1)` is 3.
    /// let die = Die::from_iter([1, 2, 2, 3]);
    /// assert_eq!(die.quantity_where(&[false, true, true]), Quantity::from(3u8));
    /// ```
    pub fn quantity_where(&self, selected: &[bool]) -> Quantity {
        self.items()
            .iter()
            .zip(selected)
            .filter(|(_, s)| **s)
            .map(|((_, q), _)| q)
            .sum()
    }

    /// The highest quantity of any single outcome (`max(self.quantities())`: `ValueError` for an empty die).
    ///
    /// ```
    /// use icepool_core::{Die, Quantity};
    /// assert_eq!(Die::from_iter([1, 2, 2, 3]).modal_quantity().unwrap(), Quantity::from(2u8));
    /// // Python: `ValueError: max() iterable argument is empty`.
    /// assert!(Die::<i64>::from_iter([]).modal_quantity().is_err());
    /// ```
    pub fn modal_quantity(&self) -> Result<Quantity> {
        self.items()
            .iter()
            .map(|(_, q)| q)
            .max()
            .cloned()
            .ok_or_else(|| Error::InvalidArgument("max() iterable argument is empty".into()))
    }

    /// The most common outcome(s), lowest to highest. An empty die has none (and never asks `modal_quantity`).
    ///
    /// ```
    /// use icepool_core::Die;
    /// // Python's `Die([1, 1, 2, 3, 3]).mode()` is `(1, 3)`.
    /// assert_eq!(Die::from_iter([1, 1, 2, 3, 3]).mode(), [1, 3]);
    /// assert!(Die::<i64>::from_iter([]).mode().is_empty());
    /// ```
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

    /// `self.outcomes()[bisect.bisect_right(self.quantities('<='), r)]`: the outcome `Population.sample` picks for
    /// `r = random.randrange(self.denominator())` (the draw stays with the caller, whose generator it is).
    /// `IndexError` for an `r` beyond the denominator.
    ///
    /// ```
    /// use icepool_core::{d, Quantity};
    /// // Draws 0..=5 of a d6 pick 1..=6.
    /// assert_eq!(d(6).sample_at(&Quantity::ZERO).unwrap(), 1);
    /// assert_eq!(d(6).sample_at(&Quantity::from(5u8)).unwrap(), 6);
    /// assert!(d(6).sample_at(&Quantity::from(6u8)).is_err()); // IndexError
    /// ```
    pub fn sample_at(&self, r: &Quantity) -> Result<O> {
        let index = self.quantities_le().partition_point(|q| q <= r);
        match self.items().get(index) {
            Some((outcome, _)) => Ok(outcome.clone()),
            None => Err(Error::IndexOutOfRange("tuple index out of range".into())),
        }
    }

    /// `group_by(key_map)` given each outcome's key: for each distinct key (by `==`, in order of first
    /// appearance), the die of the outcomes with that key (`Die(dict)`, so zero quantities drop out).
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// // Python's `d6.group_by(lambda x: x % 2)` is `{1: Die([1, 3, 5]), 0: Die([2, 4, 6])}`.
    /// let keys = d(6).outcomes().map(|x| x % 2).collect();
    /// let groups = d(6).group_by(keys).unwrap();
    /// assert_eq!(groups, [(1, Die::from_iter([1, 3, 5])), (0, Die::from_iter([2, 4, 6]))]);
    /// ```
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
    ///
    /// ```
    /// use icepool_core::{d, Comparison, Quantity};
    /// // Python's `d6.quantity('<', 1)` is 0, `d6.quantity('>=', 3)` is 4.
    /// assert_eq!(d(6).quantity(Comparison::Lt, &1), Quantity::ZERO);
    /// assert_eq!(d(6).quantity(Comparison::Ge, &3), Quantity::from(4u8));
    /// ```
    pub fn quantity(&self, comparison: Comparison, outcome: &O) -> Quantity {
        infallible(self.try_quantity(comparison, outcome))
    }
}

/// The guard of `_Marginals.__getattr__` and `_GroupBy.__getattr__`: `if key[0] == '_': raise AttributeError(key)`
/// (and `key[0]` of an empty name raises IndexError).
///
/// ```
/// use icepool_core::value::check_public_attr;
/// assert!(check_public_attr("mean").is_ok());
/// assert!(check_public_attr("_private").is_err());
/// assert!(check_public_attr("").is_err());
/// ```
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
    use crate::value::{BinOp, PyDie, PyStatistics};

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
