//! A multiset source the host implements (`MultisetSource::Host`): in the Python facade, a user's
//! `MultisetSource` subclass (made by a user's `MultisetGenerator` subclass), or any other source the core has no
//! variant for. The core driver pops it like its own sources; each of its methods is a callback.
//!
//! Python's driver treats every source alike through the `MultisetSourceBase` methods (generator/
//! multiset_generator.py, expression/multiset_expression_base.py); this is the same contract.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::error::Result;
use crate::order::{Order, OrderReason};
use crate::pool::{Count, SourcePopped};

/// The callbacks of a host source: `MultisetSourceBase`'s abstract methods, plus the host's `==` and hash
/// (`MaybeHashKeyed`: by `hash_key`), which key the driver's memo of rooms as they key Python's.
pub trait HostSourceObject<O>: Send + Sync {
    /// `outcomes()`: the possible outcomes, ascending.
    fn outcomes(&self) -> Result<Vec<O>>;
    /// `size()`: the total count, if only non-negative counts are output.
    fn size(&self) -> Result<Option<Count>>;
    /// `is_resolvable()`.
    fn is_resolvable(&self) -> Result<bool>;
    /// `order_preference()`.
    fn order_preference(&self) -> Result<(Order, OrderReason)>;
    /// `pop(order, outcome)`: the popped sources (any kind), counts and weights.
    fn pop(&self, order: Order, outcome: &O) -> Result<Vec<SourcePopped<O>>>;
    /// The host's `self == other` for another host source.
    fn host_eq(&self, other: &dyn HostSourceObject<O>) -> bool;
    /// The host's `hash(self)`, taken once.
    fn host_hash(&self) -> u64;
    /// For `host_eq`: the implementation's own type.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A host source with its outcomes, asked for once (the driver borrows them).
#[derive(Clone)]
pub struct HostSource<O> {
    object: Arc<dyn HostSourceObject<O>>,
    outcomes: Arc<OnceLock<Arc<[O]>>>,
}

impl<O> HostSource<O> {
    pub fn new(object: Arc<dyn HostSourceObject<O>>) -> Self {
        HostSource {
            object,
            outcomes: Arc::new(OnceLock::new()),
        }
    }

    pub fn object(&self) -> &Arc<dyn HostSourceObject<O>> {
        &self.object
    }

    pub fn outcomes(&self) -> Result<&[O]> {
        if let Some(o) = self.outcomes.get() {
            return Ok(o);
        }
        let outcomes: Arc<[O]> = self.object.outcomes()?.into();
        Ok(self.outcomes.get_or_init(|| outcomes))
    }

    pub fn size(&self) -> Result<Option<Count>> {
        self.object.size()
    }

    pub fn is_resolvable(&self) -> Result<bool> {
        self.object.is_resolvable()
    }

    pub fn order_preference(&self) -> Result<(Order, OrderReason)> {
        self.object.order_preference()
    }

    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<SourcePopped<O>>> {
        self.object.pop(order, outcome)
    }
}

impl<O> PartialEq for HostSource<O> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) || self.object.host_eq(&*other.object)
    }
}

impl<O> Eq for HostSource<O> {}

impl<O> Hash for HostSource<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.object.host_hash().hash(state);
    }
}

impl<O> fmt::Debug for HostSource<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HostSource({:#x})", self.object.host_hash())
    }
}
