//! Port of icepool `generator/compound_keep.py`: `CompoundKeepGenerator`, the additive union of keep generators
//! (pools, deals, other compound keeps) that is still a keep generator, with a keep tuple over all their
//! elements together.
//!
//! Python's generator holds its inner generators and makes their sources when evaluated; sources are immutable
//! values here, so [`CompoundKeep`] holds the inner sources directly.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::function::sorted_union;
use crate::keep_tuple::{
    self, pop_max_from_keep_tuple, pop_min_from_keep_tuple, sum_counts, KeepIndex,
};
use crate::order::{merge_order_preferences, Order, OrderReason};
use crate::outcome::Outcome;
use crate::pool::{Count, MultisetSource, Popped, Product};
use crate::Quantity;

/// `CompoundKeepGenerator(inners, keep_tuple)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompoundKeep<O> {
    inner_sources: Arc<[MultisetSource<O>]>,
    keep_tuple: Arc<[i64]>,
}

impl<O: Outcome> CompoundKeep<O> {
    /// `CompoundKeepGenerator(inners, keep_tuple)`, given the inners' sources.
    pub fn new(inner_sources: Vec<MultisetSource<O>>, keep_tuple: Vec<i64>) -> Self {
        CompoundKeep {
            inner_sources: inner_sources.into(),
            keep_tuple: keep_tuple.into(),
        }
    }

    pub fn keep_tuple(&self) -> &[i64] {
        &self.keep_tuple
    }

    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `keep_size()`: the total count produced by this generator.
    pub fn keep_size(&self) -> Result<i64> {
        sum_counts(&self.keep_tuple)
    }

    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple.iter().any(|&x| x < 0)
    }

    /// `_set_keep_tuple`: a copy with a modified keep tuple.
    pub fn with_keep_tuple(&self, keep_tuple: Vec<i64>) -> Self {
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
    pub fn multiply_counts(&self, constant: i64) -> Result<Self> {
        Ok(self.with_keep_tuple(keep_tuple::multiply_counts(&self.keep_tuple, constant)?))
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
    keep_tuple: Arc<[i64]>,
    /// `outcomes()`: the sorted union of the inner sources' (remaining) outcomes, computed once.
    outcomes: Arc<[O]>,
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

impl<O: Outcome> CompoundKeepSource<O> {
    fn new(inner_sources: Arc<[MultisetSource<O>]>, keep_tuple: Arc<[i64]>) -> Result<Self> {
        let outcomes = sorted_union(inner_sources.iter().map(|s| s.outcomes().iter()))?;
        Ok(CompoundKeepSource {
            inner_sources,
            keep_tuple,
            outcomes: outcomes.into(),
        })
    }

    pub fn outcomes(&self) -> &[O] {
        &self.outcomes
    }

    pub fn keep_tuple(&self) -> &[i64] {
        &self.keep_tuple
    }

    pub fn inner_sources(&self) -> &[MultisetSource<O>] {
        &self.inner_sources
    }

    /// `KeepSource.size`: `None` if any count is negative.
    pub fn size(&self) -> Result<Option<i64>> {
        if self.keep_tuple.iter().any(|&x| x < 0) {
            return Ok(None);
        }
        sum_counts(&self.keep_tuple).map(Some)
    }

    pub fn is_resolvable(&self) -> bool {
        self.inner_sources.iter().all(MultisetSource::is_resolvable)
    }

    pub fn order_preference(&self) -> (Order, OrderReason) {
        // Python can raise ConflictingOrderError here only for two opposite `Mandatory` preferences, and no source
        // has one: a merge reaches `Mandatory` only together with `Order.Any`, which later merges skip.
        merge_order_preferences(
            self.inner_sources
                .iter()
                .map(MultisetSource::order_preference),
        )
        .expect("no source prefers an order with Mandatory reason")
    }

    /// Pops `outcome` off every inner source (`itertools.product` of their possibilities, the last varying
    /// fastest); the total count comes off the matching end of the keep tuple.
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<Popped<Self>>> {
        let pops = self
            .inner_sources
            .iter()
            .map(|s| s.pop(order, outcome))
            .collect::<Result<Vec<_>>>()?;
        let mut result = Vec::new();
        for t in Product::new(&pops) {
            let mut sources = Vec::with_capacity(t.len());
            let mut total_count = 0i64;
            let mut total_weight = Quantity::ONE;
            for (source, count, weight) in t {
                let Count::Int(count) = count else {
                    return Err(Error::Unsupported(
                        "a compound keep of tuple sources".into(),
                    ));
                };
                sources.push(source.clone());
                total_count = total_count
                    .checked_add(*count)
                    .ok_or_else(|| Error::Unsupported("counts beyond 64 bits".into()))?;
                total_weight *= weight;
            }
            // The inner generators never have negative keeps (`additive_union` checks).
            let total_count = usize::try_from(total_count)
                .map_err(|_| Error::Unsupported("a negative count in a compound keep".into()))?;
            let (popped_keep_tuple, result_count) = if order.is_ascending() {
                pop_min_from_keep_tuple(&self.keep_tuple, total_count)?
            } else {
                pop_max_from_keep_tuple(&self.keep_tuple, total_count)?
            };
            let popped = CompoundKeepSource::new(sources.into(), popped_keep_tuple.into())?;
            result.push((popped, result_count, total_weight));
        }
        Ok(result)
    }
}
