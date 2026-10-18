// Shared setup of the doctests in src/host/source.rs, pulled into their hidden lines by `include!` (a doctest
// cannot import another's items). A copy of the toy that `HostSourceObject`'s own example shows in full: keep the two
// in step. Not part of the crate.

use icepool_core::evaluator::{Count, MultisetSource, Order, OrderReason, SourcePopped};
use icepool_core::host::{HostSource, HostSourceObject};
use icepool_core::{Quantity, Result};
use std::any::Any;
use std::hash::BuildHasher;
use std::sync::Arc;
struct Fixed(Vec<(i64, i64)>);
fn fixed(items: Vec<(i64, i64)>) -> MultisetSource<i64> {
    MultisetSource::Host(HostSource::new(Arc::new(Fixed(items))))
}
impl HostSourceObject<i64> for Fixed {
    fn outcomes(&self) -> Result<Vec<i64>> {
        Ok(self.0.iter().map(|&(o, _)| o).collect())
    }
    fn size(&self) -> Result<Option<Count>> {
        Ok(Some(Count::Int(
            self.0.iter().map(|&(_, c)| c).sum::<i64>().into(),
        )))
    }
    fn is_resolvable(&self) -> Result<bool> {
        Ok(true)
    }
    fn order_preference(&self) -> Result<(Order, OrderReason)> {
        Ok((Order::Any, OrderReason::NoPreference))
    }
    fn pop(&self, _order: Order, outcome: &i64) -> Result<Vec<SourcePopped<i64>>> {
        let count: i64 = self
            .0
            .iter()
            .filter(|(o, _)| o == outcome)
            .map(|&(_, c)| c)
            .sum();
        let rest = self
            .0
            .iter()
            .filter(|(o, _)| o != outcome)
            .copied()
            .collect();
        Ok(vec![(fixed(rest), Count::Int(count.into()), Quantity::ONE)])
    }
    fn host_eq(&self, other: &dyn HostSourceObject<i64>) -> bool {
        other
            .as_any()
            .downcast_ref::<Fixed>()
            .is_some_and(|o| o.0 == self.0)
    }
    fn host_hash(&self) -> u64 {
        rustc_hash::FxBuildHasher.hash_one(&self.0)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
