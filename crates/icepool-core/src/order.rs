//! Port of icepool `order.py`: the order in which evaluators see outcomes, and how sources negotiate it.

use std::ops::Neg;

use crate::die::Die;
use crate::error::{Error, Result};
use crate::outcome::OutcomeOrder;
use crate::Int;

/// Can be used to define what order outcomes are seen in by MultisetEvaluators.
///
/// Python's `Order` is an `IntEnum`; sources test it as a number (`order > 0` pops the min outcome, anything else
/// the max), so `Any` pops like `Descending`.
///
/// ```
/// use icepool_core::Order;
/// assert_eq!(-Order::Ascending, Order::Descending);
/// assert_eq!(-Order::Any, Order::Any);
/// assert_eq!(Order::from_int(-1), Some(Order::Descending));
/// assert_eq!((Order::Ascending.value(), Order::Ascending.name()), (1, "Ascending"));
/// assert!(!Order::Any.is_ascending());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Smallest outcome first.
    Ascending = 1,
    /// Largest outcome first.
    Descending = -1,
    /// Either order will do.
    Any = 0,
}

impl Order {
    /// `Order(value)`: `None` where Python raises `ValueError`.
    pub fn from_int(value: i64) -> Option<Order> {
        match value {
            1 => Some(Order::Ascending),
            -1 => Some(Order::Descending),
            0 => Some(Order::Any),
            _ => None,
        }
    }

    /// `order > 0`.
    pub fn is_ascending(self) -> bool {
        self == Order::Ascending
    }

    /// `order.value`.
    pub fn value(self) -> i64 {
        self as i64
    }

    /// The name, as Python's `Order.name`.
    pub fn name(self) -> &'static str {
        match self {
            Order::Ascending => "Ascending",
            Order::Descending => "Descending",
            Order::Any => "Any",
        }
    }
}

impl Neg for Order {
    type Output = Order;

    fn neg(self) -> Order {
        match self {
            Order::Ascending => Order::Descending,
            Order::Descending => Order::Ascending,
            Order::Any => Order::Any,
        }
    }
}

/// Greater values represent higher priorities, which strictly override lower priorities.
///
/// ```
/// use icepool_core::evaluator::{Order, OrderReason};
/// use icepool_core::d;
/// assert!(OrderReason::Mandatory > OrderReason::PoolComposition);
/// assert_eq!(OrderReason::from_int(2), Some(OrderReason::KeepSkip));
/// // Keeping the lowest die of three, a pool prefers to pop from the bottom.
/// let source = d(6).pool(3).lowest(1).source();
/// assert_eq!(source.order_preference(), (Order::Ascending, OrderReason::KeepSkip));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderReason {
    /// There is no preference for either pop order.
    NoPreference = 0,
    /// This ordering is preferred by default.
    Default = 1,
    /// The keep_tuple for allows more skips in this pop order.
    KeepSkip = 2,
    /// The composition of dice in the pool favor this pop order.
    PoolComposition = 3,
    /// Something strictly requires this pop order.
    Mandatory = 4,
}

impl OrderReason {
    /// `OrderReason(value)`: `None` for a value it does not have.
    pub fn from_int(value: i64) -> Option<OrderReason> {
        Some(match value {
            0 => OrderReason::NoPreference,
            1 => OrderReason::Default,
            2 => OrderReason::KeepSkip,
            3 => OrderReason::PoolComposition,
            4 => OrderReason::Mandatory,
            _ => return None,
        })
    }

    /// `OrderReason(reason + 1)`; only called below `Mandatory`.
    fn next(self) -> OrderReason {
        match self {
            OrderReason::NoPreference => OrderReason::Default,
            OrderReason::Default => OrderReason::KeepSkip,
            OrderReason::KeepSkip => OrderReason::PoolComposition,
            OrderReason::PoolComposition | OrderReason::Mandatory => OrderReason::Mandatory,
        }
    }

    /// The name, as Python's `OrderReason.name`.
    pub fn name(self) -> &'static str {
        match self {
            OrderReason::NoPreference => "NoPreference",
            OrderReason::Default => "Default",
            OrderReason::KeepSkip => "KeepSkip",
            OrderReason::PoolComposition => "PoolComposition",
            OrderReason::Mandatory => "Mandatory",
        }
    }
}

/// Returns a pop order that fits the highest priority preferences.
///
/// Greater priorities strictly outrank lower priorities. Conflicting orders of the same priority are equal to an
/// `Order.Any` of the next-higher priority, except for conflicting `Mandatory` orders, which produces an exception
/// (`ConflictingOrderError`, an `Error::ConflictingOrder` here).
pub fn merge_order_preferences(
    preferences: impl IntoIterator<Item = (Order, OrderReason)>,
) -> Result<(Order, OrderReason)> {
    let mut result_order = Order::Any;
    let mut result_reason = OrderReason::NoPreference;
    for (order, reason) in preferences {
        if order == Order::Any || reason == OrderReason::NoPreference {
            continue;
        } else if reason > result_reason {
            result_order = order;
            result_reason = reason;
        } else if reason == result_reason {
            if result_order == Order::Any {
                result_order = order;
            } else if result_order == order {
                continue;
            } else if result_reason < OrderReason::Mandatory {
                result_order = Order::Any;
                result_reason = result_reason.next();
            } else {
                return Err(Error::ConflictingOrder(
                    "Conflicting order preferences.\nTip: If you are using highest(keep=k), try using \
                     lowest(drop=n-k) instead, or vice versa."
                        .into(),
                ));
            }
        }
    }
    Ok((result_order, result_reason))
}

/// Determines if the dice can be expressed as a one-sided truncation of a single base `Die`.
///
/// Returns `(can_truncate_min, can_truncate_max)`: if both are true, all dice are identical.
pub fn can_truncate<'a, O: OutcomeOrder + 'a>(
    dice: impl IntoIterator<Item = &'a Die<O>>,
) -> (bool, bool) {
    let mut dice_iter = dice.into_iter();
    let Some(mut base_die) = dice_iter.next() else {
        return (true, true);
    };
    let mut can_truncate_min = true;
    let mut can_truncate_max = true;
    for mut die in dice_iter {
        if die.len() == base_die.len() {
            if die == base_die {
                continue;
            } else {
                return (false, false);
            }
        }

        if die.len() > base_die.len() {
            std::mem::swap(&mut base_die, &mut die);
        }

        // Items compare as `(outcome, quantity)` tuples, by `==`.
        if can_truncate_min
            && die
                .items()
                .iter()
                .rev()
                .zip(base_die.items().iter().rev())
                .any(|(a, b)| a != b)
        {
            can_truncate_min = false;
        }

        if can_truncate_max
            && die
                .items()
                .iter()
                .zip(base_die.items())
                .any(|(a, b)| a != b)
        {
            can_truncate_max = false;
        }

        if !(can_truncate_min || can_truncate_max) {
            return (false, false);
        }
    }
    (can_truncate_min, can_truncate_max)
}

/// The number of dice that can be skipped from the ends of keep_tuple: `(lo_skip, hi_skip)`.
pub fn lo_hi_skip(keep_tuple: &[Int]) -> (usize, usize) {
    let Some(lo_skip) = keep_tuple.iter().position(|c| !c.is_zero()) else {
        return (keep_tuple.len(), keep_tuple.len());
    };
    let hi_skip = keep_tuple
        .iter()
        .rev()
        .position(|c| !c.is_zero())
        .expect("a nonzero count exists");
    (lo_skip, hi_skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_reason_from_int_is_the_enum_value() {
        // The facade reads a host source's `order_preference()` reason back through this.
        for reason in [
            OrderReason::NoPreference,
            OrderReason::Default,
            OrderReason::KeepSkip,
            OrderReason::PoolComposition,
            OrderReason::Mandatory,
        ] {
            assert_eq!(OrderReason::from_int(reason as i64), Some(reason));
        }
        assert_eq!(OrderReason::from_int(5), None);
        assert_eq!(OrderReason::from_int(-1), None);
    }
}
