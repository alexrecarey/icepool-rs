//! Die methods that manage quantities and outcomes without a callable: `zero`, `simplify`, `_pop_min` /
//! `_pop_max`, and `icepool.from_cumulative` (function.py).

use super::map::the_die;
use super::ops::{die_with_agains, py_iter, type_error};
use super::{PErr, PValue};
use crate::value::BinOp;
use crate::{Die, Value};

/// `die.zero()`: every outcome times `0`, which must resolve to a single outcome.
pub fn zero(die: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(the_die(die)?.zero()?))
}

/// `die.simplify()`: all quantities divided by their greatest common denominator.
pub fn simplify(die: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(the_die(die)?.simplify()?))
}

/// `die._pop_max()` if `max` else `die._pop_min()`: `(die without that outcome, its quantity)`. Zero quantities
/// stay in the popped die.
pub fn pop(die: PValue, max: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let (popped, quantity) = if super::catalogue::truthy(&max)? {
        die.pop_max()?
    } else {
        die.pop_min()?
    };
    Ok(PValue::tuple(vec![
        PValue::Die(popped),
        PValue::Value(Value::Int(quantity.into())),
    ]))
}

/// Whether two objects are the same `dict` key (`hash` and `==`). Only outcomes and dice reach here.
fn same_key(a: &PValue, b: &PValue) -> bool {
    match (a, b) {
        (PValue::Value(x), PValue::Value(y)) => x == y,
        (PValue::Die(x), PValue::Die(y)) => x == y,
        (PValue::Reroll, PValue::Reroll) => true,
        _ => false,
    }
}

/// `icepool.from_cumulative(outcomes, cumulative, reverse=reverse)` for `int` cumulative quantities: each
/// outcome's quantity is its cumulative quantity minus the previous one, collected in a `dict` (a repeated outcome
/// keeps its first position and its last difference) and passed to `Die`, which raises for a negative difference.
/// The other form, cumulative dice, is not ported yet.
pub fn from_cumulative(
    outcomes: PValue,
    cumulative: PValue,
    reverse: PValue,
) -> Result<PValue, PErr> {
    let mut outcomes = py_iter(outcomes)?;
    if outcomes.is_empty() {
        return Ok(PValue::Die(Die::from_dict(vec![])?));
    }
    let mut cumulative = py_iter(cumulative)?;
    if super::catalogue::truthy(&reverse)? {
        outcomes.reverse();
        cumulative.reverse();
    }
    match cumulative.first() {
        None => {
            return Err(PErr::Raise(crate::Error::Index(
                "list index out of range".into(),
            )))
        }
        Some(PValue::Value(Value::Int(_) | Value::Bool(_))) => {}
        Some(PValue::Die(_)) => {
            return Err(PErr::Unimplemented("from_cumulative with dice".into()))
        }
        Some(_) => return type_error("Unsupported type <class 'list'> for cumulative values."),
    }
    let mut keys: Vec<PValue> = Vec::new();
    let mut values: Vec<PValue> = Vec::new();
    let mut prev = Value::from(0);
    for (outcome, quantity) in outcomes.into_iter().zip(cumulative) {
        let PValue::Value(quantity) = quantity else {
            return Err(PErr::Unimplemented("non-number cumulative quantity".into()));
        };
        let diff = PValue::Value(quantity.binary(BinOp::Sub, &prev)?);
        match keys.iter().position(|k| same_key(k, &outcome)) {
            Some(i) => values[i] = diff,
            None => {
                keys.push(outcome);
                values.push(diff);
            }
        }
        prev = quantity;
    }
    die_with_agains(
        PValue::List(keys),
        Some(PValue::List(values)),
        None,
        PValue::Value(Value::None),
    )
}
