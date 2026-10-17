//! Port of icepool `population/die.py`: sampling with replacement, quantities are weights.

use crate::ownership::{Owners, Traverse};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use rustc_hash::{FxHashMap, FxHasher};

use crate::counts::{Counts, OrderedMerge};
use crate::creation_args::{expand_args_for_die, DieArg};
use crate::error::{value_error, Error, Result};
use crate::outcome::{infallible, ErrInto, Outcome, OutcomeArith, OutcomeOrder};
use crate::Quantity;

/// Dice are immutable; methods return new dice. Cloning is cheap (the items are shared).
///
/// It's also possible to have "empty" dice with no outcomes at all, though these have little use other than being
/// sentinel values.
///
/// Equality and hashing are structural, as icepool's `Die.equals` / `hash_key`: the same outcomes (by `==`) with
/// the same quantities. A 2:2 coin is not equal to a 1:1 coin.
#[derive(Clone, Debug)]
pub struct Die<O> {
    data: Arc<DieData<O>>,
}

/// A die's items plus what Python caches on the die object: its hash, and the dice left after popping the min or
/// max outcome (`_popped_min` / `_popped_max` are `cached_property`s). The evaluator hashes and pops the same dice
/// over and over. `index` stands in for Python's dict lookup (`outcome in self`) and `cumulative` for the
/// `_cumulative_quantities` `cached_property`: without them `quantity('<=', x)` over every outcome of a die with
/// tens of thousands of outcomes (`cdf_distance`) is quadratic. `denominator` is `_denominator`, cached for the
/// same reason.
#[derive(Debug)]
struct DieData<O> {
    counts: Counts<O>,
    hash: OnceLock<u64>,
    popped_min: OnceLock<(Die<O>, Quantity)>,
    popped_max: OnceLock<(Die<O>, Quantity)>,
    index: OnceLock<FxHashMap<O, usize>>,
    cumulative: OnceLock<Vec<Quantity>>,
    denominator: OnceLock<Quantity>,
}

/// A die holds its outcomes, and more clones of them in its caches (its popped dice and its index).
impl<O: OutcomeOrder + Traverse> Traverse for Die<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc_with(&self.data, |owners, data| {
            for (o, _) in data.counts.items() {
                o.traverse(owners);
            }
            for (die, _) in [data.popped_min.get(), data.popped_max.get()]
                .into_iter()
                .flatten()
            {
                die.traverse(owners);
            }
            for o in data.index.get().into_iter().flat_map(|index| index.keys()) {
                o.traverse(owners);
            }
        });
    }
}

impl<O: PartialEq> PartialEq for Die<O> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) || self.data.counts == other.data.counts
    }
}

impl<O: Eq> Eq for Die<O> {}

impl<O: Hash> Hash for Die<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let h = *self.data.hash.get_or_init(|| {
            let mut hasher = FxHasher::default();
            self.data.counts.hash(&mut hasher);
            hasher.finish()
        });
        state.write_u64(h);
    }
}

impl<O: OutcomeOrder> Die<O> {
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
            data: Arc::new(DieData {
                counts: data,
                hash: OnceLock::new(),
                popped_min: OnceLock::new(),
                popped_max: OnceLock::new(),
                index: OnceLock::new(),
                cumulative: OnceLock::new(),
                denominator: OnceLock::new(),
            }),
        }
    }

    /// Every `(outcome, quantity)` in sorted order, zero quantities included.
    pub fn items(&self) -> &[(O, Quantity)] {
        self.data.counts.items()
    }

    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = &O> {
        self.data.counts.items().iter().map(|(o, _)| o)
    }

    /// The number of outcomes.
    pub fn len(&self) -> usize {
        self.data.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.counts.is_empty()
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
        Self::from_mapping(data).err_into()
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
            crate::interrupt::poll()?;
            for (outcome_other, quantity_other) in other.items() {
                data.add(
                    op(outcome_self, outcome_other)?,
                    quantity_self * quantity_other,
                );
            }
        }
        Self::from_mapping(data).err_into()
    }

    /// `Die(data)` for a dict of outcome -> quantity (`self._new_type(data)` in icepool). `itemize` and
    /// `merge_weights_lcm` see denominator 1 for every outcome, so `weighted_lcm` passes the quantities through
    /// unchanged except that zero ones drop out; then `Counts` sorts what is left.
    pub(crate) fn from_mapping(data: OrderedMerge<O>) -> std::result::Result<Self, O::Incomparable> {
        Self::from_dict(data.into_items())
    }

    /// `Die(dict)` for items whose outcomes are already distinct.
    pub fn from_dict(items: Vec<(O, Quantity)>) -> std::result::Result<Self, O::Incomparable> {
        let items = items.into_iter().filter(|(_, q)| !q.is_zero()).collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// Whether the two are clones of one die (not merely equal): `is` on Python's die objects.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    /// The sum of all quantities.
    pub fn denominator(&self) -> Quantity {
        // `_denominator` is a `cached_property` in Python too.
        self.data
            .denominator
            .get_or_init(|| self.data.counts.items().iter().map(|(_, q)| q).sum())
            .clone()
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
            Some(((_, q), rest)) => Ok(self
                .data
                .popped_min
                .get_or_init(|| {
                    (
                        Self::from_counts(Counts::from_sorted(rest.to_vec())),
                        q.clone(),
                    )
                })
                .clone()),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `Die._pop_max`: a die with the max outcome removed, and the quantity of the removed outcome.
    pub fn pop_max(&self) -> Result<(Self, Quantity)> {
        match self.items().split_last() {
            Some(((_, q), rest)) => Ok(self
                .data
                .popped_max
                .get_or_init(|| {
                    (
                        Self::from_counts(Counts::from_sorted(rest.to_vec())),
                        q.clone(),
                    )
                })
                .clone()),
            None => Err(Error::Index("tuple index out of range".into())),
        }
    }

    /// `quantities('<=')`: the cumulative quantities in sorted order (the CDF).
    pub fn quantities_le(&self) -> Vec<Quantity> {
        self.cumulative_quantities().to_vec()
    }

    /// The cumulative quantities in sorted order, computed once per die (`_cumulative_quantities`).
    pub fn cumulative_quantities(&self) -> &[Quantity] {
        self.data.cumulative.get_or_init(|| {
            let mut total = Quantity::ZERO;
            self.items()
                .iter()
                .map(|(_, q)| {
                    total += q;
                    total.clone()
                })
                .collect()
        })
    }

    /// `outcome in self`: the position of an equal outcome (by `==`, which `Hash` agrees with), through a map built
    /// once per die. The first of equal outcomes wins, as a linear search would.
    pub fn position(&self, outcome: &O) -> Option<usize> {
        let index = self.data.index.get_or_init(|| {
            let mut index = FxHashMap::default();
            for (i, (o, _)) in self.items().iter().enumerate() {
                index.entry(o.clone()).or_insert(i);
            }
            index
        });
        index.get(outcome).copied()
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
    pub fn try_quantity_ge(&self, outcome: &O) -> std::result::Result<Quantity, O::Incomparable> {
        let index = bisect(self.items(), |o| o.try_lt(outcome))?;
        let below: Quantity = self.items()[..index].iter().map(|(_, q)| q).sum();
        Ok(self.denominator() - below)
    }

    /// `quantity('<=', outcome)`: the cumulative quantity of the nearest outcome at or below (`bisect_right`).
    pub fn try_quantity_le(&self, outcome: &O) -> std::result::Result<Quantity, O::Incomparable> {
        let index = bisect(self.items(), |o| Ok(!outcome.try_lt(o)?))?;
        Ok(self.items()[..index].iter().map(|(_, q)| q).sum())
    }

    /// Roll this die several times and keep the lowest (`Die._lowest_single`, `rolls > 0`; the facade handles
    /// `rolls == 0`, which is `zero().simplify()`).
    pub fn lowest_single(&self, rolls: usize) -> Result<Self> {
        // Every cumulative quantity is at most the denominator: `denominator**rolls` bounds their size.
        crate::memory::check_pow(&self.denominator(), rolls, "a quantity ** rolls")?;
        let cumulative = self.quantities_ge().iter().map(|x| x.pow(rolls)).collect();
        from_cumulative(self.outcomes().cloned().collect(), cumulative, true).err_into()
    }

    /// Roll this die several times and keep the highest (`Die._highest_single`, `rolls > 0`).
    pub fn highest_single(&self, rolls: usize) -> Result<Self> {
        // Every cumulative quantity is at most the denominator: `denominator**rolls` bounds their size.
        crate::memory::check_pow(&self.denominator(), rolls, "a quantity ** rolls")?;
        let cumulative = self.quantities_le().iter().map(|x| x.pow(rolls)).collect();
        from_cumulative(self.outcomes().cloned().collect(), cumulative, false).err_into()
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
                crate::memory::check_pow(&denominator, depth as usize + 1, "a reroll depth")?;
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
        Self::from_dict(data).err_into()
    }

    /// Splits this die into one containing the selected outcomes and another containing the rest.
    ///
    /// The sum of the denominators of the results is equal to the denominator of this die (zero quantities drop
    /// out of both).
    pub fn split(&self, selected: &[bool]) -> std::result::Result<(Self, Self), O::Incomparable> {
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
    pub fn try_truncate(
        &self,
        min_outcome: Option<&O>,
        max_outcome: Option<&O>,
    ) -> std::result::Result<Self, O::Incomparable> {
        let items = self.items();
        // bisect_left / bisect_right, making exactly the comparisons Python's bisect makes.
        let start = match min_outcome {
            Some(x) => bisect(items, |o| o.try_lt(x))?,
            None => 0,
        };
        let stop = match max_outcome {
            Some(x) => bisect(items, |o| Ok(!x.try_lt(o)?))?,
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
    pub fn try_clip(
        &self,
        min_outcome: Option<&O>,
        max_outcome: Option<&O>,
    ) -> std::result::Result<Self, O::Incomparable> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            let new = match (min_outcome, max_outcome) {
                (Some(min), _) if outcome.try_le(min)? => min,
                (_, Some(max)) if outcome.try_ge(max)? => max,
                _ => outcome,
            };
            data.add(new.clone(), quantity.clone());
        }
        Self::from_mapping(data)
    }
}

/// The typed API: an [`Outcome`]'s comparisons cannot fail, so neither can these.
impl<O: Outcome> Die<O> {
    /// A die that always rolls `outcome` (Python's `Die([outcome])`).
    ///
    /// ```
    /// use icepool_core::Die;
    /// let five = Die::single(5);
    /// assert_eq!(five.outcomes().collect::<Vec<_>>(), [&5]);
    /// ```
    pub fn single(outcome: O) -> Self {
        Self::from_counts(Counts::from_sorted(vec![(outcome, Quantity::ONE)]))
    }

    /// A die from `(outcome, quantity)` pairs (Python's `Die({outcome: quantity, ...})`). Equal outcomes add up
    /// their quantities; outcomes with zero quantity are dropped.
    ///
    /// ```
    /// use icepool_core::Die;
    /// let loaded = Die::from_weights([(1, 2u32), (2, 1), (1, 1)]);
    /// assert_eq!(loaded.quantities().map(|q| q.to_string()).collect::<Vec<_>>(), ["3", "1"]);
    /// ```
    pub fn from_weights<Q: Into<Quantity>>(items: impl IntoIterator<Item = (O, Q)>) -> Self {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in items {
            data.add(outcome, quantity.into());
        }
        infallible(Self::from_mapping(data))
    }

    /// The quantities in the order of their outcomes, zero quantities included (Python's `die.quantities()`).
    ///
    /// ```
    /// use icepool_core::{d, Quantity};
    /// assert!(d(6).quantities().all(|q| *q == Quantity::ONE));
    /// ```
    pub fn quantities(&self) -> impl ExactSizeIterator<Item = &Quantity> {
        self.items().iter().map(|(_, q)| q)
    }

    /// The die whose outcomes are `f` of this die's outcomes: equal results add up their quantities (Python's
    /// `die.map(f)` for an `f` that returns plain outcomes).
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let parity: Die<bool> = d(6).map(|&x| x % 2 == 0);
    /// assert_eq!(parity, Die::from_weights([(false, 3u32), (true, 3)]));
    /// ```
    pub fn map<P: Outcome>(&self, mut f: impl FnMut(&O) -> P) -> Die<P> {
        let mut data = OrderedMerge::default();
        for (outcome, quantity) in self.items() {
            data.add(f(outcome), quantity.clone());
        }
        infallible(Die::from_mapping(data))
    }

    /// Rolls this die and `other` and combines the two outcomes with `f`: every pair of outcomes, the quantities
    /// multiplied, equal results adding up (Python's `die.binary_operator`, for any function). The operators
    /// (`+`, `&`, ...) and the comparison methods ([`Die::lt`], ...) are this.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let highest = d(6).binary(&d(6), |a, b| *a.max(b));
    /// assert_eq!(highest.quantity_le(&1).to_string(), "1");
    /// assert_eq!(highest.denominator().to_string(), "36");
    /// ```
    pub fn binary<Q: Outcome, P: Outcome>(&self, other: &Die<Q>, mut f: impl FnMut(&O, &Q) -> P) -> Die<P> {
        infallible(self.try_binary(other, |a, b| Ok(f(a, b))))
    }

    /// [`Die::binary`] with a function that may fail: the first error is returned.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// let sum = d(6).try_binary(&Die::single(i64::MAX), |a, b| a.checked_add(*b).ok_or("overflow"));
    /// assert_eq!(sum, Err("overflow"));
    /// ```
    pub fn try_binary<Q: Outcome, P: Outcome, E>(
        &self,
        other: &Die<Q>,
        mut f: impl FnMut(&O, &Q) -> std::result::Result<P, E>,
    ) -> std::result::Result<Die<P>, E> {
        let mut data = OrderedMerge::default();
        for (outcome_self, quantity_self) in self.items() {
            for (outcome_other, quantity_other) in other.items() {
                data.add(f(outcome_self, outcome_other)?, quantity_self * quantity_other);
            }
        }
        Ok(infallible(Die::from_mapping(data)))
    }

    /// `quantity('>=', outcome)`: the total quantity of the outcomes at or above `outcome`.
    pub fn quantity_ge(&self, outcome: &O) -> Quantity {
        infallible(self.try_quantity_ge(outcome))
    }

    /// `quantity('<=', outcome)`: the total quantity of the outcomes at or below `outcome`.
    pub fn quantity_le(&self, outcome: &O) -> Quantity {
        infallible(self.try_quantity_le(outcome))
    }

    /// Truncates the outcomes of this die to the given range (see [`Die::try_truncate`]).
    pub fn truncate(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Self {
        infallible(self.try_truncate(min_outcome, max_outcome))
    }

    /// Clips the outcomes of this die to the given values (see [`Die::try_clip`]).
    pub fn clip(&self, min_outcome: Option<&O>, max_outcome: Option<&O>) -> Self {
        infallible(self.try_clip(min_outcome, max_outcome))
    }
}

impl<O: OutcomeArith> Die<O> {
    /// Zeros all outcomes of this die (`Population.zero`). This is done by multiplying all outcomes by `0`.
    ///
    /// The result will have the same denominator. Raises `ValueError` if the zeros did not resolve to a single
    /// outcome (an empty die included).
    pub fn zero(&self) -> Result<Self> {
        let result = self.unary_operator(O::try_zero)?;
        if result.len() != 1 {
            return value_error("zero() did not resolve to a single outcome.");
        }
        Ok(result)
    }

    /// `Die.equals(other, simplify=...)` with `other` a die: the same outcomes and quantities (`hash_key`s equal).
    /// Truth value does not matter. A zero-quantity outcome the other die lacks makes them unequal. With
    /// `simplify`, both are simplified first (otherwise a 2:2 coin is not `equals()` to a 1:1 coin).
    pub fn equals(&self, other: &Die<O>, simplify: bool) -> Result<bool> {
        if Arc::ptr_eq(&self.data, &other.data) {
            return Ok(true);
        }
        if simplify {
            Ok(self.simplify()? == other.simplify()?)
        } else {
            Ok(self == other)
        }
    }

    /// Divides all quantities by their greatest common denominator (`Die.simplify`, via `Counts.simplify`, then
    /// `Die(counts)`, which drops zero quantities).
    pub fn simplify(&self) -> Result<Self> {
        let gcd = self
            .items()
            .iter()
            .fold(Quantity::ZERO, |g, (_, q)| dashu_base::Gcd::gcd(g, q));
        if gcd <= Quantity::ONE {
            return Self::from_dict(self.items().to_vec()).err_into();
        }
        Self::from_dict(
            self.items()
                .iter()
                .map(|(o, q)| (o.clone(), q / &gcd))
                .collect(),
        )
        .err_into()
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
        Self::from_mapping(data).err_into()
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
        crate::memory::check_pow(&denominator, max_abs, "n @ die")?;
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

impl<'a, O: OutcomeArith> SumCache<'a, O> {
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
            let result = self.positive_sum(k as usize)?.unary_operator(O::try_neg)?;
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
                .binary_operator(self.die, O::try_add)?;
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
pub fn from_cumulative<O: OutcomeOrder>(
    mut outcomes: Vec<O>,
    mut cumulative: Vec<Quantity>,
    reverse: bool,
) -> std::result::Result<Die<O>, O::Incomparable> {
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
fn bisect<O, E>(items: &[(O, Quantity)], go_right: impl Fn(&O) -> std::result::Result<bool, E>) -> std::result::Result<usize, E> {
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

impl Die<crate::Value> {
    /// A `Die` with outcomes 1, -1, and 0 (`Die.cmp`).
    ///
    /// The quantities are equal to the positive outcome of `self > other`, `self < other`, and the remainder
    /// respectively. As in icepool, the three comparison dice are made in the order `<`, `==`, `>` (so the first
    /// that raises decides the error), and each contributes the quantity of its `True` outcome if it has one
    /// (`if True in lt: data[-1] = lt[True]`, a dict lookup, so an outcome equal to `True` such as `1` counts).
    pub fn cmp_die(&self, other: &Self) -> Result<Self> {
        use crate::value::BinOp;
        use crate::Value;
        let truth = Value::Bool(true);
        let mut data = Vec::new();
        // `None` is `==`, which has no `BinOp`.
        for (sign, op) in [(-1, Some(BinOp::Lt)), (0, None), (1, Some(BinOp::Gt))] {
            let compared = self.binary_operator(other, |a, b| match op {
                Some(op) => a.binary(op, b),
                None => a.eq_op(b, false),
            })?;
            if let Some(i) = compared.position(&truth) {
                data.push((Value::from(sign), compared.items()[i].1.clone()));
            }
        }
        Die::from_dict(data)
    }

    /// `Die.mean_time_to_sum(target)`: the mean number of rolls until the cumulative sum is greater or equal to
    /// the target (`ValueError` if this die has negative outcomes, `ZeroDivisionError` if it only rolls 0).
    ///
    /// `cache` is the die's `_mean_time_to_sum_cache` (a `cached_property` starting as `[Fraction(0)]`), the
    /// mean time for each target so far, which this extends up to `target`. Each new entry is
    /// `time_per_effect + self.reroll([0], depth='inf').map(lambda x: self.mean_time_to_sum(i - x)).mean()`:
    /// the rolls until a nonzero outcome, then the time left after it. Python's recursive call finds every
    /// smaller target in the cache; outcomes that are not ints get there as floats or Fractions, which index a
    /// list with `TypeError`, as here.
    pub fn mean_time_to_sum(
        &self,
        cache: &mut Vec<crate::Value>,
        target: &crate::Value,
    ) -> Result<crate::Value> {
        use crate::value::BinOp;
        use crate::Value;
        let zero = Value::from(0);
        // `target = max(target, 0)`: keeps `target` unless `0 > target`.
        let target = if zero.binary(BinOp::Gt, target)?.truthy()? {
            zero.clone()
        } else {
            target.clone()
        };
        let cached = |cache: &[Value], target: &Value| -> Result<Option<Value>> {
            // `if target < len(cache): return cache[target]`.
            if !target
                .binary(BinOp::Lt, &Value::from(cache.len() as i64))?
                .truthy()?
            {
                return Ok(None);
            }
            match list_index(target) {
                Some(i) => Ok(Some(cache[i].clone())),
                None => Err(Error::Type(format!(
                    "list indices must be integers or slices, not {}",
                    target.type_name()
                ))),
            }
        };
        if let Some(result) = cached(cache, &target)? {
            return Ok(result);
        }
        if self.min_outcome()?.py_lt(&zero)? {
            return value_error("mean_time_to_sum does not handle negative outcomes.");
        }
        let denominator = self.denominator();
        let effect = &denominator - &self.try_quantity(crate::population::Comparison::Eq, &zero)?;
        if effect.is_zero() {
            return Err(Error::ZeroDivision(format!("Fraction({denominator}, 0)")));
        }
        let time_per_effect = dashu_ratio::RBig::from_parts(denominator.into(), effect);
        // `range(len(cache), target + 1)`.
        let stop = target.binary(BinOp::Add, &Value::from(1))?;
        let stop = match (&stop, list_index(&stop)) {
            (_, Some(stop)) => stop,
            // Python would loop over every target up to this one, for hours.
            (Value::Int(_), None) => {
                return Err(Error::Unsupported(
                    "mean_time_to_sum of a huge target".into(),
                ))
            }
            _ => {
                return Err(Error::Type(format!(
                    "'{}' object cannot be interpreted as an integer",
                    stop.type_name()
                )))
            }
        };
        // `self.reroll([0], depth='inf')`: the outcomes `in [0]` (`==`, as a list compares) are rerolled.
        let rerolled = self.reroll(
            &self
                .outcomes()
                .map(|o| o.eq_op(&zero, false)?.truthy())
                .collect::<Result<Vec<_>>>()?,
            None,
        )?;
        let mut result = None;
        for i in cache.len()..stop {
            crate::interrupt::poll()?;
            let i_value = Value::from(i as i64);
            // `.map(lambda x: self.mean_time_to_sum(i - x)).mean()`: the mapped Fractions weighted by quantity.
            let mut total = dashu_ratio::RBig::ZERO;
            for (x, quantity) in rerolled.items() {
                let next = i_value.binary(BinOp::Sub, x)?;
                let next = if zero.binary(BinOp::Gt, &next)?.truthy()? {
                    zero.clone()
                } else {
                    next
                };
                // Every `i - x` is below `i`, so cached; one that is not an int raises in the lookup or (NaN, never
                // `<` anything) in `range()` of the recursive call.
                let time = match cached(cache, &next)? {
                    Some(Value::Fraction(f)) => f,
                    _ => {
                        return Err(Error::Type(format!(
                            "'{}' object cannot be interpreted as an integer",
                            next.type_name()
                        )))
                    }
                };
                total += time * dashu_ratio::RBig::from(dashu_int::IBig::from(quantity.clone()));
            }
            let mean =
                total / dashu_ratio::RBig::from(dashu_int::IBig::from(rerolled.denominator()));
            let value = Value::Fraction(time_per_effect.clone() + mean);
            cache.push(value.clone());
            result = Some(value);
        }
        // `target >= len(cache)` here, so the loop ran at least once.
        result.ok_or_else(|| Error::Value("mean_time_to_sum: empty range".into()))
    }

    /// Outcomes become 1 if greater than `zero()`, -1 if less than `zero()`, and 0 otherwise (`Die.sign`).
    ///
    /// Note that for `float`s, +0.0, -0.0, and nan all become 0. Each outcome is compared with its own zero,
    /// `x * 0` (`Die._sign`), so a tuple compares with `()` and a str with `''`.
    pub fn sign(&self) -> Result<Self> {
        self.unary_operator(|x| {
            let z = x.try_zero()?;
            Ok(crate::Value::from(if x.py_gt(&z)? {
                1
            } else if x.py_lt(&z)? {
                -1
            } else {
                0
            }))
        })
    }
}

/// `x` as a Python list index, if it is one (an `int` or a `bool`) and fits the port's lists: `None` for anything
/// else, which Python rejects with `TypeError` (negative indices never reach here).
fn list_index(x: &crate::Value) -> Option<usize> {
    match x {
        crate::Value::Bool(b) => Some(*b as usize),
        crate::Value::Int(i) => usize::try_from(i).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::population::Comparison;
    use crate::{d, Die, Quantity};

    /// The typed API on `Die<i64>` answers comparisons with plain values, as the `Value` path does with `Ok`.
    #[test]
    fn typed_comparisons_return_plain_values() {
        let d6: Die<i64> = d(6);
        let q = |x: u8| Quantity::from(x);
        assert_eq!(d6.quantity_le(&4), q(4));
        assert_eq!(d6.quantity_ge(&4), q(3));
        assert_eq!(d6.quantity(Comparison::Lt, &4), q(3));
        assert_eq!(d6.quantity(Comparison::Ne, &7), q(6));
        let clipped = d6.clip(Some(&2), Some(&5));
        assert_eq!(clipped.items(), [(2, q(2)), (3, q(1)), (4, q(1)), (5, q(2))]);
        let truncated = d6.truncate(Some(&5), None);
        assert_eq!(truncated.items(), [(5, q(1)), (6, q(1))]);
        assert_eq!(d6.try_clip(Some(&2), Some(&5)), Ok(clipped));
    }
}
