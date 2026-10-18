//! The Python side of `expand.py`: `cartesian_product` with Python's tuple or `Vector` as the outcome type, whose
//! dict keys Python hashes (an unhashable outcome raises there). The product itself is `crate::expand`'s.

use crate::error::Result;
use crate::{Die, Quantity, Value};

/// `cartesian_product(*args, outcome_type=tuple)` when the population type is `Die`: a die of tuples, one per
/// joint outcome of `args` (each given as for `iter_cartesian_product`).
pub fn tupleize_dice(args: &[Vec<(Value, Quantity)>]) -> Result<Die<Value>> {
    Die::from_dict(cartesian_product_items(args, Value::tuple)?)
}

/// `cartesian_product(*args, outcome_type=Vector)` (`vectorize`) where some argument is a die.
pub fn vectorize_dice(args: &[Vec<(Value, Quantity)>]) -> Result<Die<Value>> {
    Die::from_dict(cartesian_product_items(args, |items| {
        Value::Vector(crate::value::Vector::new(items))
    })?)
}

/// The `data` dict of `cartesian_product` (`crate::expand::cartesian_product_items`), each key hashed as Python's
/// `data[key] = ...` hashes it. The caller makes `Die(data)` or `Deck(data)` of it.
pub fn cartesian_product_items(
    args: &[Vec<(Value, Quantity)>],
    outcome_type: impl Fn(Vec<Value>) -> Value,
) -> Result<Vec<(Value, Quantity)>> {
    crate::expand::cartesian_product_items(args, |outcomes| {
        let key = outcome_type(outcomes);
        key.check_hashable()?;
        Ok(key)
    })
}
