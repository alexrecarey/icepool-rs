//! A typed `MultisetMixture` (expression/multiset_mixture.py) of keep generators: what `Die.explode_to_pool` and
//! `Die.reroll_to_pool` return.
//!
//! Python's mixture may hold any multiset expression; the typed one holds what those two methods put in it, pools
//! and the compound keeps `pool + pool.highest(n)` makes. Every such inner is a keep generator, so the mixture is
//! always `_static_keepable`: keeping and multiplying counts apply to each inner, as Python forwards them.

use crate::compound_keep::CompoundKeep;
use crate::creation_args::DieArg;
use crate::die::Die;
use crate::error::{Error, Result};
use crate::evaluator::MultisetEvaluator;
use crate::keep_tuple::{self, KeepIndex};
use crate::outcome::{Outcome, OutcomeOrder};
use crate::pool::{MultisetSource, Pool};
use crate::{Int, Quantity};

/// A keep generator (Python's `KeepGenerator`) of the kinds a [`PoolMixture`] holds: a [`Pool`], or the
/// [`CompoundKeep`] that adding keep generators makes when their positions do not all count the same.
///
/// Two keep generators are equal when Python's `hash_key`s are: the same kind, the same dice (or inner sources)
/// and the same keep tuple. A pool's outcomes are not part of it, so a mixture merges two pools that differ only in
/// the outcomes of dice put in zero times, keeping the first.
///
/// ```
/// use icepool_core::{d, Int, KeepGenerator};
/// // `d6.pool(2) + d4.pool(1)` is one pool; with only the highest d6 kept it is a compound keep.
/// let both = KeepGenerator::from(d(6).pool(2)).additive_union(&d(4).pool(1).into()).unwrap();
/// assert!(matches!(both, KeepGenerator::Pool(_)));
/// let mixed = KeepGenerator::from(d(6).pool(2).highest(1)).additive_union(&d(4).pool(1).into()).unwrap();
/// assert!(matches!(mixed, KeepGenerator::CompoundKeep(_)));
/// assert_eq!(mixed.keep_size(), Int::from(2));
/// ```
#[derive(Clone, Debug)]
pub enum KeepGenerator<O> {
    /// A pool of dice.
    Pool(Pool<O>),
    /// `CompoundKeepGenerator`: the additive union of other keep generators, with its own keep tuple.
    CompoundKeep(CompoundKeep<O>),
}

impl<O> From<Pool<O>> for KeepGenerator<O> {
    fn from(pool: Pool<O>) -> Self {
        KeepGenerator::Pool(pool)
    }
}

impl<O> From<CompoundKeep<O>> for KeepGenerator<O> {
    fn from(keep: CompoundKeep<O>) -> Self {
        KeepGenerator::CompoundKeep(keep)
    }
}

/// `hash_key` equality: `(Pool, dice, keep_tuple)` or `(CompoundKeepGenerator, inners, keep_tuple)`.
impl<O: OutcomeOrder> PartialEq for KeepGenerator<O> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (KeepGenerator::Pool(a), KeepGenerator::Pool(b)) => {
                a.dice() == b.dice() && a.keep_tuple() == b.keep_tuple()
            }
            (KeepGenerator::CompoundKeep(a), KeepGenerator::CompoundKeep(b)) => a == b,
            _ => false,
        }
    }
}

impl<O: OutcomeOrder> KeepGenerator<O> {
    /// How many times each sorted position counts, lowest first.
    ///
    /// ```
    /// use icepool_core::{d, Int, KeepGenerator};
    /// assert_eq!(KeepGenerator::from(d(6).pool(3).lowest(1)).keep_tuple(), [1, 0, 0].map(Int::from));
    /// ```
    pub fn keep_tuple(&self) -> &[Int] {
        match self {
            KeepGenerator::Pool(p) => p.keep_tuple(),
            KeepGenerator::CompoundKeep(c) => c.keep_tuple(),
        }
    }

    /// The total count produced by this generator (the sum of its keep tuple).
    ///
    /// ```
    /// use icepool_core::{d, Int, KeepGenerator};
    /// assert_eq!(KeepGenerator::from(d(6).pool(4).highest(3)).keep_size(), Int::from(3));
    /// ```
    pub fn keep_size(&self) -> Int {
        keep_tuple::sum_counts(self.keep_tuple())
    }

    /// Whether any element of the keep tuple is negative.
    ///
    /// ```
    /// use icepool_core::{d, Int, KeepGenerator};
    /// let negated = KeepGenerator::from(d(6).pool(2)).multiply_counts(&Int::from(-1));
    /// assert!(negated.has_negative_keeps());
    /// ```
    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple().iter().any(Int::is_negative)
    }

    /// `KeepGenerator.keep` with a sequence or slice index: counts the sorted rolls relative to the current keep
    /// tuple (see [`Pool::keep`]).
    ///
    /// ```
    /// use icepool_core::evaluator::KeepIndex;
    /// use icepool_core::{d, Int, KeepGenerator};
    /// let highest = KeepGenerator::from(d(6).pool(3)).keep(&KeepIndex::Seq(vec![None, Some(Int::ONE)])).unwrap();
    /// assert_eq!(highest.keep_tuple(), [0, 0, 1].map(Int::from));
    /// ```
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        Ok(match self {
            KeepGenerator::Pool(p) => KeepGenerator::Pool(p.keep(index)?),
            KeepGenerator::CompoundKeep(c) => KeepGenerator::CompoundKeep(c.keep(index)?),
        })
    }

    /// `KeepGenerator.multiply_counts` (`generator * constant`, `-generator`).
    ///
    /// ```
    /// use icepool_core::{d, Int, KeepGenerator};
    /// let doubled = KeepGenerator::from(d(6).pool(2)).multiply_counts(&Int::from(2));
    /// assert_eq!(doubled.keep_tuple(), [2, 2].map(Int::from));
    /// ```
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        match self {
            KeepGenerator::Pool(p) => KeepGenerator::Pool(p.multiply_counts(constant)),
            KeepGenerator::CompoundKeep(c) => {
                KeepGenerator::CompoundKeep(c.multiply_counts(constant))
            }
        }
    }

    /// `self + other` (`additive_union`): two pools whose sorted positions all count the same merge into one pool
    /// (`Pool.additive_union`); otherwise the result is a compound keep counting every kept roll once
    /// (`KeepGenerator.additive_union`). With negative counts on either side Python makes a general multiset
    /// expression instead, which a typed keep generator cannot hold: [`Error::Unported`].
    ///
    /// ```
    /// use icepool_core::{d, Int, KeepGenerator};
    /// // `d6.pool(2).highest(1) + d4.pool(1)`: the higher d6 and the d4, each counted once.
    /// let sum = KeepGenerator::from(d(6).pool(2).highest(1)).additive_union(&d(4).pool(1).into()).unwrap();
    /// assert_eq!(sum.keep_tuple(), [1, 1].map(Int::from));
    /// ```
    pub fn additive_union(&self, other: &Self) -> Result<Self> {
        if let (KeepGenerator::Pool(a), KeepGenerator::Pool(b)) = (self, other) {
            if let Some(merged) = Pool::additive_union(&[a.clone(), b.clone()])? {
                return Ok(KeepGenerator::Pool(merged));
            }
        }
        if self.has_negative_keeps() || other.has_negative_keeps() {
            return Err(Error::Unported(
                "adding keep generators with negative counts (a multiset expression)".into(),
            ));
        }
        let size = (self.keep_size() + other.keep_size())
            .to_usize()
            .ok_or_else(|| Error::Overflow("too many dice in the pool".into()))?;
        let inners = vec![self.source()?, other.source()?];
        Ok(KeepGenerator::CompoundKeep(CompoundKeep::new(
            inners,
            vec![Int::ONE; size],
        )))
    }

    /// `_make_source`: what an evaluation pops outcomes from.
    ///
    /// ```
    /// use icepool_core::{d, KeepGenerator};
    /// let source = KeepGenerator::from(d(6).pool(2)).source().unwrap();
    /// assert_eq!(source.outcomes().unwrap(), [1, 2, 3, 4, 5, 6]);
    /// ```
    pub fn source(&self) -> Result<MultisetSource<O>> {
        Ok(match self {
            KeepGenerator::Pool(p) => MultisetSource::Pool(p.source()),
            KeepGenerator::CompoundKeep(c) => MultisetSource::CompoundKeep(c.source()?),
        })
    }
}

/// A weighted mixture of keep generators (Python's `MultisetMixture` of pools): evaluating it evaluates each inner
/// and mixes the results by weight. [`Die::explode_to_pool`] and [`Die::reroll_to_pool`] make one.
///
/// ```
/// use icepool_core::evaluator::basic::SumEvaluator;
/// use icepool_core::{d, Die, PoolMixture, Quantity};
/// // `MultisetMixture({d(4).pool(1): 1, d(6).pool(1): 1}).sum()`: a d4 or a d6, half the time each.
/// let mixture = PoolMixture::new([(d(4).pool(1).into(), Quantity::ONE), (d(6).pool(1).into(), Quantity::ONE)]);
/// let sum = mixture.evaluate(&SumEvaluator).unwrap();
/// assert_eq!(sum, Die::from_weights([(1, 5u32), (2, 5), (3, 5), (4, 5), (5, 2), (6, 2)]));
/// ```
#[derive(Clone, Debug)]
pub struct PoolMixture<O> {
    /// `_inner_expressions`: each inner with its weight, in insertion order, equal inners merged.
    inners: Vec<(KeepGenerator<O>, Quantity)>,
}

/// A single pool, as the mixture `MultisetMixture({pool: 1})`: it evaluates, keeps and multiplies exactly as the
/// pool does. `explode_to_pool` and `reroll_to_pool` return a plain pool where nothing explodes or is rerolled.
impl<O> From<Pool<O>> for PoolMixture<O> {
    fn from(pool: Pool<O>) -> Self {
        PoolMixture {
            inners: vec![(KeepGenerator::Pool(pool), Quantity::ONE)],
        }
    }
}

impl<O: OutcomeOrder> PoolMixture<O> {
    /// `MultisetMixture(mapping)` built up as `data[inner] += weight`: equal inners (by `hash_key`) add their
    /// weights into the first one seen.
    ///
    /// ```
    /// use icepool_core::{d, PoolMixture, Quantity};
    /// let twice = PoolMixture::new([(d(6).pool(2).into(), Quantity::ONE), (d(6).pool(2).into(), Quantity::ONE)]);
    /// assert_eq!(twice.inners().len(), 1);
    /// assert_eq!(twice.inners()[0].1, Quantity::from(2u8));
    /// ```
    pub fn new(inners: impl IntoIterator<Item = (KeepGenerator<O>, Quantity)>) -> Self {
        let mut data: Vec<(KeepGenerator<O>, Quantity)> = Vec::new();
        for (inner, weight) in inners {
            match data.iter_mut().find(|(key, _)| *key == inner) {
                Some((_, w)) => *w += weight,
                None => data.push((inner, weight)),
            }
        }
        PoolMixture { inners: data }
    }

    /// The inners with their weights, in insertion order.
    ///
    /// ```
    /// use icepool_core::{d, Quantity};
    /// // `d6.explode_to_pool(1, depth=1)`: no explosion (5 of 6), or a 6 then another roll (1 of 6 each).
    /// let mixture = d(6).explode_to_pool(1, |&x| x == 6, 1).unwrap();
    /// let weights: Vec<_> = mixture.inners().iter().map(|(_, w)| w.clone()).collect();
    /// assert_eq!(weights, [30u8, 5, 1].map(Quantity::from));
    /// ```
    pub fn inners(&self) -> &[(KeepGenerator<O>, Quantity)] {
        &self.inners
    }

    /// `MultisetMixture._unary_operation(op)`: `op` applied to each inner, the results counted with the inners'
    /// weights (equal results merge).
    fn map_inners(
        &self,
        op: impl Fn(&KeepGenerator<O>) -> Result<KeepGenerator<O>>,
    ) -> Result<Self> {
        let mapped = self
            .inners
            .iter()
            .map(|(inner, weight)| Ok((op(inner)?, weight.clone())))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::new(mapped))
    }

    /// `mixture.keep(index)` for a sequence or slice index: every inner is a keep generator, so each is kept.
    /// (Python's `keep(int)` goes on to evaluate the result with the keep evaluator; run that evaluator on the
    /// mixture for the same die.)
    ///
    /// ```
    /// use icepool_core::evaluator::KeepIndex;
    /// use icepool_core::{d, Int};
    /// let mixture = d(6).explode_to_pool(2, |&x| x == 6, 1).unwrap();
    /// // `[..., 1]`: only the highest roll of each pool counts.
    /// let highest = mixture.keep(&KeepIndex::Seq(vec![None, Some(Int::ONE)])).unwrap();
    /// assert!(highest.inners().iter().all(|(inner, _)| inner.keep_size() == Int::ONE));
    /// ```
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        self.map_inners(|inner| inner.keep(index))
    }

    /// `mixture.multiply_counts(constant)`: each inner's counts multiplied.
    ///
    /// ```
    /// use icepool_core::{d, Int};
    /// let doubled = d(6).explode_to_pool(1, |&x| x == 6, 1).unwrap().multiply_counts(&Int::from(2));
    /// assert_eq!(doubled.inners()[0].0.keep_tuple(), [Int::from(2)]);
    /// ```
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        let mapped = self
            .inners
            .iter()
            .map(|(inner, weight)| (inner.multiply_counts(constant), weight.clone()));
        Self::new(mapped)
    }
}

/// The typed API: keeping and evaluating without Python's arguments, as on [`Pool`].
impl<O: Outcome> PoolMixture<O> {
    /// Keeps the `keep` highest rolls of every inner (Python's `mixture.highest(keep)`).
    ///
    /// # Panics
    ///
    /// If an inner already has negative counts, as [`Pool::highest`].
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SumEvaluator;
    /// use icepool_core::Die;
    /// use icepool_core::d;
    /// // Python: `d(6).explode_to_pool(2, depth=1).highest(1).sum()`.
    /// let best = d(6).explode_to_pool(2, |&x| x == 6, 1).unwrap().highest(1).evaluate(&SumEvaluator).unwrap();
    /// assert_eq!(best, Die::from_weights([(1, 36u32), (2, 108), (3, 180), (4, 252), (5, 324), (6, 396)]));
    /// ```
    pub fn highest(&self, keep: usize) -> Self {
        self.keep_slice(keep_tuple::highest_slice(
            Some(i64::try_from(keep).unwrap_or(i64::MAX)),
            None,
        ))
    }

    /// Keeps the `keep` lowest rolls of every inner (Python's `mixture.lowest(keep)`); see
    /// [`PoolMixture::highest`].
    ///
    /// ```
    /// use icepool_core::{d, Int};
    /// let worst = d(6).explode_to_pool(2, |&x| x == 6, 1).unwrap().lowest(1);
    /// assert!(worst.inners().iter().all(|(inner, _)| inner.keep_size() == Int::ONE));
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

    /// Evaluates the mixture (Python's `evaluator.evaluate(mixture)`): each inner is evaluated into a die, and the
    /// dice are mixed by weight (`final_data[result] += weight; Die(final_data)`), each scaled to a common
    /// denominator first.
    ///
    /// ```
    /// use icepool_core::evaluator::poker::LargestCountEvaluator;
    /// use icepool_core::{d, Die, Int};
    /// // Python: `d(4).explode_to_pool(3, depth=2).largest_count()`.
    /// let largest = d(4).explode_to_pool(3, |&x| x == 4, 2).unwrap().evaluate(&LargestCountEvaluator).unwrap();
    /// let expected = [43008u32, 165888, 45120, 6048, 1620, 387, 63, 9, 1];
    /// assert_eq!(largest, Die::from_weights((1..=9).map(Int::from).zip(expected)));
    /// ```
    pub fn evaluate<E>(&self, evaluator: &E) -> Result<Die<E::Final>>
    where
        E: MultisetEvaluator<O>,
        E::Final: Outcome,
    {
        let mut final_data: Vec<(Die<E::Final>, Quantity)> = Vec::new();
        for (inner, weight) in &self.inners {
            // `Die(final_outcomes, final_weights)` of each inner: plain outcomes, so equal ones simply add up.
            let finals = crate::evaluator::evaluate(evaluator, &[inner.source()?])?;
            let result = Die::from_weights(finals);
            match final_data.iter_mut().find(|(die, _)| *die == result) {
                Some((_, w)) => *w += weight,
                None => final_data.push((result, weight.clone())),
            }
        }
        let (dice, weights) = final_data
            .into_iter()
            .map(|(die, weight)| (DieArg::Die(die), weight))
            .unzip();
        Die::new(dice, weights)
    }
}
