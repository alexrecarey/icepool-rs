//! Die methods that manage quantities and outcomes without a callable: `zero`, `zero_outcome`, `simplify`,
//! `_pop_min` / `_pop_max`, `sign`, `cmp`, `is_in`, `count`, `if_else`, `__repr__`, and `icepool.from_cumulative`
//! (function.py).

use super::function::harmonize;
use super::map::{contains, the_die};
use super::ops::{die_with_agains, matmul, py_iter, to_die, type_error};
use super::{PErr, PValue};
use crate::counts::OrderedMerge;
use crate::population::Comparison;
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

/// `icepool.from_cumulative(outcomes, cumulative, reverse=reverse)`: each outcome's quantity is its cumulative
/// quantity minus the previous one, collected in a `dict` (a repeated outcome keeps its first position and its
/// last difference) and passed to `Die`, which raises for a negative difference. The cumulative values are `int`
/// quantities, or dice representing the cumulative distribution at that point (if the first one is a die): those
/// are harmonized to a common denominator and each counts its quantity `!= False`.
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
    let cumulative: Vec<PValue> = match cumulative.first() {
        None => {
            return Err(PErr::Raise(crate::Error::Index(
                "list index out of range".into(),
            )))
        }
        Some(PValue::Value(Value::Int(_) | Value::Bool(_))) => cumulative,
        // `harmonize_denominators(cumulative)` (later elements may be outcomes, made into dice), then
        // `die.quantity('!=', False)`.
        Some(PValue::Die(_) | PValue::DieWithTruth(..)) => {
            harmonize(PValue::List(cumulative), PValue::Value(Value::None))?
                .iter()
                .map(|die| {
                    let q = die.try_quantity(Comparison::Ne, &Value::Bool(false))?;
                    Ok(PValue::Value(Value::Int(q.into())))
                })
                .collect::<Result<_, crate::Error>>()?
        }
        Some(_) => return type_error("Unsupported type <class 'list'> for cumulative values."),
    };
    let mut keys: Vec<PValue> = Vec::new();
    let mut values: Vec<PValue> = Vec::new();
    let mut prev = Value::from(0);
    for (outcome, quantity) in outcomes.into_iter().zip(cumulative) {
        let quantity = match quantity {
            PValue::Value(q) => q,
            // An `int` first and a die later: its difference is a die, which `Die(d)` then compares with
            // `times < 0` (after any earlier negative difference, which raises ValueError), and a die has no truth
            // value. Either way it raises.
            PValue::Die(_) | PValue::DieWithTruth(..) => {
                return type_error(
                    "A `Die` only has a truth value if it is the result of == or !=.",
                )
            }
            _ => return Err(PErr::Unimplemented("non-number cumulative quantity".into())),
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
        PValue::Die(_) => "Die",
        // `repr()` takes anything: a "die" node may give a value (`d1.sequence(0)` is `()`).
        PValue::Value(v) => return Ok(PValue::Value(Value::str(&crate::value::pystr::py_repr(v)?))),
        _ => return Err(PErr::Unimplemented("repr() of this kind of object".into())),
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

/// Die's mapping protocol and its `Counts` views (`CountsKeysView` / `CountsValuesView` / `CountsItemsView`, which
/// are both views and sequences): `die[arg]`, `arg in die`, `len(die)`, `tuple(die)`, `tuple(die.keys())` (and
/// values / items), `view[arg]` for an int or a `[start, stop, step]` slice, `view == arg`, and `min_outcome()` /
/// `max_outcome()` / `is_empty()`.
pub fn die_method(die: PValue, name: PValue, arg: PValue) -> Result<PValue, PErr> {
    use crate::value::builtins::{getitem, Index};
    let die = the_die(die)?;
    let PValue::Value(Value::Str(name)) = name else {
        return Err(PErr::Unimplemented("a die method that is not a str".into()));
    };
    let int = |n: &crate::Quantity| Value::Int(n.clone().into());
    // `self._mapping._keys` / `_values` / `_items`: tuples.
    let view = |v: &str| -> Value {
        Value::tuple(match v {
            "keys" => die.outcomes().cloned().collect(),
            "values" => die.items().iter().map(|(_, q)| int(q)).collect(),
            _ => die
                .items()
                .iter()
                .map(|(o, q)| Value::tuple(vec![o.clone(), int(q)]))
                .collect(),
        })
    };
    // `self._data[key]` / `key in self._data`: a dict lookup, which hashes the key first.
    let lookup = |arg: PValue| -> Result<Option<usize>, PErr> {
        match arg {
            PValue::Value(key) => {
                key.check_hashable()?;
                Ok(die.position(&key))
            }
            PValue::List(_) => type_error("unhashable type: 'list'"),
            _ => Err(PErr::Unimplemented("die[] of this object".into())),
        }
    };
    let value = |v: Value| Ok(PValue::Value(v));
    match &*name {
        "getitem" => match lookup(arg.clone())? {
            Some(i) => value(int(&die.items()[i].1)),
            None => super::raise(crate::error::Error::Key(match arg {
                PValue::Value(key) => format!("{key}"),
                _ => String::new(),
            })),
        },
        "contains" => value(Value::Bool(lookup(arg)?.is_some())),
        "len" => value(Value::from(die.len() as i64)),
        "iter" => value(view("keys")),
        "keys" | "values" | "items" => value(view(&name)),
        "keys_index" | "values_index" | "items_index" => {
            let index = match arg {
                PValue::Value(i) => Index::Item(i),
                PValue::List(s) => {
                    let [a, b, c]: [PValue; 3] = s
                        .try_into()
                        .map_err(|_| PErr::Unimplemented("a slice that is not 3 values".into()))?;
                    let v = |x: PValue| match x {
                        PValue::Value(v) => Ok(v),
                        _ => Err(PErr::Unimplemented(
                            "a slice bound that is not a value".into(),
                        )),
                    };
                    Index::Slice(v(a)?, v(b)?, v(c)?)
                }
                _ => return Err(PErr::Unimplemented("this view index".into())),
            };
            value(getitem(&view(&name[..name.len() - 6]), &index)?)
        }
        // `self._mapping._keys == other`: a tuple against the argument (a list is never equal to a tuple).
        "keys_eq" | "values_eq" | "items_eq" => match arg {
            PValue::List(_) => value(Value::Bool(false)),
            arg => value(Value::Bool(super::catalogue::py_eq(
                &PValue::Value(view(&name[..name.len() - 3])),
                &arg,
            )?)),
        },
        // `self.outcomes()[0]` / `[-1]`: IndexError for an empty die.
        "min_outcome" | "max_outcome" => {
            let outcome = if &*name == "min_outcome" {
                die.items().first()
            } else {
                die.items().last()
            };
            match outcome {
                Some((o, _)) => value(o.clone()),
                None => super::raise(crate::error::Error::Index(
                    "tuple index out of range".into(),
                )),
            }
        }
        "is_empty" => value(Value::Bool(die.is_empty())),
        _ => Err(PErr::Unimplemented(format!("Die.{name}"))),
    }
}
