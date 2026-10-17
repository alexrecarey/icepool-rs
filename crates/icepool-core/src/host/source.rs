//! A multiset source the host implements (`MultisetSource::Host`): in the Python facade, a user's
//! `MultisetSource` subclass (made by a user's `MultisetGenerator` subclass), or any other source the core has no
//! variant for. The core driver pops it like its own sources; each of its methods is a callback.
//!
//! Python's driver treats every source alike through the `MultisetSourceBase` methods (generator/
//! multiset_generator.py, expression/multiset_expression_base.py); this is the same contract.

use crate::ownership::{Owners, Traverse};
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

/// The host's object is a leaf: what it holds is the host's to report.
impl<O: Traverse> Traverse for HostSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        owners.host(&self.object, self.object.as_any());
        owners.arc_with(&self.outcomes, |owners, outcomes| {
            if let Some(outcomes) = outcomes.get() {
                owners.arc(outcomes);
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::MultisetSource;
    use crate::Quantity;
    use std::hash::BuildHasher;

    /// A host source answering with fixed values; equal to another one with the same `key` (as a host's
    /// `hash_key` would make it).
    struct Fixed {
        key: u64,
        outcomes: Vec<i64>,
        size: Option<i64>,
        resolvable: bool,
    }

    impl HostSourceObject<i64> for Fixed {
        fn outcomes(&self) -> Result<Vec<i64>> {
            Ok(self.outcomes.clone())
        }
        fn size(&self) -> Result<Option<Count>> {
            Ok(self.size.map(|s| Count::Int(s.into())))
        }
        fn is_resolvable(&self) -> Result<bool> {
            Ok(self.resolvable)
        }
        fn order_preference(&self) -> Result<(Order, OrderReason)> {
            Ok((Order::Any, OrderReason::NoPreference))
        }
        /// Pops everything at once: one possibility, the source emptied, with the whole size as the count.
        fn pop(&self, _: Order, _: &i64) -> Result<Vec<SourcePopped<i64>>> {
            let rest = fixed(self.key, vec![], Some(0), true);
            let count = Count::Int(self.size.unwrap_or(0).into());
            Ok(vec![(
                MultisetSource::Host(rest),
                count,
                Quantity::from(3u8),
            )])
        }
        fn host_eq(&self, other: &dyn HostSourceObject<i64>) -> bool {
            other
                .as_any()
                .downcast_ref::<Fixed>()
                .is_some_and(|o| o.key == self.key)
        }
        fn host_hash(&self) -> u64 {
            self.key
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn fixed(key: u64, outcomes: Vec<i64>, size: Option<i64>, resolvable: bool) -> HostSource<i64> {
        HostSource::new(Arc::new(Fixed {
            key,
            outcomes,
            size,
            resolvable,
        }))
    }

    #[test]
    fn methods_call_the_host() {
        let s = fixed(7, vec![1, 2, 3], Some(2), true);
        assert_eq!(s.outcomes().unwrap(), &[1, 2, 3]);
        // The second call borrows the outcomes taken the first time.
        assert_eq!(s.outcomes().unwrap(), &[1, 2, 3]);
        assert_eq!(s.size().unwrap(), Some(Count::Int(2.into())));
        assert!(s.is_resolvable().unwrap());
        assert!(!fixed(7, vec![], None, false).is_resolvable().unwrap());
        assert_eq!(fixed(7, vec![], None, false).size().unwrap(), None);
        let popped = s.pop(Order::Ascending, &1).unwrap();
        assert_eq!(popped.len(), 1);
        let (rest, count, weight) = &popped[0];
        assert_eq!(*count, Count::Int(2.into()));
        assert_eq!(*weight, Quantity::from(3u8));
        assert_eq!(rest.outcomes().unwrap(), &[] as &[i64]);
    }

    #[test]
    fn equality_and_hash_are_the_hosts() {
        let a = fixed(1, vec![1], Some(1), true);
        // Another object the host says is equal: the driver's memo must merge them.
        let b = fixed(1, vec![1], Some(1), true);
        let c = fixed(2, vec![1], Some(1), true);
        assert_eq!(a, a.clone());
        assert_eq!(a, b);
        assert_ne!(a, c);
        let hasher = rustc_hash::FxBuildHasher;
        assert_eq!(hasher.hash_one(&a), hasher.hash_one(&b));
        assert_ne!(hasher.hash_one(&a), hasher.hash_one(&c));
        assert_eq!(format!("{c:?}"), "HostSource(0x2)");
    }
}
