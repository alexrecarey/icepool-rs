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

/// `CompoundKeepGenerator(inners, keep_tuple)`.
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
    pub fn new(inner_sources: Vec<MultisetSource<O>>, keep_tuple: Vec<Int>) -> Self {
        CompoundKeep {
            inner_sources: inner_sources.into(),
            keep_tuple: keep_tuple.into(),
        }
    }

    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `keep_size()`: the total count produced by this generator.
    pub fn keep_size(&self) -> Int {
        sum_counts(&self.keep_tuple)
    }

    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple.iter().any(Int::is_negative)
    }

    /// `_set_keep_tuple`: a copy with a modified keep tuple.
    pub fn with_keep_tuple(&self, keep_tuple: Vec<Int>) -> Self {
        CompoundKeep {
            inner_sources: self.inner_sources.clone(),
            keep_tuple: keep_tuple.into(),
        }
    }

    /// `KeepGenerator.keep` with a non-negative keep tuple (see `keep_tuple::keep`).
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        Ok(self.with_keep_tuple(keep_tuple::keep(&self.keep_tuple, index)?))
    }

    /// `KeepGenerator.multiply_counts`.
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        self.with_keep_tuple(keep_tuple::multiply_counts(&self.keep_tuple, constant))
    }

    /// `_make_source`.
    pub fn source(&self) -> Result<CompoundKeepSource<O>> {
        CompoundKeepSource::new(self.inner_sources.clone(), self.keep_tuple.clone())
    }
}

/// `CompoundKeepSource`. Equality and hashing follow its `hash_key` `(CompoundKeepSource, inner_sources,
/// keep_tuple)`.
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

    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `KeepSource.size`: `None` if any count is negative.
    pub fn size(&self) -> Option<Int> {
        if self.keep_tuple.iter().any(Int::is_negative) {
            return None;
        }
        Some(sum_counts(&self.keep_tuple))
    }

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
                    return Err(Error::Unsupported(
                        "a compound keep of tuple sources".into(),
                    ));
                };
                sources.push(source.clone());
                total_count += count;
                total_weight *= weight;
            }
            // The inner generators never have negative keeps (`additive_union` checks).
            if total_count.is_negative() {
                return Err(Error::Unsupported(
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
