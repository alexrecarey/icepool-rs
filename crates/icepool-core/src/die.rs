//! Port of icepool `population/die.py`: sampling with replacement, quantities are weights.

use std::sync::Arc;

use crate::counts::{Counts, OrderedMerge};
use crate::creation_args::{expand_args_for_die, DieArg};
use crate::error::{value_error, Error, Result};
use crate::outcome::{Outcome, SumOutcome};
use crate::Quantity;

/// Dice are immutable; methods return new dice. Cloning is cheap (the items are shared).
///
/// It's also possible to have "empty" dice with no outcomes at all, though these have little use other than being
/// sentinel values.
///
/// Equality and hashing are structural, as icepool's `Die.equals` / `hash_key`: the same outcomes (by `==`) with
/// the same quantities. A 2:2 coin is not equal to a 1:1 coin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Die<O> {
    data: Arc<Counts<O>>,
}

impl<O: Outcome> Die<O> {
    /// `Die(outcomes, times)` after `itemize`: one quantity multiplier per element of `outcomes`.
    ///
    /// Outcomes with zero total weight are omitted; dice among the outcomes are mixed in, scaled by
    /// `weighted_lcm` so that the overall denominator is minimised while preserving the relative weighting.
    pub fn new(outcomes: Vec<DieArg<O>>, times: Vec<Quantity>) -> Result<Self> {
        if outcomes.len() != times.len() {
            return value_error(format!(
                "The number of times ({}) must equal the number of keys ({}).",
                times.len(),
                outcomes.len()
            ));
        }
        // A single die with weight 1 is returned as-is (zero quantities and all).
        if let ([DieArg::Die(die)], [t]) = (outcomes.as_slice(), times.as_slice()) {
            if t.is_one() {
                return Ok(die.clone());
            }
        }
        Ok(Self::from_counts(expand_args_for_die(&outcomes, &times)?))
    }

    /// `Die._new_raw`: wraps already-processed data.
    pub fn from_counts(data: Counts<O>) -> Self {
        Die {
            data: Arc::new(data),
        }
    }

    /// Every `(outcome, quantity)` in sorted order, zero quantities included.
    pub fn items(&self) -> &[(O, Quantity)] {
        self.data.items()
    }

    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = &O> {
        self.data.items().iter().map(|(o, _)| o)
    }

    /// The number of outcomes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Performs the unary operation on the outcomes (`Population._unary_operator`).
    ///
    /// This is used for the standard unary operators `-, +, abs, ~, round, trunc, floor, ceil` as well as the
    /// additional methods `zero, bool`. Outcomes that map to the same result add up their quantities.
    pub fn unary_operator(&self, op: impl Fn(&O) -> Result<O>) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            data.add(op(outcome)?, quantity.clone());
        }
        Self::from_mapping(data)
    }

    /// Performs the operation on pairs of outcomes (`Die.binary_operator`).
    ///
    /// By the time this is called, the other operand has already been converted to a `Die`. This is used for the
    /// standard binary operators `+, -, *, /, //, %, **, <<, >>, &, |, ^` and the standard binary comparators
    /// `<, <=, >=, >, ==, !=, cmp`. The `@` operator does NOT use this method directly.
    ///
    /// Every pair is visited (zero quantities included, so an `op` that raises on one raises here), `self` in the
    /// outer loop, as `itertools.product(self.items(), other.items())`.
    pub fn binary_operator(
        &self,
        other: &Die<O>,
        op: impl Fn(&O, &O) -> Result<O>,
    ) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome_self, quantity_self) in self.items() {
            for (outcome_other, quantity_other) in other.items() {
                data.add(
                    op(outcome_self, outcome_other)?,
                    quantity_self * quantity_other,
                );
            }
        }
        Self::from_mapping(data)
    }

    /// `Die(data)` for a dict of outcome -> quantity (`self._new_type(data)` in icepool). `itemize` and
    /// `merge_weights_lcm` see denominator 1 for every outcome, so `weighted_lcm` passes the quantities through
    /// unchanged except that zero ones drop out; then `Counts` sorts what is left.
    pub(crate) fn from_mapping(data: OrderedMerge<O>) -> Result<Self> {
        Self::from_dict(data.into_items())
    }

    /// `Die(dict)` for items whose outcomes are already distinct.
    pub fn from_dict(items: Vec<(O, Quantity)>) -> Result<Self> {
        let items = items.into_iter().filter(|(_, q)| !q.is_zero()).collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// The sum of all quantities.
    pub fn denominator(&self) -> Quantity {
        self.data.items().iter().map(|(_, q)| q).sum()
    }

    /// The least outcome (`IndexError` for an empty die).
    pub fn min_outcome(&self) -> Result<&O> {
        match self.items().first() {
            Some((o, _)) => Ok(o),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// The greatest outcome (`IndexError` for an empty die).
    pub fn max_outcome(&self) -> Result<&O> {
        match self.items().last() {
            Some((o, _)) => Ok(o),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `Die._pop_min`: a die with the min outcome removed (zero quantities kept), and the quantity of the removed
    /// outcome. Raises `IndexError` if this die has no outcome to pop.
    pub fn pop_min(&self) -> Result<(Self, Quantity)> {
        match self.items().split_first() {
            Some(((_, q), rest)) => Ok((
                Self::from_counts(Counts::from_sorted(rest.to_vec())),
                q.clone(),
            )),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `Die._pop_max`: a die with the max outcome removed, and the quantity of the removed outcome.
    pub fn pop_max(&self) -> Result<(Self, Quantity)> {
        match self.items().split_last() {
            Some(((_, q), rest)) => Ok((
                Self::from_counts(Counts::from_sorted(rest.to_vec())),
                q.clone(),
            )),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `quantities('<=')`: the cumulative quantities in sorted order (the CDF).
    pub fn quantities_le(&self) -> Vec<Quantity> {
        let mut total = Quantity::ZERO;
        self.items()
            .iter()
            .map(|(_, q)| {
                total += q;
                total.clone()
            })
            .collect()
    }

    /// `quantities('>=')`: the quantities at or above each outcome in sorted order (the survival function).
    pub fn quantities_ge(&self) -> Vec<Quantity> {
        let mut total = self.denominator();
        let mut result = Vec::with_capacity(self.len());
        for (_, q) in self.items() {
            result.push(total.clone());
            total -= q;
        }
        result
    }

    /// `quantity('>=', outcome)`: the denominator minus `quantity('<', outcome)`, which looks up the cumulative
    /// quantity of the nearest outcome below with `bisect_left`.
    pub fn quantity_ge(&self, outcome: &O) -> Result<Quantity> {
        let index = bisect(self.items(), |o| o.py_lt(outcome))?;
        let below: Quantity = self.items()[..index].iter().map(|(_, q)| q).sum();
        Ok(self.denominator() - below)
    }

    /// `quantity('<=', outcome)`: the cumulative quantity of the nearest outcome at or below (`bisect_right`).
    pub fn quantity_le(&self, outcome: &O) -> Result<Quantity> {
        let index = bisect(self.items(), |o| Ok(!outcome.py_lt(o)?))?;
        Ok(self.items()[..index].iter().map(|(_, q)| q).sum())
    }

    /// Roll this die several times and keep the lowest (`Die._lowest_single`, `rolls > 0`; the facade handles
    /// `rolls == 0`, which is `zero().simplify()`).
    pub fn lowest_single(&self, rolls: usize) -> Result<Self> {
        let cumulative = self.quantities_ge().iter().map(|x| x.pow(rolls)).collect();
        from_cumulative(self.outcomes().cloned().collect(), cumulative, true)
    }

    /// Roll this die several times and keep the highest (`Die._highest_single`, `rolls > 0`).
    pub fn highest_single(&self, rolls: usize) -> Result<Self> {
        let cumulative = self.quantities_le().iter().map(|x| x.pow(rolls)).collect();
        from_cumulative(self.outcomes().cloned().collect(), cumulative, false)
    }

    /// Rerolls the outcomes selected by `reroll` (one flag per outcome, as `items()`).
    ///
    /// `depth` is the maximum number of times to reroll; `None` (Python's `'inf'`) rerolls an unlimited number of
    /// times. If the reroll would never terminate, the result has no outcomes. The facade computes the selection
    /// (`_select_outcomes`, or `{self.min_outcome()}` when no outcomes are given) and checks that `depth` is not
    /// negative.
    pub fn reroll(&self, reroll: &[bool], depth: Option<u32>) -> Result<Self> {
        let items = self.items().iter().zip(reroll);
        let data = match depth {
            None => items
                .filter(|(_, &r)| !r)
                .map(|((o, q), _)| (o.clone(), q.clone()))
                .collect(),
            Some(depth) => {
                let total_reroll_quantity: Quantity =
                    items.clone().filter(|(_, &r)| r).map(|((_, q), _)| q).sum();
                let denominator = self.denominator();
                let total_stop_quantity = &denominator - &total_reroll_quantity;
                if total_stop_quantity.is_zero() {
                    return Err(Error::ZeroDivision(
                        "integer division or modulo by zero".into(),
                    ));
                }
                // A stopping outcome is reached after 0..=depth rerolls: the factor sums
                // den**(depth - k) * rerolled**k over those, i.e. (den**(depth+1) - rerolled**(depth+1)) / stop.
                let rerollable_factor = total_reroll_quantity.pow(depth as usize);
                let stop_factor = (denominator.pow(depth as usize + 1)
                    - &rerollable_factor * &total_reroll_quantity)
                    / total_stop_quantity;
                items
                    .map(|((o, q), &r)| {
                        let factor = if r { &rerollable_factor } else { &stop_factor };
                        (o.clone(), factor * q)
                    })
                    .collect()
            }
        };
        Self::from_dict(data)
    }

    /// Splits this die into one containing the selected outcomes and another containing the rest.
    ///
    /// The sum of the denominators of the results is equal to the denominator of this die (zero quantities drop
    /// out of both).
    pub fn split(&self, selected: &[bool]) -> Result<(Self, Self)> {
        let (sel, rest): (Vec<_>, Vec<_>) =
            self.items().iter().zip(selected).partition(|(_, &s)| s);
        let unflag =
            |v: Vec<(&(O, Quantity), &bool)>| v.into_iter().map(|(i, _)| i.clone()).collect();
        Ok((
            Self::from_dict(unflag(sel))?,
            Self::from_dict(unflag(rest))?,
        ))
    }

    /// Truncates the outcomes of this die to the given range.
    ///
    /// The endpoints are included in the result if applicable. If one of the arguments is not provided, that side
    /// will not be truncated. This effectively rerolls outcomes outside the given range. If instead you want to
    /// replace those outcomes with the nearest endpoint, use `clip()`.
    pub fn truncate(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Result<Self> {
        let items = self.items();
        // bisect_left / bisect_right, making exactly the comparisons Python's bisect makes.
        let start = match min_outcome {
            Some(x) => bisect(items, |o| o.py_lt(x))?,
            None => 0,
        };
        let stop = match max_outcome {
            Some(x) => bisect(items, |o| Ok(!x.py_lt(o)?))?,
            None => items.len(),
        };
        // `items()[start:stop]` is empty when start > stop.
        Self::from_dict(items[start..stop.max(start)].to_vec())
    }

    /// Clips the outcomes of this die to the given values.
    ///
    /// The endpoints are included in the result if applicable. If one of the arguments is not provided, that side
    /// will not be clipped. This is not the same as rerolling outcomes beyond this range; the outcome is simply
    /// adjusted to fit within the range. This will typically cause some quantity to bunch up at the endpoint(s).
    /// If you want to reroll outcomes beyond this range, use `truncate()`.
    pub fn clip(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            let new = match (min_outcome, max_outcome) {
                (Some(min), _) if outcome.py_le(min)? => min,
                (_, Some(max)) if outcome.py_ge(max)? => max,
                _ => outcome,
            };
            data.add(new.clone(), quantity.clone());
        }
        Self::from_mapping(data)
    }
}

impl<O: SumOutcome> Die<O> {
    /// Zeros all outcomes of this die (`Population.zero`). This is done by multiplying all outcomes by `0`.
    ///
    /// The result will have the same denominator. Raises `ValueError` if the zeros did not resolve to a single
    /// outcome (an empty die included).
    pub fn zero(&self) -> Result<Self> {
        let result = self.unary_operator(O::times_zero)?;
        if result.len() != 1 {
            return value_error("zero() did not resolve to a single outcome.");
        }
        Ok(result)
    }

    /// Divides all quantities by their greatest common denominator (`Die.simplify`, via `Counts.simplify`, then
    /// `Die(counts)`, which drops zero quantities).
    pub fn simplify(&self) -> Result<Self> {
        let gcd = self
            .items()
            .iter()
            .fold(Quantity::ZERO, |g, (_, q)| dashu_base::Gcd::gcd(g, q));
        if gcd <= Quantity::ONE {
            return Self::from_dict(self.items().to_vec());
        }
        Self::from_dict(
            self.items()
                .iter()
                .map(|(o, q)| (o.clone(), q / &gcd))
                .collect(),
        )
    }

    /// Roll `counts` (the items of the left die of `@`, already checked to be integers), then roll this die that
    /// many times and sum the outcomes (`Die.__matmul__`, with this die on the right).
    ///
    /// The sum is computed one at a time, with each additional item on the right, similar to `functools.reduce()`.
    /// A negative count rolls `abs(count)` times and negates the result. Each count's sum is scaled by
    /// `den**(max|count| - |count|)` so that every count's results share the denominator `den**max|count|`.
    pub fn matmul(&self, counts: &[(i64, Quantity)]) -> Result<Self> {
        let mut data = OrderedMerge::default();
        for term in self.matmul_terms(counts)? {
            for (outcome, quantity) in term {
                data.add(outcome, quantity);
            }
        }
        Self::from_mapping(data)
    }

    /// The terms `matmul` adds up, one per count in order: each outcome of the count's sum with its quantity
    /// times the count's quantity times the count's factor (zero quantities included). For facades whose counts
    /// may make non-int quantities (a Python float count), which only show once the terms are added.
    pub fn matmul_terms(&self, counts: &[(i64, Quantity)]) -> Result<Vec<Vec<(O, Quantity)>>> {
        let max_abs_die_count = match counts.iter().map(|(c, _)| c.unsigned_abs()).max() {
            Some(m) => m,
            // `max(abs(self.min_outcome()), ...)` of an empty die.
            None => return Err(Error::Index("tuple index out of range".into())),
        };
        let max_abs = usize::try_from(max_abs_die_count)
            .map_err(|_| Error::Overflow("die count too large".into()))?;
        let mut sums = SumCache::new(self);
        let denominator = self.denominator();
        let mut terms = Vec::with_capacity(counts.len());
        for (die_count, die_count_quantity) in counts {
            let abs = die_count.unsigned_abs() as usize;
            let factor = denominator.pow(max_abs - abs);
            let subresult = sums.sum_all(*die_count)?;
            terms.push(
                subresult
                    .items()
                    .iter()
                    .map(|(outcome, subresult_quantity)| {
                        (
                            outcome.clone(),
                            subresult_quantity * die_count_quantity * &factor,
                        )
                    })
                    .collect(),
            );
        }
        Ok(terms)
    }
}

/// `Die._sum_all` with its `_sum_cache`: the sum of `rolls` rolls of a die, each sum computed at most once.
///
/// Python only computes what it is asked for, so a sum that would raise (`zero()` of mixed types, `-` of a str)
/// raises only if some count needs it.
struct SumCache<'a, O> {
    die: &'a Die<O>,
    /// `positive[k - 1]` is the sum of `k` rolls.
    positive: Vec<Die<O>>,
    zero: Option<Die<O>>,
    negative: rustc_hash::FxHashMap<u64, Die<O>>,
}

impl<'a, O: SumOutcome> SumCache<'a, O> {
    fn new(die: &'a Die<O>) -> Self {
        SumCache {
            die,
            positive: Vec::new(),
            zero: None,
            negative: Default::default(),
        }
    }

    /// Roll this die `rolls` times and sum the results. If `rolls` is negative, roll the die `abs(rolls)` times
    /// and negate the result.
    fn sum_all(&mut self, rolls: i64) -> Result<Die<O>> {
        if rolls < 0 {
            let k = rolls.unsigned_abs();
            if let Some(d) = self.negative.get(&k) {
                return Ok(d.clone());
            }
            let result = self.positive_sum(k as usize)?.unary_operator(O::neg)?;
            self.negative.insert(k, result.clone());
            Ok(result)
        } else if rolls == 0 {
            if self.zero.is_none() {
                self.zero = Some(self.die.zero()?.simplify()?);
            }
            Ok(self.zero.clone().expect("just set"))
        } else {
            self.positive_sum(rolls as usize)
        }
    }

    fn positive_sum(&mut self, rolls: usize) -> Result<Die<O>> {
        if self.positive.is_empty() {
            self.positive.push(self.die.clone());
        }
        while self.positive.len() < rolls {
            // In addition to working similar to reduce(), this seems to perform better than binary split.
            let next = self
                .positive
                .last()
                .expect("not empty")
                .binary_operator(self.die, O::add)?;
            self.positive.push(next);
        }
        Ok(self.positive[rolls - 1].clone())
    }
}

/// Constructs a `Die` from a sequence of cumulative `int` quantities (`icepool.from_cumulative`).
///
/// `reverse`: iff true, both of the arguments will be reversed. This allows e.g. constructing using a survival
/// distribution. The cumulative quantities must be non-decreasing in the order used; differences of zero drop out
/// (it ends in `Die(dict)`).
pub fn from_cumulative<O: Outcome>(
    mut outcomes: Vec<O>,
    mut cumulative: Vec<Quantity>,
    reverse: bool,
) -> Result<Die<O>> {
    if reverse {
        outcomes.reverse();
        cumulative.reverse();
    }
    let mut prev = Quantity::ZERO;
    let mut d = OrderedMerge::default();
    for (outcome, quantity) in outcomes.into_iter().zip(cumulative) {
        d.add(outcome, &quantity - &prev);
        prev = quantity;
    }
    Die::from_mapping(d)
}

/// Python's `bisect` loop over sorted items: the first index whose outcome does not satisfy `go_right` (which must
/// hold for a prefix of the items). `lo` moves right while `go_right(a[mid])`.
fn bisect<O>(items: &[(O, Quantity)], go_right: impl Fn(&O) -> Result<bool>) -> Result<usize> {
    let (mut lo, mut hi) = (0, items.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if go_right(&items[mid].0)? {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}
