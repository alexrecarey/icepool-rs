//! The Python side of `expand.py`: `cartesian_product` with Python's tuple or `Vector` as the outcome type, whose
//! dict keys Python hashes (an unhashable outcome raises there). The product itself is the generic
//! `cartesian_product_items`.

use crate::error::Result;
use crate::{Die, Quantity, Value};

/// `cartesian_product(*args, outcome_type=tuple)` when the population type is `Die`: a die of tuples, one per
/// joint outcome of `args` (each given as for `iter_cartesian_product`).
///
/// ```
/// use icepool_core::value::tupleize_dice;
/// use icepool_core::{Quantity, Value};
/// // `tupleize(d2, 'x')`: a die's items, and a plain argument as a single item.
/// let d2 = vec![(Value::from(1), Quantity::ONE), (Value::from(2), Quantity::ONE)];
/// let die = tupleize_dice(&[d2, vec![(Value::str("x"), Quantity::ONE)]]).unwrap();
/// let pair = |n: i64| Value::tuple(vec![Value::from(n), Value::str("x")]);
/// assert_eq!(die.outcomes().cloned().collect::<Vec<_>>(), [pair(1), pair(2)]);
/// ```
pub fn tupleize_dice(args: &[Vec<(Value, Quantity)>]) -> Result<Die<Value>> {
    Die::from_dict(cartesian_product_items(args, Value::tuple)?)
}

/// `cartesian_product(*args, outcome_type=Vector)` (`vectorize`) where some argument is a die.
///
/// ```
/// use icepool_core::value::{vectorize_dice, Vector};
/// use icepool_core::{Quantity, Value};
/// let d2 = vec![(Value::from(1), Quantity::ONE), (Value::from(2), Quantity::ONE)];
/// let die = vectorize_dice(&[d2.clone(), d2]).unwrap();
/// assert_eq!(die.len(), 4);
/// let top = Value::Vector(Vector::new(vec![Value::from(2), Value::from(2)]));
/// assert_eq!(die.max_outcome().unwrap(), &top);
/// ```
pub fn vectorize_dice(args: &[Vec<(Value, Quantity)>]) -> Result<Die<Value>> {
    Die::from_dict(cartesian_product_items(args, |items| {
        Value::Vector(crate::value::Vector::new(items))
    })?)
}

/// The `data` dict of `cartesian_product` (the generic product of `iter_cartesian_product`), each key hashed as
/// Python's `data[key] = ...` hashes it. The caller makes `Die(data)` or `Deck(data)` of it. Equal keys overwrite
/// each other's quantity and keep the first key, as dict assignment does.
///
/// ```
/// use icepool_core::value::cartesian_product_items;
/// use icepool_core::{Quantity, Value};
/// // `(1,)` and `(True,)` are one dict key: the second quantity overwrites the first.
/// let arg = vec![(Value::from(1), Quantity::from(2u8)), (Value::Bool(true), Quantity::from(3u8))];
/// let data = cartesian_product_items(&[arg], Value::tuple).unwrap();
/// assert_eq!(data, [(Value::tuple(vec![Value::from(1)]), Quantity::from(3u8))]);
/// ```
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
