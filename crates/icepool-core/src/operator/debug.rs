//! Port of icepool `operator/debug.py`: `MultisetForceOrder`, which forces a particular evaluation order and
//! does nothing else.

use super::Statelet;
use crate::error::{Error, Result};
use crate::pool::Count;

/// `MultisetForceOrder._initial_state`: `UnsupportedOrder` unless the order is the forced one (or the forced
/// order is `Any`); the size is the child's.
pub(super) fn initial_state(
    force_order: i64,
    order: i64,
    child_size: Option<Count>,
) -> Result<(Statelet, Option<Count>)> {
    if force_order != 0 && order != force_order {
        return Err(Error::UnsupportedOrder(format!(
            "Order was forced to {force_order}"
        )));
    }
    Ok((Statelet::None, child_size))
}
