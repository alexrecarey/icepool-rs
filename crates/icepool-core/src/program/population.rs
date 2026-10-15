//! Nodes for what population/base.py and die.py had left in Python under the facade (S5c,
//! harness/grammar/population.py): `append` / `remove` / `to_one_hot` of a die or deck, the deck forms of `zero` /
//! `split`, and `Die.mean_time_to_sum`.

use super::deck::{deck_from_data, the_deck};
use super::map::{contains, select, the_die};
use super::ops::{py_iter, type_error, weights_die};
use super::{PErr, PValue};
use crate::population::Data;
use crate::value::builtins::py_len;
use crate::{Die, Value};

/// A die or a deck, whose `Population` methods are the same on its cards; `true` for a deck.
fn the_population(p: PValue) -> Result<(Die<Value>, bool), PErr> {
    match p {
        PValue::Deck(d) => Ok((d.population().clone(), true)),
        p => Ok((the_die(p)?, false)),
    }
}

/// `self._new_type(data)`: `Deck(data)` or `Die(data)`.
fn new_population(is_deck: bool, data: Data) -> Result<PValue, PErr> {
    if is_deck {
        deck_from_data(data)
    } else {
        let (keys, values) = data
            .into_iter()
            .map(|(o, q)| (PValue::Value(o), PValue::Value(q)))
            .unzip();
        weights_die(keys, values)
    }
}

/// An outcome used as a `Counter` key: a plain value (a list is unhashable).
fn key(outcome: PValue) -> Result<Value, PErr> {
    match outcome {
        PValue::Value(v) => {
            v.check_hashable()?;
            Ok(v)
        }
        PValue::List(_) => type_error("unhashable type: 'list'"),
        _ => Err(PErr::Unimplemented(
            "append / remove of an outcome that is not a plain value".into(),
        )),
    }
}

/// `population.append(outcome, quantity)`.
pub fn append(population: PValue, outcome: PValue, quantity: PValue) -> Result<PValue, PErr> {
    let (die, is_deck) = the_population(population)?;
    let PValue::Value(quantity) = quantity else {
        return Err(PErr::Unimplemented("a quantity that is not a value".into()));
    };
    new_population(is_deck, die.append(&key(outcome)?, &quantity)?)
}

/// `population.remove(outcome, quantity)` (`None`: all of it).
pub fn remove(population: PValue, outcome: PValue, quantity: PValue) -> Result<PValue, PErr> {
    let (die, is_deck) = the_population(population)?;
    let quantity = match quantity {
        PValue::Value(Value::None) => None,
        PValue::Value(q) => Some(q),
        _ => return Err(PErr::Unimplemented("a quantity that is not a value".into())),
    };
    new_population(is_deck, die.remove(&key(outcome)?, quantity.as_ref())?)
}

/// `outcomes.index(x)`: the first position whose element `in` finds equal to `x` (a str finds a substring and
/// gives its position in characters); `None` where `x in outcomes` is false.
fn index_of(outcomes: &PValue, x: &Value) -> Result<Option<i64>, PErr> {
    if !contains(outcomes, x)? {
        return Ok(None);
    }
    let position = match outcomes {
        PValue::List(items) | PValue::Tuple(items) => {
            let x = PValue::Value(x.clone());
            let mut found = None;
            for (i, item) in items.iter().enumerate() {
                if super::catalogue::py_eq(item, &x)? {
                    found = Some(i);
                    break;
                }
            }
            found
        }
        PValue::Value(Value::Tuple(items)) => {
            let mut found = None;
            for (i, item) in items.iter().enumerate() {
                if item.eq_op(x, false)?.truthy()? {
                    found = Some(i);
                    break;
                }
            }
            found
        }
        PValue::Value(Value::Str(s)) => match x {
            Value::Str(sub) => s.find(&**sub).map(|b| s[..b].chars().count()),
            _ => None,
        },
        PValue::Die(_) | PValue::DieWithTruth(..) => {
            return type_error("AttributeError: 'Die' object has no attribute 'index'")
        }
        _ => {
            return Err(PErr::Unimplemented(
                "to_one_hot over this kind of sequence".into(),
            ))
        }
    };
    // `in` found it, so `index` does too.
    Ok(position.map(|i| i as i64))
}

/// `len(outcomes)`.
fn sequence_len(outcomes: &PValue) -> Result<usize, PErr> {
    match outcomes {
        PValue::List(items) | PValue::Tuple(items) => Ok(items.len()),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d.len()),
        PValue::Value(v) => Ok(py_len(v)?),
        _ => Err(PErr::Unimplemented(
            "to_one_hot over this kind of sequence".into(),
        )),
    }
}

/// `population.to_one_hot(outcomes)`: `None` means `population.outcomes()`. `len(outcomes)` is only taken once
/// there is an outcome to convert, as Python takes it inside the loop.
pub fn to_one_hot(population: PValue, outcomes: PValue) -> Result<PValue, PErr> {
    let (die, is_deck) = the_population(population)?;
    let (len, positions) = match outcomes {
        PValue::Value(Value::None) => (die.len(), (0..die.len() as i64).map(Some).collect()),
        outcomes => {
            let len = if die.is_empty() {
                0
            } else {
                sequence_len(&outcomes)?
            };
            let positions = die
                .outcomes()
                .map(|o| index_of(&outcomes, o))
                .collect::<Result<Vec<_>, _>>()?;
            (len, positions)
        }
    };
    let items = die.one_hot_items(len, &positions)?;
    Ok(if is_deck {
        PValue::Deck(crate::Deck::from_dict(items)?)
    } else {
        PValue::Die(Die::from_dict(items)?)
    })
}

/// `deck.zero()`.
pub fn deck_zero(deck: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Deck(the_deck(deck)?.zero()?))
}

/// `deck.split(which, star=star)`: a tuple of two decks.
pub fn deck_split(deck: PValue, which: PValue, star: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let mask = select(deck.population(), &which, &star)?;
    let (selected, rest) = deck.split(&mask)?;
    Ok(PValue::tuple(vec![
        PValue::Deck(selected),
        PValue::Deck(rest),
    ]))
}

/// `tuple(die.mean_time_to_sum(target) for target in targets)`: the calls share the die's cache, as on one Python
/// die object.
pub fn mean_time_to_sum(die: PValue, targets: PValue) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let mut cache = vec![Value::Fraction(dashu_ratio::RBig::ZERO)];
    let results = py_iter(targets)?
        .into_iter()
        .map(|t| match t {
            PValue::Value(t) => Ok(PValue::Value(die.mean_time_to_sum(&mut cache, &t)?)),
            _ => Err(PErr::Unimplemented("a target that is not a value".into())),
        })
        .collect::<Result<Vec<_>, PErr>>()?;
    Ok(PValue::tuple(results))
}
