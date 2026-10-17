//! MultiDeal nodes (harness/grammar/deck.py): generator/multi_deal.py. `deck.deal(hand_sizes)` with an iterable,
//! `deck.deal_groups(*hand_groups)`, reading a multi-deal and popping its source. Evaluations take a multi-deal as
//! one of their inputs (evaluate.rs `catalogue`).

use dashu_int::IBig;

use super::deck::the_deck;
use super::ops::{missing_attribute, py_iter, wrong_type};
use super::{raise, PErr, PValue};
use icepool_core::Error;
use icepool_core::Order;
use icepool_core::{Deck, MultiDeal, Value};

fn the_multi_deal(x: PValue) -> Result<MultiDeal<Value>, PErr> {
    match x {
        PValue::MultiDeal(d) => Ok(d),
        _ => missing_attribute("only multi-deals have this method"),
    }
}

fn int(i: impl Into<IBig>) -> Value {
    Value::Int(i.into())
}

/// A hand size or group size: an exact `int`. (A `bool` would work in Python but show up as `True` in the
/// hand sizes and the repr, which the core does not keep.)
fn hand_int(x: PValue) -> Result<IBig, PErr> {
    match x {
        PValue::Value(Value::Int(i)) => Ok(i),
        _ => Err(PErr::Unimplemented(
            "a MultiDeal hand size that is not an int".into(),
        )),
    }
}

/// `deck.deal(hand_sizes)` for anything but an `int`: `MultiDeal(deck, tuple((hand_size, 1) for hand_size in
/// hand_sizes))`.
pub fn deal(deck: Deck<Value>, hand_sizes: PValue) -> Result<PValue, PErr> {
    let groups = match hand_sizes {
        x @ (PValue::List(_) | PValue::Value(Value::Tuple(_) | Value::Str(_))) => py_iter(x)?
            .into_iter()
            .map(|h| match h {
                // `sum(hand_size * group_size ...)` starts from 0: `0 + 'a'` is a TypeError.
                PValue::Value(Value::Str(_)) => {
                    wrong_type("unsupported operand type(s) for +: 'int' and 'str'")
                }
                h => Ok((hand_int(h)?, IBig::ONE)),
            })
            .collect::<Result<Vec<_>, _>>()?,
        x @ PValue::Value(_) => {
            py_iter(x)?;
            unreachable!("only tuples and strs among values are iterable")
        }
        _ => return Err(PErr::Unimplemented("these MultiDeal hand sizes".into())),
    };
    Ok(PValue::MultiDeal(MultiDeal::new(deck, &groups)?))
}

/// `deck.deal_groups(*hand_groups)` with each group a `(hand_size, group_size)` sequence.
pub fn deal_groups(deck: PValue, hand_groups: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let groups = py_iter(hand_groups)?
        .into_iter()
        .map(|g| {
            let g = py_iter(g)?;
            // `for hand_size, group_size in self._hand_groups`: unpacking anything but a pair is a ValueError.
            let [h, n]: [PValue; 2] = g.try_into().map_err(|g: Vec<PValue>| {
                PErr::Raise(Error::InvalidArgument(format!(
                    "expected 2 values to unpack, got {}",
                    g.len()
                )))
            })?;
            Ok((h, n))
        })
        .collect::<Result<Vec<_>, PErr>>()?
        .into_iter()
        .map(|(h, n)| Ok((hand_int(h)?, hand_int(n)?)))
        .collect::<Result<Vec<_>, PErr>>()?;
    Ok(PValue::MultiDeal(MultiDeal::new(deck, &groups)?))
}

fn int_tuple(xs: impl IntoIterator<Item = usize>) -> Value {
    Value::tuple(xs.into_iter().map(int).collect())
}

fn items(deck: &Deck<Value>) -> Value {
    Value::tuple(
        deck.items()
            .iter()
            .map(|(o, q)| Value::tuple(vec![o.clone(), int(q.clone())]))
            .collect(),
    )
}

pub(super) fn hand_groups(groups: &[(usize, usize)]) -> Value {
    Value::tuple(groups.iter().map(|&(h, g)| int_tuple([h, g])).collect())
}

/// `(tuple(deck items), hand_sizes(), total_cards_dealt(), len(md), denominator(), tuple(outcomes()))`.
pub fn multi_deal_info(md: PValue) -> Result<PValue, PErr> {
    let md = the_multi_deal(md)?;
    Ok(PValue::Value(Value::tuple(vec![
        items(md.deck()),
        int_tuple(md.hand_sizes()),
        int(md.total_cards_dealt()),
        int(md.len()),
        int(md.denominator()),
        Value::tuple(md.deck().outcomes().cloned().collect()),
    ])))
}

/// What `md._make_source().pop(Order(order), outcome)` yields, as `(deck items, hand_groups, counts, weight)`.
pub fn multi_deal_pop(md: PValue, order: PValue, outcome: PValue) -> Result<PValue, PErr> {
    let md = the_multi_deal(md)?;
    let order = match &order {
        PValue::Value(v) => [Order::Ascending, Order::Descending, Order::Any]
            .into_iter()
            .find(|o| *v == int(o.value())),
        _ => None,
    };
    let Some(order) = order else {
        return raise(Error::InvalidArgument("not a valid Order".into()));
    };
    let PValue::Value(outcome) = outcome else {
        return Err(PErr::Unimplemented("popping a non-value outcome".into()));
    };
    let popped = md.source().pop(order, &outcome)?;
    Ok(PValue::Value(Value::tuple(
        popped
            .into_iter()
            .map(|(source, counts, weight)| {
                Value::tuple(vec![
                    items(source.deck()),
                    hand_groups(source.hand_groups()),
                    Value::tuple(counts.iter().map(int).collect()),
                    int(weight),
                ])
            })
            .collect(),
    )))
}

/// `repr(md)`: `MultiDeal(<repr of the deck>, hand_groups=<the hand groups tuple>)`.
pub(super) fn repr(md: &MultiDeal<Value>, deck_repr: &str) -> icepool_core::Result<String> {
    Ok(format!(
        "MultiDeal({deck_repr}, hand_groups={})",
        icepool_core::value::pystr::py_repr(&hand_groups(md.hand_groups()))?
    ))
}
