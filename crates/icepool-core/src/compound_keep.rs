//! Port of icepool `generator/compound_keep.py`: `CompoundKeepGenerator`, the additive union of keep generators
//! (pools, deals, other compound keeps) that is still a keep generator, with a keep tuple over all their
//! elements together.
//!
//! Python's generator holds its inner generators and makes their sources when evaluated; sources are immutable
//! values here, so [`CompoundKeep`] holds the inner sources directly.
//!
//! A compound keep of compound keeps (`g = g + d6.pool(1)` in a loop) nests, and its sources recurse through the
//! nesting: a level each (`value::recursion`), so a deep one raises `RecursionError` rather than overflowing.

use crate::ownership::{Owners, Traverse};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::error::{Error, Result};
use crate::function::sorted_union;
use crate::keep_tuple::{
    self, pop_max_from_keep_tuple, pop_min_from_keep_tuple, sum_counts, KeepIndex,
};
use crate::order::{merge_order_preferences, Order, OrderReason};
use crate::outcome::OutcomeOrder;
use crate::pool::{Count, MultisetSource, Popped, Product};
use crate::value::recursion::level;
use crate::{Int, Quantity};

/// `CompoundKeepGenerator(inners, keep_tuple)`. For language bindings, whose `pool + pool` (of keep generators)
/// builds one.
///
/// ```
/// use icepool_core::evaluator::{CompoundKeep, KeepIndex, MultisetSource, Slice};
/// use icepool_core::{d, Int};
///
/// // `d6.pool(2) + d4.pool(1)`, then the highest two of the three dice.
/// let inners = vec![MultisetSource::Pool(d(6).pool(2).source()), MultisetSource::Pool(d(4).pool(1).source())];
/// let all = CompoundKeep::new(inners, vec![Int::ONE; 3]);
/// let index = KeepIndex::Slice { slice: Slice::new(Some(-2), None), step: false };
/// let highest = all.keep(&index).unwrap();
/// assert_eq!(highest.keep_tuple(), [Int::ZERO, Int::ONE, Int::ONE]);
/// assert_eq!(highest.keep_size(), Int::from(2));
/// assert_eq!(highest.source().unwrap().outcomes().unwrap(), [1, 2, 3, 4, 5, 6]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompoundKeep<O> {
    inner_sources: Arc<[MultisetSource<O>]>,
    keep_tuple: Arc<[Int]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for CompoundKeep<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc(&self.inner_sources);
    }
}

impl<O: OutcomeOrder> CompoundKeep<O> {
    /// `CompoundKeepGenerator(inners, keep_tuple)`, given the inners' sources.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SumEvaluator;
    /// use icepool_core::evaluator::{evaluate, CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Die, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (0, 1))`: the higher of a d6 and a d4.
    /// let inners = vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())];
    /// let highest = CompoundKeep::new(inners, vec![Int::ZERO, Int::ONE]);
    /// let source = MultisetSource::CompoundKeep(highest.source().unwrap());
    /// let sum = Die::from_weights(evaluate(&SumEvaluator, &[source]).unwrap());
    /// assert_eq!(sum, Die::from_weights([(1, 1u32), (2, 3), (3, 5), (4, 7), (5, 4), (6, 4)]));
    /// ```
    pub fn new(inner_sources: Vec<MultisetSource<O>>, keep_tuple: Vec<Int>) -> Self {
        CompoundKeep {
            inner_sources: inner_sources.into(),
            keep_tuple: keep_tuple.into(),
        }
    }

    /// The keep tuple over the elements of all the inner generators together, in ascending order of the rolls.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// assert_eq!(both.keep_tuple(), [1, 1].map(Int::from));
    /// ```
    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The sources of the inner generators.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// assert_eq!(both.inner_sources()[1], MultisetSource::Pool(d(4).pool(1).source()));
    /// ```
    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `keep_size()`: the total count produced by this generator.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// assert_eq!(both.keep_size(), Int::from(2));
    /// assert_eq!(both.multiply_counts(&Int::from(3)).keep_size(), Int::from(6));
    /// ```
    pub fn keep_size(&self) -> Int {
        sum_counts(&self.keep_tuple)
    }

    /// Whether any count of the keep tuple is negative.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// assert!(!both.has_negative_keeps());
    /// assert!(both.multiply_counts(&Int::from(-1)).has_negative_keeps());
    /// ```
    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple.iter().any(Int::is_negative)
    }

    /// `_set_keep_tuple`: a copy with a modified keep tuple.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// let lowest = both.with_keep_tuple(vec![Int::ONE, Int::ZERO]);
    /// assert_eq!((lowest.keep_size(), lowest.inner_sources().len()), (Int::ONE, 2));
    /// ```
    pub fn with_keep_tuple(&self, keep_tuple: Vec<Int>) -> Self {
        CompoundKeep {
            inner_sources: self.inner_sources.clone(),
            keep_tuple: keep_tuple.into(),
        }
    }

    /// `KeepGenerator.keep` with a non-negative keep tuple (see `keep_tuple::keep`).
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, KeepIndex, MultisetSource, Slice};
    /// use icepool_core::{d, Int};
    /// let inners = vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())];
    /// let both = CompoundKeep::new(inners, vec![Int::ONE; 2]);
    /// // Python's `[-1:]`: the higher of the two dice.
    /// let index = KeepIndex::Slice { slice: Slice::new(Some(-1), None), step: false };
    /// assert_eq!(both.keep(&index).unwrap().keep_tuple(), [0, 1].map(Int::from));
    /// ```
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        Ok(self.with_keep_tuple(keep_tuple::keep(&self.keep_tuple, index)?))
    }

    /// `KeepGenerator.multiply_counts`.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// // Python's `-generator`.
    /// assert_eq!(both.multiply_counts(&Int::from(-1)).keep_tuple(), [-1, -1].map(Int::from));
    /// ```
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        self.with_keep_tuple(keep_tuple::multiply_counts(&self.keep_tuple, constant))
    }

    /// `_make_source`.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// let source = both.source().unwrap();
    /// assert_eq!((source.keep_tuple(), source.inner_sources()), (both.keep_tuple(), both.inner_sources()));
    /// ```
    pub fn source(&self) -> Result<CompoundKeepSource<O>> {
        CompoundKeepSource::new(self.inner_sources.clone(), self.keep_tuple.clone())
    }
}

/// `CompoundKeepSource`. Equality and hashing follow its `hash_key` `(CompoundKeepSource, inner_sources,
/// keep_tuple)`.
///
/// ```
/// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
/// use icepool_core::{d, Int};
///
/// // `d6.pool(1) + d6.pool(1)`, keeping both.
/// let inner = || MultisetSource::Pool(d(6).pool(1).source());
/// let source = CompoundKeep::new(vec![inner(), inner()], vec![Int::ONE; 2]).source().unwrap();
/// assert_eq!(source.size(), Some(Int::from(2)));
/// assert!(source.is_resolvable().unwrap());
/// assert_eq!(source.inner_sources().len(), 2);
/// ```
#[derive(Clone, Debug)]
pub struct CompoundKeepSource<O> {
    inner_sources: Arc<[MultisetSource<O>]>,
    keep_tuple: Arc<[Int]>,
    /// `outcomes()`: the sorted union of the inner sources' (remaining) outcomes, computed when first asked (Python
    /// computes it per call: sorting can raise, and an evaluation with an unresolvable source never asks).
    outcomes: OnceLock<Arc<[O]>>,
}

impl<O: OutcomeOrder + Traverse> Traverse for CompoundKeepSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc(&self.inner_sources);
        if let Some(outcomes) = self.outcomes.get() {
            owners.arc(outcomes);
        }
    }
}

impl<O: PartialEq> PartialEq for CompoundKeepSource<O> {
    fn eq(&self, other: &Self) -> bool {
        self.inner_sources == other.inner_sources && self.keep_tuple == other.keep_tuple
    }
}

impl<O: Eq> Eq for CompoundKeepSource<O> {}

impl<O: Hash> Hash for CompoundKeepSource<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner_sources.hash(state);
        self.keep_tuple.hash(state);
    }
}

impl<O: OutcomeOrder> CompoundKeepSource<O> {
    fn new(inner_sources: Arc<[MultisetSource<O>]>, keep_tuple: Arc<[Int]>) -> Result<Self> {
        Ok(CompoundKeepSource {
            inner_sources,
            keep_tuple,
            outcomes: OnceLock::new(),
        })
    }

    /// `outcomes()`: the sorted union of the inner sources' outcomes.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// // The sorted union of 1..=6 and 1..=4.
    /// assert_eq!(both.source().unwrap().outcomes().unwrap(), [1, 2, 3, 4, 5, 6]);
    /// ```
    pub fn outcomes(&self) -> Result<&[O]> {
        if let Some(outcomes) = self.outcomes.get() {
            return Ok(outcomes);
        }
        let _level = level()?;
        let inner = self
            .inner_sources
            .iter()
            .map(MultisetSource::outcomes)
            .collect::<Result<Vec<_>>>()?;
        let outcomes = sorted_union(inner.into_iter().map(|o| o.iter()))?;
        Ok(self.outcomes.get_or_init(|| outcomes.into()))
    }

    /// The keep tuple over the elements of all the inner sources together, in ascending order of the rolls.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource, Order};
    /// use icepool_core::{d, Int};
    /// let inners = vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())];
    /// let source = CompoundKeep::new(inners, vec![Int::ONE; 2]).source().unwrap();
    /// assert_eq!(source.keep_tuple(), [1, 1].map(Int::from));
    /// // Where the d6 rolled a 6, the highest position is used up.
    /// let (rest, _, _) = &source.pop(Order::Descending, &6).unwrap()[1];
    /// assert_eq!(rest.keep_tuple(), [Int::ONE]);
    /// ```
    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The inner sources.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// let source = both.source().unwrap();
    /// assert_eq!(source.inner_sources()[0], MultisetSource::Pool(d(6).pool(1).source()));
    /// ```
    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `KeepSource.size`: `None` if any count is negative.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d4.pool(1)], (1, 1))`.
    /// let both = CompoundKeep::new(
    ///     vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())],
    ///     vec![Int::ONE; 2],
    /// );
    /// assert_eq!(both.source().unwrap().size(), Some(Int::from(2)));
    /// assert_eq!(both.multiply_counts(&Int::from(-1)).source().unwrap().size(), None);
    /// ```
    pub fn size(&self) -> Option<Int> {
        if self.keep_tuple.iter().any(Int::is_negative) {
            return None;
        }
        Some(sum_counts(&self.keep_tuple))
    }

    /// `is_resolvable()`: whether every inner source is.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource};
    /// use icepool_core::{d, Die, Int};
    /// let inner = |die: Die<i64>| MultisetSource::Pool(die.pool(1).source());
    /// let ok = CompoundKeep::new(vec![inner(d(6)), inner(d(4))], vec![Int::ONE; 2]);
    /// assert!(ok.source().unwrap().is_resolvable().unwrap());
    /// // One inner source without probability makes the whole unresolvable.
    /// let empty = CompoundKeep::new(vec![inner(d(6)), inner(Die::from_iter([]))], vec![Int::ONE; 2]);
    /// assert!(!empty.source().unwrap().is_resolvable().unwrap());
    /// ```
    pub fn is_resolvable(&self) -> Result<bool> {
        let _level = level()?;
        for source in self.inner_sources.iter() {
            if !source.is_resolvable()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The merged preferences of the inner sources. Python raises ConflictingOrderError here for two opposite
    /// `Mandatory` preferences; no built-in source has one (a merge reaches `Mandatory` only together with
    /// `Order.Any`, which later merges skip), but a host source may.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource, Order, OrderReason};
    /// use icepool_core::{d, Int};
    /// // `CompoundKeepGenerator([d6.pool(1), d6.pool(2)[-1:]], (1, 1))`: the second inner source prefers descending.
    /// let inners = vec![
    ///     MultisetSource::Pool(d(6).pool(1).source()),
    ///     MultisetSource::Pool(d(6).pool(2).highest(1).source()),
    /// ];
    /// let source = CompoundKeep::new(inners, vec![Int::ONE; 2]).source().unwrap();
    /// assert_eq!(source.order_preference().unwrap(), (Order::Descending, OrderReason::KeepSkip));
    /// ```
    pub fn order_preference(&self) -> Result<(Order, OrderReason)> {
        let _level = level()?;
        let preferences = self
            .inner_sources
            .iter()
            .map(MultisetSource::order_preference)
            .collect::<Result<Vec<_>>>()?;
        merge_order_preferences(preferences)
    }

    /// Pops `outcome` off every inner source (`itertools.product` of their possibilities, the last varying
    /// fastest); the total count comes off the matching end of the keep tuple.
    ///
    /// ```
    /// use icepool_core::evaluator::{CompoundKeep, MultisetSource, Order};
    /// use icepool_core::{d, Int, Quantity};
    /// let inners = vec![MultisetSource::Pool(d(6).pool(1).source()), MultisetSource::Pool(d(4).pool(1).source())];
    /// let source = CompoundKeep::new(inners, vec![Int::ONE; 2]).source().unwrap();
    /// // Popping the 1s: neither die, only the d4, only the d6, or both (the last inner source varies fastest).
    /// let popped = source.pop(Order::Ascending, &1).unwrap();
    /// let counts: Vec<Int> = popped.iter().map(|(_, count, _)| count.clone()).collect();
    /// assert_eq!(counts, [0, 1, 1, 2].map(Int::from));
    /// assert!(popped.iter().all(|(_, _, weight)| *weight == Quantity::ONE));
    /// ```
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<Popped<Self>>> {
        let _level = level()?;
        let pops = self
            .inner_sources
            .iter()
            .map(|s| s.pop(order, outcome))
            .collect::<Result<Vec<_>>>()?;
        let mut result = Vec::new();
        for t in Product::new(&pops) {
            let mut sources = Vec::with_capacity(t.len());
            let mut total_count = Int::ZERO;
            let mut total_weight = Quantity::ONE;
            for (source, count, weight) in t {
                let Count::Int(count) = count else {
                    return Err(Error::Unported("a compound keep of tuple sources".into()));
                };
                sources.push(source.clone());
                total_count += count;
                total_weight *= weight;
            }
            // The inner generators never have negative keeps (`additive_union` checks).
            if total_count.is_negative() {
                return Err(Error::Unported(
                    "a negative count in a compound keep".into(),
                ));
            }
            // `keep_tuple[total:]`: any total past the end takes everything.
            let total_count = total_count.clamp_usize();
            let (popped_keep_tuple, result_count) = if order.is_ascending() {
                pop_min_from_keep_tuple(&self.keep_tuple, total_count)
            } else {
                pop_max_from_keep_tuple(&self.keep_tuple, total_count)
            };
            let popped = CompoundKeepSource::new(sources.into(), popped_keep_tuple.into())?;
            result.push((popped, result_count, total_weight));
        }
        Ok(result)
    }
}
