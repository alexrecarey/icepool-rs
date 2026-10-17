//! Port of icepool `generator/pool.py` (with the keep-tuple half of `generator/keep.py` `KeepGenerator`) and the
//! sources the evaluator pops outcomes from.
//!
//! A `Pool` represents a multiset of outcomes resulting from the roll of several dice. Note that operators are
//! performed on the multiset of rolls, not the multiset of dice. Evaluation is most efficient when the dice are the
//! same or same-side truncations of each other (d4, d6, d8, d10, d12 are all same-side truncations of d12): popping
//! the top outcome of a d8 twice leaves a die equal to a d6, and equal dice merge into one entry with a count, whose
//! rolls are then popped together with a binomial row (`comb_row`) instead of one by one.

use crate::ownership::{Owners, Traverse};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_base::BitTest;

use crate::compound_keep::CompoundKeepSource;
use crate::deal::DealSource;
use crate::die::Die;
use crate::error::{Error, Result};
use crate::evaluator::MultisetEvaluator;
use crate::function::sorted_union;
use crate::keep_tuple::{
    self, pop_max_from_keep_tuple, pop_min_from_keep_tuple, sum_counts, KeepIndex,
};
use crate::math::comb_row;
use crate::multi_deal::MultiDealSource;
use crate::order::{can_truncate, lo_hi_skip, Order, OrderReason};
use crate::outcome::{Outcome, OutcomeOrder};
use crate::weightless::WeightlessSource;
use crate::{Int, Quantity};

/// Above this many dice the port refuses a pool (`(1,) * n` keep tuple): Python would spend minutes or run out of
/// memory.
pub(crate) const MAX_POOL_SIZE: usize = 1 << 24;

/// `dice_counts[die] += count` in a dict: insertion order, equal dice (same `hash_key`) merged.
fn add_die<O: OutcomeOrder>(counts: &mut Vec<(Die<O>, usize)>, die: Die<O>, count: usize) {
    match counts.iter_mut().find(|(d, _)| *d == die) {
        Some((_, c)) => *c += count,
        None => counts.push((die, count)),
    }
}

/// `die_a.hash_key < die_b.hash_key`: `(Die, tuple(items))` compared as Python compares tuples. The classes are the
/// same object, so it comes down to the items: the first `(outcome, quantity)` pair that differs (by `==`) decides,
/// by outcome if those differ (which may raise), else by quantity; a prefix is smaller.
fn hash_key_lt<O: OutcomeOrder>(
    a: &Die<O>,
    b: &Die<O>,
) -> std::result::Result<bool, O::Incomparable> {
    for ((oa, qa), (ob, qb)) in a.items().iter().zip(b.items()) {
        if oa != ob {
            return oa.try_lt(ob);
        }
        if qa != qb {
            return Ok(qa < qb);
        }
    }
    Ok(a.len() < b.len())
}

/// A pool's dice: `(die, count)` pairs sorted by the die's `hash_key`.
type Dice<O> = Arc<[(Die<O>, usize)]>;

/// `tuple(sorted(dice_counts.items(), key=lambda kv: kv[0].hash_key))`: the canonical order of a pool's dice.
fn sort_dice<O: OutcomeOrder>(dice_counts: Vec<(Die<O>, usize)>) -> Result<Dice<O>> {
    Ok(crate::outcome::sort_by::<O, _>(dice_counts, |(a, _), (b, _)| hash_key_lt(a, b))?.into())
}

/// `math.prod(die.denominator()**count for die, count in dice)`.
///
/// A pool holds up to `MAX_POOL_SIZE` dice, so the product can have `2**24` times as many bits as a die's
/// denominator: `MemoryError` past the allocation cap (on wasm32 a die with a 129+-bit denominator would otherwise
/// trap), where Python answers given memory.
fn denominator<O: OutcomeOrder>(dice: &[(Die<O>, usize)]) -> Result<Quantity> {
    let bits = dice.iter().try_fold(0u64, |total, (die, count)| {
        (die.denominator().bit_len() as u64)
            .checked_mul(*count as u64)?
            .checked_add(total)
    });
    crate::memory::check_bits(bits, "a pool's denominator")?;
    Ok(dice
        .iter()
        .map(|(die, count)| die.denominator().pow(*count))
        .product())
}

/// A pool of dice with a keep tuple (Python `Pool`, a `KeepGenerator`). Cloning is cheap.
///
/// The keep tuple says how many times each die counts, in ascending order of the rolls: `[0, 1, 1, 1]` keeps the
/// highest three of four.
///
/// ```
/// use icepool_core::{d, Int, Pool};
/// let pool = Pool::new(vec![(d(6), 2), (d(8), 1)]).unwrap();
/// assert_eq!(pool.raw_size(), 3);
/// assert_eq!(pool.lowest(1).keep_tuple(), [1, 0, 0].map(Int::from));
/// ```
#[derive(Clone, Debug)]
pub struct Pool<O> {
    /// Dice put in the pool zero times are not here.
    dice: Dice<O>,
    /// The union of possible outcomes among all dice in this pool in ascending order, including those of dice put
    /// in zero times.
    outcomes: Arc<[O]>,
    /// How many times to count each sorted roll, lowest first.
    keep_tuple: Arc<[Int]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for Pool<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc_with(&self.dice, |owners, dice| {
            for (die, _) in dice {
                die.traverse(owners);
            }
        });
        owners.arc(&self.outcomes);
    }
}

impl<O: OutcomeOrder> Pool<O> {
    /// `Pool(dice, times)` after `itemize` and `implicit_convert_to_die`: one `(die, times)` per element, in order.
    ///
    /// Dice may be in the pool zero times, in which case their outcomes will be considered but without any count
    /// (unless another die has that outcome).
    pub fn new(dice: Vec<(Die<O>, usize)>) -> Result<Self> {
        let size = dice
            .iter()
            .try_fold(0usize, |a, (_, t)| a.checked_add(*t))
            .filter(|&n| n <= MAX_POOL_SIZE)
            .ok_or_else(|| Error::Overflow("too many dice in the pool".into()))?;
        Self::from_parts(&dice, vec![Int::ONE; size].into())
    }

    /// The pool `new(dice)` makes, with `keep_tuple` (one count per die put in) instead of all ones: what keeping
    /// and multiplying counts make of it, and, given the dice of all the pools in turn, what merging pools does
    /// (`additive_union`). For a host that holds the dice rather than the pool (the Python facade's `RsPool`).
    pub fn from_parts(dice: &[(Die<O>, usize)], keep_tuple: Arc<[Int]>) -> Result<Self> {
        let mut dice_counts = Vec::new();
        for (die, count) in dice {
            if *count == 0 {
                continue;
            }
            add_die(&mut dice_counts, die.clone(), *count);
        }
        // Includes dice with zero qty.
        let outcomes = sorted_union(dice.iter().map(|(d, _)| d.outcomes()))?;
        Self::from_mapping(dice_counts, outcomes.into(), keep_tuple)
    }

    /// `Pool._new_from_mapping`: `dice_counts` in insertion order.
    fn from_mapping(
        dice_counts: Vec<(Die<O>, usize)>,
        outcomes: Arc<[O]>,
        keep_tuple: Arc<[Int]>,
    ) -> Result<Self> {
        Ok(Pool {
            dice: sort_dice(dice_counts)?,
            outcomes,
            keep_tuple,
        })
    }

    /// The `(die, count)` pairs, sorted by the die's `hash_key`.
    pub fn dice(&self) -> &[(Die<O>, usize)] {
        &self.dice
    }

    /// The union of possible outcomes among all dice in this pool in ascending order.
    pub fn outcomes(&self) -> &[O] {
        &self.outcomes
    }

    /// The tuple indicating which elements will be counted.
    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The total count produced by this generator.
    pub fn keep_size(&self) -> Int {
        sum_counts(&self.keep_tuple)
    }

    /// The number of dice in this pool before the keep_tuple is applied.
    pub fn raw_size(&self) -> usize {
        self.dice.iter().map(|(_, c)| c).sum()
    }

    /// The number of ways the dice can roll: the product of each die's denominator to the power of its count.
    pub fn denominator(&self) -> Result<Quantity> {
        denominator(&self.dice)
    }

    /// Whether any element of the keep tuple is negative.
    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple.iter().any(Int::is_negative)
    }

    /// Produces a copy with a modified keep_tuple.
    fn set_keep_tuple(&self, keep_tuple: Vec<Int>) -> Self {
        Pool {
            dice: self.dice.clone(),
            outcomes: self.outcomes.clone(),
            keep_tuple: keep_tuple.into(),
        }
    }

    /// `KeepGenerator.keep` (`pool[index]`): modifies the generator by counting elements in sorted order.
    ///
    /// The rolls are sorted in ascending order for this purpose, regardless of which order the outcomes are
    /// evaluated in. If this is called more than once, the selection is applied relative to the previous
    /// `keep_tuple`. For example, applying `[:2][-1]` would produce the second-lowest roll.
    ///
    /// For an `int` index Python goes on to evaluate the result into a `Die` (the keep evaluator): that is the
    /// caller's job. If any of the current `keep_tuple` has negative counts, Python falls back to the less-capable
    /// `MultisetExpression` version and the result is not a pool: not ported yet.
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        Ok(self.set_keep_tuple(keep_tuple::keep(&self.keep_tuple, index)?))
    }

    /// `pool * constant`, `constant * pool`, and `-pool` (constant -1): every count multiplied.
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        self.set_keep_tuple(keep_tuple::multiply_counts(&self.keep_tuple, constant))
    }

    /// `Pool.additive_union` of pools: if all sorted positions count the same, the pools merge into one pool.
    /// `None` when they don't (Python builds a `CompoundKeepGenerator` or an expression instead).
    pub fn additive_union(pools: &[Pool<O>]) -> Result<Option<Self>> {
        let keep_tuple: Vec<Int> = pools
            .iter()
            .flat_map(|p| p.keep_tuple.iter().cloned())
            .collect();
        if !keep_tuple.iter().all(|x| *x == keep_tuple[0]) {
            return Ok(None);
        }
        let mut dice = Vec::new();
        for pool in pools {
            for (die, count) in pool.dice.iter() {
                add_die(&mut dice, die.clone(), *count);
            }
        }
        let outcomes = sorted_union(pools.iter().map(|p| p.outcomes.iter()))?;
        Ok(Some(Self::from_mapping(
            dice,
            outcomes.into(),
            keep_tuple.into(),
        )?))
    }

    /// `_make_source`.
    pub fn source(&self) -> PoolSource<O> {
        PoolSource {
            dice: self.dice.clone(),
            outcomes: self.outcomes.clone(),
            lo: 0,
            hi: self.outcomes.len(),
            keep_tuple: self.keep_tuple.clone(),
        }
    }
}

/// The typed API (design/library-architecture.md section 4): keeping and evaluating without Python's arguments.
impl<O: Outcome> Pool<O> {
    /// Keeps the `keep` highest rolls (Python's `pool.highest(keep)`); the others count zero times. Applied to a
    /// pool that already keeps some rolls, it keeps relative to those (as [`Pool::keep`]).
    ///
    /// # Panics
    ///
    /// If the pool already has negative counts, which Python cannot keep as a pool either.
    ///
    /// ```
    /// use icepool_core::{d, Int};
    /// let best = d(6).pool(4).highest(3);
    /// assert_eq!(best.keep_tuple(), [0, 1, 1, 1].map(Int::from));
    /// ```
    pub fn highest(&self, keep: usize) -> Self {
        self.keep_slice(keep_tuple::highest_slice(
            Some(i64::try_from(keep).unwrap_or(i64::MAX)),
            None,
        ))
    }

    /// Keeps the `keep` lowest rolls (Python's `pool.lowest(keep)`); see [`Pool::highest`].
    ///
    /// ```
    /// use icepool_core::{d, Int};
    /// let worst = d(6).pool(3).lowest(1);
    /// assert_eq!(worst.keep_tuple(), [1, 0, 0].map(Int::from));
    /// ```
    pub fn lowest(&self, keep: usize) -> Self {
        self.keep_slice(keep_tuple::lowest_slice(
            Some(i64::try_from(keep).unwrap_or(i64::MAX)),
            None,
        ))
    }

    fn keep_slice(&self, slice: Result<keep_tuple::Slice>) -> Self {
        slice
            .and_then(|slice| self.keep(&KeepIndex::Slice { slice, step: false }))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Evaluates this pool (Python's `evaluator.evaluate(pool)`): the die of the evaluator's final outcomes, each
    /// weighted by how likely its rolls are.
    ///
    /// ```
    /// use icepool_core::evaluator::poker::LargestCountEvaluator;
    /// use icepool_core::{d, Die, Int};
    /// // How many of a kind in 3d6: 120 of the 216 rolls have none, 90 a pair, 6 three of a kind.
    /// let largest: Die<Int> = d(6).pool(3).evaluate(&LargestCountEvaluator).unwrap();
    /// assert_eq!(largest, Die::from_weights([(Int::from(1), 120u32), (Int::from(2), 90), (Int::from(3), 6)]));
    /// ```
    pub fn evaluate<E>(&self, evaluator: &E) -> Result<Die<E::Final>>
    where
        E: MultisetEvaluator<O>,
        E::Final: Outcome,
    {
        // `Die(final_outcomes, final_weights)`: plain outcomes, so equal ones simply add up.
        let finals = crate::evaluator::evaluate(evaluator, &[MultisetSource::Pool(self.source())])?;
        Ok(Die::from_weights(finals))
    }
}

/// What the evaluator pops outcomes from: one variant per kind of multiset generator (`MultisetSource`), each a
/// small immutable value that hashes for the evaluator's memo. `Pool` and `Deal` produce an `int` count per
/// outcome; `MultiDeal` (a `MultisetTupleSource`) a tuple of counts, one per hand. `CompoundKeep` is the additive
/// union of other keep generators' sources. `Weightless` wraps another source with every weight set to 1
/// (`MultisetGenerator.weightless()`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MultisetSource<O> {
    /// A [`Pool`].
    Pool(PoolSource<O>),
    /// A [`crate::Deal`].
    Deal(DealSource<O>),
    /// A [`crate::MultiDeal`], which produces a tuple of counts.
    MultiDeal(MultiDealSource<O>),
    /// The additive union of keep generators.
    CompoundKeep(CompoundKeepSource<O>),
    /// Another source with all weights 1.
    Weightless(WeightlessSource<O>),
    /// A source the host implements ([`crate::host::HostSource`]): the facade's user-defined sources.
    Host(crate::host::HostSource<O>),
}

/// One possibility after popping an outcome: the rest of the source, the count of the popped outcome, its weight.
pub type Popped<S> = (S, Int, Quantity);

/// What a source produces for one outcome, and its size: an `int` (multiset generators) or a tuple of `int`s
/// (multiset tuple generators). The evaluator's `next_state` receives it as is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Count {
    /// One count.
    Int(Int),
    /// One count per hand (a `MultiDeal`).
    Tuple(Arc<[Int]>),
}

/// A source's possibilities as the driver sees them.
pub type SourcePopped<O> = (MultisetSource<O>, Count, Quantity);

fn int_popped<S, O>(
    popped: Vec<Popped<S>>,
    f: impl Fn(S) -> MultisetSource<O>,
) -> Vec<SourcePopped<O>> {
    popped
        .into_iter()
        .map(|(s, c, w)| (f(s), Count::Int(c), w))
        .collect()
}

impl<O: OutcomeOrder + Traverse> Traverse for MultisetSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        match self {
            MultisetSource::Pool(s) => s.traverse(owners),
            MultisetSource::Deal(s) => s.traverse(owners),
            MultisetSource::MultiDeal(s) => s.traverse(owners),
            MultisetSource::CompoundKeep(s) => s.traverse(owners),
            MultisetSource::Weightless(s) => s.traverse(owners),
            MultisetSource::Host(s) => s.traverse(owners),
        }
    }
}

impl<O: OutcomeOrder> MultisetSource<O> {
    /// The possible outcomes, ascending. A compound keep sorts its inner sources' outcomes together, which can
    /// raise.
    pub fn outcomes(&self) -> Result<&[O]> {
        Ok(match self {
            MultisetSource::Pool(s) => s.outcomes(),
            MultisetSource::Deal(s) => s.outcomes(),
            MultisetSource::MultiDeal(s) => s.outcomes(),
            MultisetSource::CompoundKeep(s) => s.outcomes()?,
            MultisetSource::Weightless(s) => s.outcomes()?,
            MultisetSource::Host(s) => s.outcomes()?,
        })
    }

    /// The total number of elements output by this source, if only non-negative counts are output (else `None`).
    /// A tuple source's size is the tuple of its hands' sizes.
    pub fn size(&self) -> Result<Option<Count>> {
        Ok(match self {
            MultisetSource::Pool(s) => s.size().map(Count::Int),
            MultisetSource::Deal(s) => s.size().map(Count::Int),
            MultisetSource::MultiDeal(s) => Some(Count::Tuple(s.size())),
            MultisetSource::CompoundKeep(s) => s.size().map(Count::Int),
            MultisetSource::Weightless(s) => s.size()?,
            MultisetSource::Host(s) => s.size()?,
        })
    }

    /// Whether this source contains any probability (a host source's answer may raise).
    pub fn is_resolvable(&self) -> Result<bool> {
        Ok(match self {
            MultisetSource::Pool(s) => s.is_resolvable(),
            MultisetSource::Deal(s) => s.is_resolvable(),
            MultisetSource::MultiDeal(s) => s.is_resolvable(),
            MultisetSource::CompoundKeep(s) => s.is_resolvable()?,
            MultisetSource::Weightless(s) => s.is_resolvable()?,
            MultisetSource::Host(s) => s.is_resolvable()?,
        })
    }

    /// The preferred pop order of this source and a reason why the order is preferred.
    pub fn order_preference(&self) -> Result<(Order, OrderReason)> {
        Ok(match self {
            MultisetSource::Pool(s) => s.order_preference(),
            MultisetSource::Deal(s) => s.order_preference(),
            MultisetSource::MultiDeal(s) => s.order_preference(),
            MultisetSource::CompoundKeep(s) => s.order_preference()?,
            MultisetSource::Weightless(s) => s.order_preference()?,
            MultisetSource::Host(s) => s.order_preference()?,
        })
    }

    /// Yields `(this source with the outcome popped, count of that outcome, weight)` for each possibility. If
    /// `outcome` is not the one this source pops in `order`, the only possibility is `(self, 0, 1)` (a tuple of
    /// zeros for a tuple source).
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<SourcePopped<O>>> {
        Ok(match self {
            MultisetSource::Pool(s) => int_popped(s.pop(order, outcome)?, MultisetSource::Pool),
            MultisetSource::Deal(s) => int_popped(s.pop(order, outcome)?, MultisetSource::Deal),
            MultisetSource::MultiDeal(s) => s
                .pop(order, outcome)?
                .into_iter()
                .map(|(s, c, w)| (MultisetSource::MultiDeal(s), Count::Tuple(c), w))
                .collect(),
            MultisetSource::CompoundKeep(s) => {
                int_popped(s.pop(order, outcome)?, MultisetSource::CompoundKeep)
            }
            MultisetSource::Weightless(s) => s.pop(order, outcome)?,
            MultisetSource::Host(s) => s.pop(order, outcome)?,
        })
    }
}

/// `PoolSource`: a pool as the evaluator sees it, with the outcomes not popped yet.
///
/// Equality and hashing follow Python's `hash_key` `(PoolSource, dice, keep_tuple)`: the outcomes are not part of
/// it.
#[derive(Clone, Debug)]
pub struct PoolSource<O> {
    dice: Dice<O>,
    /// The remaining outcomes are `outcomes[lo..hi]`, shared with the pool they came from.
    outcomes: Arc<[O]>,
    lo: usize,
    hi: usize,
    keep_tuple: Arc<[Int]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for PoolSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc_with(&self.dice, |owners, dice| {
            for (die, _) in dice {
                die.traverse(owners);
            }
        });
        owners.arc(&self.outcomes);
    }
}

impl<O: PartialEq> PartialEq for PoolSource<O> {
    fn eq(&self, other: &Self) -> bool {
        self.dice == other.dice && self.keep_tuple == other.keep_tuple
    }
}

impl<O: Eq> Eq for PoolSource<O> {}

impl<O: Hash> Hash for PoolSource<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dice.hash(state);
        self.keep_tuple.hash(state);
    }
}

impl<O: OutcomeOrder> PoolSource<O> {
    /// The `(die, count)` pairs still to pop.
    pub fn dice(&self) -> &[(Die<O>, usize)] {
        &self.dice
    }

    /// The outcomes still to pop, in ascending order.
    pub fn outcomes(&self) -> &[O] {
        &self.outcomes[self.lo..self.hi]
    }

    /// The keep tuple of the dice still to pop.
    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The number of ways the remaining dice can roll.
    pub fn denominator(&self) -> Result<Quantity> {
        denominator(&self.dice)
    }

    /// `KeepSource.size`: the total count, unknown (`None`) with negative counts.
    pub fn size(&self) -> Option<Int> {
        if self.keep_tuple.iter().any(Int::is_negative) {
            return None;
        }
        Some(sum_counts(&self.keep_tuple))
    }

    /// Whether the pool can be evaluated: every die has some outcome.
    pub fn is_resolvable(&self) -> bool {
        self.dice.iter().all(|(die, _)| !die.is_empty())
    }

    /// `PoolSource.order_preference`. Note the `KeepSkip` directions: a keep tuple that skips more dice at the low
    /// end prefers popping descending, the opposite of `order.pool_pop_order` (copied as is).
    pub fn order_preference(&self) -> (Order, OrderReason) {
        // `unique_dice()` is a set; `can_truncate` does not depend on its iteration order.
        let (can_truncate_min, can_truncate_max) = can_truncate(self.dice.iter().map(|(d, _)| d));
        if can_truncate_min && !can_truncate_max {
            return (Order::Ascending, OrderReason::PoolComposition);
        }
        if can_truncate_max && !can_truncate_min {
            return (Order::Descending, OrderReason::PoolComposition);
        }

        let (lo_skip, hi_skip) = lo_hi_skip(&self.keep_tuple);
        if lo_skip > hi_skip {
            return (Order::Descending, OrderReason::KeepSkip);
        }
        if hi_skip > lo_skip {
            return (Order::Ascending, OrderReason::KeepSkip);
        }

        (Order::Any, OrderReason::NoPreference)
    }

    /// Pops the lowest outcome (`order > 0`) or the highest (otherwise), if it is `outcome`: every way the dice
    /// can roll it, as `(rest of the pool, count of outcome, weight)`.
    ///
    /// Each distinct die contributes its possible numbers of hits (a binomial row); the hits come off the matching
    /// end of the keep tuple, whose popped counts add up to the count. Possibilities where no count remains in the
    /// keep tuple are not yielded one by one: their dice are dumped in exchange for the denominator, and they come
    /// last as one empty pool.
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<Popped<Self>>> {
        let unchanged = || Ok(vec![(self.clone(), Int::ZERO, Quantity::ONE)]);
        if self.lo == self.hi {
            return unchanged();
        }
        let ascending = order.is_ascending();
        let (pop_outcome, next_lo, next_hi) = if ascending {
            (&self.outcomes[self.lo], self.lo + 1, self.hi)
        } else {
            (&self.outcomes[self.hi - 1], self.lo, self.hi - 1)
        };
        if outcome != pop_outcome {
            return unchanged();
        }
        // `itertools.product` takes every die's possibilities up front.
        let generators = self
            .dice
            .iter()
            .map(|(die, die_count)| iter_die_pop(die, *die_count, outcome, ascending))
            .collect::<Result<Vec<_>>>()?;
        let next_source = |dice, keep_tuple: Vec<Int>| PoolSource {
            dice,
            outcomes: self.outcomes.clone(),
            lo: next_lo,
            hi: next_hi,
            keep_tuple: keep_tuple.into(),
        };
        let mut result = Vec::new();
        let mut skip_weight: Option<Quantity> = None;
        for pop in Product::new(&generators) {
            crate::interrupt::poll()?;
            let mut total_hits = 0;
            let mut result_weight = Quantity::ONE;
            let mut next_dice_counts = Vec::new();
            for (popped_die, misses, hits, weight) in pop {
                if !popped_die.is_empty() && *misses > 0 {
                    add_die(&mut next_dice_counts, popped_die.clone(), *misses);
                }
                total_hits += hits;
                result_weight *= weight;
            }
            let (popped_keep_tuple, result_count) = if ascending {
                pop_min_from_keep_tuple(&self.keep_tuple, total_hits)
            } else {
                pop_max_from_keep_tuple(&self.keep_tuple, total_hits)
            };
            let popped_pool = next_source(sort_dice(next_dice_counts)?, popped_keep_tuple);
            if popped_pool.keep_tuple.iter().all(Int::is_zero) {
                // Dump all dice in exchange for the denominator.
                *skip_weight.get_or_insert(Quantity::ZERO) +=
                    result_weight * popped_pool.denominator()?;
                continue;
            }
            result.push((popped_pool, result_count, result_weight));
        }
        if let Some(skip_weight) = skip_weight {
            let popped_pool = next_source(Arc::new([]), Vec::new());
            result.push((popped_pool, sum_counts(&self.keep_tuple), skip_weight));
        }
        Ok(result)
    }
}

/// `(popped_die, misses, hits, weight)`: the die left after popping, how many of its rolls didn't and did roll the
/// popped outcome, and the weight of that many rolling it.
type DiePop<O> = (Die<O>, usize, usize, Quantity);

/// `iter_die_pop_min` / `iter_die_pop_max`: the possibilities of several identical dice rolling the popped outcome
/// (at or beyond the die's own end outcome).
fn iter_die_pop<O: OutcomeOrder>(
    die: &Die<O>,
    rolls: usize,
    outcome: &O,
    min: bool,
) -> Result<Vec<DiePop<O>>> {
    let end = if min {
        die.min_outcome()?
    } else {
        die.max_outcome()?
    };
    if end != outcome {
        return Ok(vec![(die.clone(), rolls, 0, Quantity::ONE)]);
    }
    let (popped_die, single_weight) = if min { die.pop_min()? } else { die.pop_max()? };
    if popped_die.is_empty() {
        // This is the last outcome. All dice must roll this outcome.
        let weight = single_weight.pow(rolls);
        return Ok(vec![(popped_die, 0, rolls, weight)]);
    }
    Ok(comb_row(rolls, &single_weight)
        .into_iter()
        .enumerate()
        .map(|(hits, weight)| (popped_die.clone(), rolls - hits, hits, weight))
        .collect())
}

/// `itertools.product` over lists: the last list varies fastest.
pub(crate) struct Product<'a, T> {
    lists: &'a [Vec<T>],
    index: Option<Vec<usize>>,
}

impl<'a, T> Product<'a, T> {
    pub(crate) fn new(lists: &'a [Vec<T>]) -> Self {
        let index = if lists.iter().any(Vec::is_empty) {
            None
        } else {
            Some(vec![0; lists.len()])
        };
        Product { lists, index }
    }
}

impl<'a, T> Iterator for Product<'a, T> {
    type Item = Vec<&'a T>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.index.as_mut()?;
        let item = index.iter().zip(self.lists).map(|(&i, l)| &l[i]).collect();
        // Advance like an odometer; past the last combination, stop.
        let mut k = index.len();
        loop {
            if k == 0 {
                self.index = None;
                break;
            }
            k -= 1;
            index[k] += 1;
            if index[k] < self.lists[k].len() {
                break;
            }
            index[k] = 0;
        }
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    #[test]
    fn a_huge_denominator_is_a_memory_error() {
        // The cap allows `8 * MAX_ALLOC_BYTES` bits; a full pool of a die with a denominator of `2**bits` passes it.
        let bits = 8 * crate::memory::MAX_ALLOC_BYTES / MAX_POOL_SIZE + 1;
        let die = |q: Quantity| Die::from_dict(vec![(Value::from(1i64), q)]).unwrap();
        let huge = Pool::new(vec![(die(Quantity::ONE << bits), MAX_POOL_SIZE)]).unwrap();
        assert!(matches!(huge.denominator(), Err(Error::TooLarge(_))));
        // A denominator of 1 is never too big; a smaller pool of the same die is answered.
        assert_eq!(
            Pool::new(vec![(die(Quantity::ONE), MAX_POOL_SIZE)])
                .unwrap()
                .denominator()
                .unwrap(),
            Quantity::ONE
        );
        let small = Pool::new(vec![(die(Quantity::ONE << bits), 2)]).unwrap();
        assert_eq!(small.denominator().unwrap(), Quantity::ONE << (2 * bits));
    }
}
