//! Die methods that manage quantities and outcomes without a callable: `zero`, `zero_outcome`, `simplify`,
//! `_pop_min` / `_pop_max`, `sign`, `cmp`, `is_in`, `count`, `if_else`, `__repr__`, and `icepool.from_cumulative`
//! (function.py).

use super::map::{contains, the_die};
use super::ops::{die_with_agains, matmul, py_iter, to_die, type_error};
use super::{PErr, PValue};
use crate::counts::OrderedMerge;
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
        None,
        PValue::Value(Value::None),
    )
}

/// `die.zero_outcome()`: `self.zero().outcomes()[0]`.
pub fn zero_outcome(die: PValue) -> Result<PValue, PErr> {
    let zero = the_die(die)?.zero()?;
    Ok(PValue::Value(zero.items()[0].0.clone()))
}

/// `die.sign()`: 1, -1 or 0 by comparing each outcome with its own `x * 0`.
pub fn sign(die: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(the_die(die)?.sign()?))
}

/// `die.cmp(other)`: `other` goes through `implicit_convert_to_die` first.
pub fn cmp(die: PValue, other: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    Ok(PValue::Die(die.cmp_die(&to_die(other)?)?))
}

/// `die.is_in(outcomes)`: `self.map(lambda x: x in outcomes)`. `map` calls the lambda on every outcome in order
/// (so the first one whose `in` raises decides the error) and passes the bools with their quantities to `Die`.
pub fn is_in(die: PValue, outcomes: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(is_in_die(&the_die(die)?, &outcomes)?))
}

fn is_in_die(die: &Die<Value>, outcomes: &PValue) -> Result<Die<Value>, PErr> {
    let mut data = OrderedMerge::default();
    for (x, q) in die.items() {
        data.add(Value::Bool(contains(outcomes, x)?), q.clone());
    }
    Ok(Die::from_mapping(data)?)
}

/// `die.count(rolls, outcomes)`: `rolls @ self.is_in(outcomes)`.
pub fn count(die: PValue, rolls: PValue, outcomes: PValue) -> Result<PValue, PErr> {
    let is_in = is_in_die(&the_die(die)?, &outcomes)?;
    matmul(rolls, PValue::Die(is_in))
}

/// `die.if_else(outcome_if_true, outcome_if_false, again_count=, again_depth=, again_end=)`: icepool maps the die
/// to `bool(x)` and then maps that through `{True: outcome_if_true, False: outcome_if_false}`. The bool die is
/// sorted (`False` first), so the final constructor sees `Die([outcome_if_false, outcome_if_true], [quantity of
/// False, quantity of True])`, leaving out a truth value no outcome has; the again arguments are forwarded to it.
pub fn if_else(
    die: PValue,
    if_true: PValue,
    if_false: PValue,
    again_count: Option<PValue>,
    again_depth: Option<PValue>,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let truths = die.unary_operator(|x| Ok(Value::Bool(x.truthy()?)))?;
    let (mut outcomes, mut times) = (vec![], vec![]);
    for (truth, q) in truths.items() {
        outcomes.push(if *truth == Value::Bool(true) {
            if_true.clone()
        } else {
            if_false.clone()
        });
        times.push(PValue::Value(Value::Int(q.clone().into())));
    }
    die_with_agains(
        PValue::List(outcomes),
        Some(PValue::List(times)),
        again_count,
        again_depth,
        again_end,
    )
}

/// `repr(die)`: `type(self).__qualname__ + '({' + 'outcome!r: quantity, ...' + '})'`, where the type is
/// `DieWithTruth` for the result of `==` / `!=`.
pub fn repr(die: PValue) -> Result<PValue, PErr> {
    let name = match &die {
        PValue::DieWithTruth(..) => "DieWithTruth",
        _ => "Die",
    };
    let die = the_die(die)?;
    let items = die
        .items()
        .iter()
        .map(|(o, q)| Ok(format!("{}: {q}", crate::value::pystr::py_repr(o)?)))
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(PValue::Value(Value::str(&format!(
        "{name}({{{}}})",
        items.join(", ")
    ))))
}
