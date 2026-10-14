//! Port of icepool `order.py` (the part the sources need so far).

/// Can be used to define what order outcomes are seen in by MultisetEvaluators.
///
/// Python's `Order` is an `IntEnum`; sources test it as a number (`order > 0` pops the min outcome, anything else
/// the max), so `Any` pops like `Descending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    Ascending = 1,
    Descending = -1,
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
}
