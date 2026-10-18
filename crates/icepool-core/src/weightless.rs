//! Port of icepool `generator/weightless.py`: `WeightlessSource`, the source of `MultisetGenerator.weightless()`.
//!
//! EXPERIMENTAL upstream: a generator wrapper that sets all weights to 1. This only works properly if the wrapped
//! generator never takes multiple paths to the same output multiset.
//!
//! Python's `WeightlessGenerator` only holds the wrapped generator and makes this source from the wrapped one's;
//! sources are immutable values here, so the program layer and the facade keep the generator side.

use crate::ownership::{Owners, Traverse};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::order::{Order, OrderReason};
use crate::outcome::OutcomeOrder;
use crate::pool::{Count, MultisetSource, SourcePopped};
use crate::Quantity;

/// `WeightlessSource(wrapped)`. Equal (and hashed) as Python's `hash_key` `(WeightlessSource, wrapped.hash_key)`.
/// For language bindings, whose `generator.weightless()` makes one from the generator's source.
///
/// ```
/// use icepool_core::evaluator::{MultisetSource, Order, WeightlessSource};
/// use icepool_core::{d, Quantity};
///
/// let source = WeightlessSource::new(MultisetSource::Pool(d(6).pool(2).source()));
/// assert_eq!(source.outcomes().unwrap(), [1, 2, 3, 4, 5, 6]);
/// // Popping the 6s: none, one or both of the two dice, each with weight 1.
/// let popped = source.pop(Order::Descending, &6).unwrap();
/// assert_eq!(popped.len(), 3);
/// assert!(popped.iter().all(|(_, _, weight)| *weight == Quantity::ONE));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WeightlessSource<O> {
    wrapped: Arc<MultisetSource<O>>,
}

impl<O: OutcomeOrder + Traverse> Traverse for WeightlessSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.arc(&self.wrapped);
    }
}

impl<O: OutcomeOrder> WeightlessSource<O> {
    /// The weightless form of `wrapped`.
    pub fn new(wrapped: MultisetSource<O>) -> Self {
        WeightlessSource {
            wrapped: Arc::new(wrapped),
        }
    }

    /// The wrapped source.
    pub fn wrapped(&self) -> &MultisetSource<O> {
        &self.wrapped
    }

    /// The wrapped source's outcomes.
    pub fn outcomes(&self) -> Result<&[O]> {
        self.wrapped.outcomes()
    }

    /// The wrapped source's possibilities, each with weight 1. Two possibilities with the same count would merge
    /// different multisets' weights into one path, so they raise `UnsupportedOrder` (the driver then tries the
    /// other order or algorithm).
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<SourcePopped<O>>> {
        let mut seen_counts: Vec<&Count> = Vec::new();
        let popped = self.wrapped.pop(order, outcome)?;
        for (_, count, _) in &popped {
            if seen_counts.contains(&count) {
                return Err(Error::UnsupportedOrder(
                    "weightless cannot handle calls to pop() that produce the same count multiple times."
                        .into(),
                ));
            }
            seen_counts.push(count);
        }
        Ok(popped
            .into_iter()
            .map(|(source, count, _)| {
                (
                    MultisetSource::Weightless(WeightlessSource::new(source)),
                    count,
                    Quantity::ONE,
                )
            })
            .collect())
    }

    /// The wrapped source's size.
    pub fn size(&self) -> Result<Option<Count>> {
        self.wrapped.size()
    }

    /// The wrapped source's order preference.
    pub fn order_preference(&self) -> Result<(Order, OrderReason)> {
        self.wrapped.order_preference()
    }

    /// Whether the wrapped source is resolvable.
    pub fn is_resolvable(&self) -> Result<bool> {
        self.wrapped.is_resolvable()
    }
}
