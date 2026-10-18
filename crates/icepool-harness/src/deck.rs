//! Deck and Deal nodes (harness/grammar/deck.py): population/deck.py and generator/deal.py.
//!
//! The keep-like nodes (`pool_keep`, `pool_highest`, ...) and the evaluations take a `Deal` as they take a pool
//! (`pool::KeepGen`); this file has what is particular to decks and deals.

use std::borrow::Cow;

use dashu_int::IBig;

use super::map::{call, star_arg};
use super::ops::{
    die_arg, itemize, itemize_mapping, missing_attribute, py_iter, unhashable_list, wrong_type,
};
use super::statistics;
use super::{raise, PErr, PValue};
use icepool_core::value::{DieArg, PyDie, Formatted, Data, Num};
use icepool_core::value::{BinOp, Counter, CounterOp};
use icepool_core::{Error, Order};
use icepool_core::{Deal, Deck, Die, Int, Quantity, Value};

pub(super) fn the_deck(x: PValue) -> Result<Deck<Value>, PErr> {
    match x {
        PValue::Deck(d) => Ok(d),
        _ => missing_attribute("only decks have this method"),
    }
}

fn the_deal(x: PValue) -> Result<Deal<Value>, PErr> {
    match x {
        PValue::Deal(d) => Ok(d),
        _ => missing_attribute("only deals have this method"),
    }
}

fn int(i: impl Into<IBig>) -> Value {
    Value::Int(i.into())
}

fn is_one(t: &Value) -> bool {
    t.as_num()
        .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ONE)))
}

/// `icepool.Deck(outcomes, times)`.
pub fn deck(outcomes: PValue, times: PValue) -> Result<PValue, PErr> {
    let (objects, times) = itemize(outcomes, Some(times))?;
    // `contains_again(outcomes)`.
    if objects.iter().any(|x| matches!(x, PValue::Again(_))) {
        return raise(Error::InvalidArgument(
            "Again cannot be used with Decks.".into(),
        ));
    }
    new_deck(objects, &times)
}

/// The rest of `Deck.__new__` after `itemize`: a single deck with `times == 1` is returned as is, everything else
/// goes through `expand_args_for_deck`.
fn new_deck(objects: Vec<PValue>, times: &[Value]) -> Result<PValue, PErr> {
    if let ([PValue::Deck(d)], [t]) = (objects.as_slice(), times) {
        if is_one(t) {
            return Ok(PValue::Deck(d.clone()));
        }
    }
    // Every argument is expanded before any is merged. A tuple holding `Reroll` expands to `Reroll` itself
    // (`cartesian_product`), which `merge_duplicates` only touches (`subdata.items()`: AttributeError) if its dup
    // is not zero.
    let mut args = Vec::with_capacity(objects.len());
    let mut reroll_tuples = Vec::new();
    for (i, x) in objects.into_iter().enumerate() {
        if let PValue::Tuple(items) = &x {
            if items
                .iter()
                .any(|x| matches!(x, PValue::Reroll | PValue::Restart))
            {
                reroll_tuples.push(i);
                args.push(DieArg::Reroll);
                continue;
            }
        }
        args.push(die_arg(x)?);
    }
    let duplicates = args
        .iter()
        .zip(times)
        .map(|(a, t)| duplicate(a, t))
        .collect::<Result<Vec<_>, _>>()?;
    if reroll_tuples.iter().any(|&i| {
        !times[i]
            .as_num()
            .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ZERO)))
    }) {
        return missing_attribute("'RerollType' object has no attribute 'items'");
    }
    Ok(PValue::Deck(Deck::new(&args, &duplicates)?))
}

/// One element of `times` as `merge_duplicates` uses it (`itemize` has checked it is a non-negative number). A
/// non-int is skipped if it is zero (`subdup == 0`); otherwise it multiplies the argument's dups into a non-int,
/// which `Counts` rejects (`ValueError`), unless the argument contributes no cards at all.
fn duplicate(arg: &DieArg<Value>, t: &Value) -> Result<Quantity, PErr> {
    match t {
        Value::Int(i) => Ok(Quantity::try_from(i.clone()).expect("itemize checked")),
        Value::Bool(b) => Ok(Quantity::from(*b as u8)),
        t if t
            .as_num()
            .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ZERO))) =>
        {
            Ok(Quantity::ZERO)
        }
        t => {
            let contributes = match arg {
                DieArg::Outcome(_) => true,
                DieArg::Die(d) => d.items().iter().any(|(_, q)| !q.is_zero()),
                DieArg::Reroll => false,
            };
            if contributes {
                raise(Error::InvalidArgument(format!(
                    "Values must be ints, got {}",
                    t.type_name()
                )))
            } else {
                Ok(Quantity::ZERO)
            }
        }
    }
}

/// `Deck(data)` for the dict a `Population` method made (`self._new_type(data)`).
pub(super) fn deck_from_data(data: Data) -> Result<PValue, PErr> {
    let (keys, values): (Vec<_>, Vec<_>) =
        data.into_iter().map(|(o, q)| (PValue::Value(o), q)).unzip();
    let (objects, times) = itemize_mapping(keys, values, PValue::Value(int(1)))?;
    new_deck(objects, &times)
}

/// `Counter(other)` for the right-hand side of a deck operator: a mapping's items (a die or deck), an iterable's
/// elements counted, or nothing for `None` (`Counter(None)` is empty).
fn counter(other: PValue) -> Result<Counter<Value>, PErr> {
    match other {
        PValue::Deck(d) => Ok(Counter::from_population(d.items())),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(Counter::from_population(d.items())),
        PValue::Value(Value::None) => Ok(Counter::default()),
        x @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Value(Value::Tuple(_) | Value::Str(_))) => {
            let elements = py_iter(x)?
                .into_iter()
                .map(|e| match e {
                    PValue::Value(v) => Ok(v),
                    PValue::List(_) => unhashable_list(),
                    _ => Err(PErr::Unimplemented("counting dice or other objects".into())),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Counter::from_elements(elements))
        }
        PValue::Value(v) => wrong_type(format!("'{}' object is not iterable", v.type_name())),
        // Neither a mapping nor an iterable.
        PValue::Again(_) => wrong_type("'AgainExpression' object is not iterable"),
        PValue::Reroll | PValue::Restart => wrong_type("'RerollType' object is not iterable"),
        PValue::Ellipsis => wrong_type("'ellipsis' object is not iterable"),
        _ => Err(PErr::Unimplemented("Counter() of this object".into())),
    }
}

/// `a op b` for `op` one of `+ - & | ^` with a deck on at least one side.
///
/// A deck on the left handles the operator. With a deck only on the right, nothing a program can put on the left
/// handles a deck (sequences have no numeric slots, numbers return `NotImplemented`), so the deck's reflected
/// operator runs: `__radd__ = __add__` (and `&`, `|`) keep the deck's cards first; `-` is `__rsub__`; there is no
/// `__rxor__`.
pub fn deck_op(op: PValue, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let PValue::Value(Value::Str(op)) = op else {
        return Err(PErr::Unimplemented(
            "a deck operator that is not a str".into(),
        ));
    };
    let (deck, other, reflected) = match (a, b) {
        (PValue::Deck(d), other) => (d, other, false),
        (other @ (PValue::Value(_) | PValue::List(_) | PValue::Tuple(_)), PValue::Deck(d)) => {
            (d, other, true)
        }
        _ => return Err(PErr::Unimplemented("this deck operation".into())),
    };
    let op = match (&*op, reflected) {
        ("+", _) => CounterOp::Add,
        ("-", false) => CounterOp::Sub,
        ("-", true) => CounterOp::RSub,
        ("&", _) => CounterOp::And,
        ("|", _) => CounterOp::Or,
        ("^", false) => CounterOp::Xor,
        ("^", true) => return wrong_type("unsupported operand type(s) for ^: 'Deck'"),
        _ => return Err(PErr::Unimplemented(format!("deck operator {op}"))),
    };
    if super::expression::as_expression(&other).is_some() {
        return deck_op_expression(&deck, op, other);
    }
    Ok(PValue::Deck(deck.counter_op(op, &counter(other)?)?))
}

/// `deck <op> expr` for a multiset expression `expr`. `Counter(expr)` iterates it through `__getitem__` (`expr[0]`,
/// `expr[1]`, ...: `keep(i)`, evaluated into a die) until an index raises `IndexError`, so its keys are dice. No die
/// is among the deck's cards, so each operator treats one as a card the deck lacks: `+`, `|` and `^` (and
/// `__rsub__`) keep its count, `-` and `&` leave it at zero or below, which `+data` drops. `Deck(+data)` then
/// flattens the dice that are left into their outcomes, after the deck's own cards. Equal dice need not be counted
/// together: one key counted twice adds the same cards as two keys counted once each.
fn deck_op_expression(deck: &Deck<Value>, op: CounterOp, expr: PValue) -> Result<PValue, PErr> {
    use super::expression::{as_expression, keepable_mixture, MExpr};
    // A keep generator (or a mixture of them) checks its keep tuple first: an index past it is an `IndexError`
    // even if the dice are empty. Anything else gets the index only in the evaluation.
    let keep_tuple_bounded = as_expression(&expr).is_some_and(|e| {
        keepable_mixture(&e).is_some() || matches!(&e, MExpr::Gen(g) if !g.has_negative_keeps())
    });
    let mut dice = Vec::new();
    for i in 0i64.. {
        match super::pool::pool_keep(expr.clone(), PValue::Value(int(i))) {
            // Evaluated with the index, an expression without outcomes at all is the empty die at every index,
            // never an `IndexError`: Python never stops.
            Ok(PValue::Die(d) | PValue::DieWithTruth(d, _))
                if !keep_tuple_bounded && d.items().is_empty() =>
            {
                return Err(PErr::Unimplemented(
                    "iterating an expression without outcomes (Python never stops)".into(),
                ))
            }
            Ok(PValue::Die(d) | PValue::DieWithTruth(d, _)) => dice.push(d),
            // Inside a multiset function `expr[i]` is a raw result, never an `IndexError`: Python never stops.
            Ok(_) => {
                return Err(PErr::Unimplemented(
                    "iterating an expression whose items are not dice".into(),
                ))
            }
            Err(PErr::Raise(Error::IndexOutOfRange(_))) => break,
            Err(e) => return Err(e),
        }
    }
    let keeps_dice = matches!(
        op,
        CounterOp::Add | CounterOp::RSub | CounterOp::Or | CounterOp::Xor
    );
    let (mut args, mut times): (Vec<_>, Vec<_>) = deck
        .counter_items(op, &Counter::default())
        .into_iter()
        .map(|(o, q)| (DieArg::Outcome(o), q))
        .unzip();
    if keeps_dice {
        for d in dice {
            args.push(DieArg::Die(d));
            times.push(Quantity::ONE);
        }
    }
    Ok(PValue::Deck(Deck::new(&args, &times)?))
}

/// `deck * n`, `n * deck` (`op` `'r*'`), `deck // n`, `deck % n`: `multiply_quantities` and friends for an `int`
/// `n`, else `NotImplemented`: a die or an Again expression then answers with its own operator, anything else
/// refuses too (`TypeError`).
pub fn deck_mul(deck: PValue, op: PValue, n: PValue) -> Result<PValue, PErr> {
    let PValue::Value(Value::Str(op)) = &op else {
        return Err(PErr::Unimplemented(
            "a deck operator that is not a str".into(),
        ));
    };
    let binop = match &**op {
        "*" | "r*" => BinOp::Mul,
        "//" => BinOp::FloorDiv,
        "%" => BinOp::Mod,
        _ => return Err(PErr::Unimplemented(format!("deck operator {op}"))),
    };
    let n = match n {
        PValue::Value(n @ (Value::Int(_) | Value::Bool(_))) => n,
        // `n * deck` is the die's or expression's `__mul__`, `deck * n` its `__rmul__` (and so on).
        n @ (PValue::Die(_) | PValue::DieWithTruth(..) | PValue::Again(_)) if &**op == "r*" => {
            return super::ops::binary(binop, n, deck)
        }
        n @ (PValue::Die(_) | PValue::DieWithTruth(..) | PValue::Again(_)) => {
            return super::ops::binary(binop, deck, n)
        }
        // A multiset expression's `__rmul__` takes an int only; a deck's and a multiset tuple expression's are none.
        PValue::Value(_)
        | PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_)
        | PValue::Deck(_) => return wrong_type("unsupported operand type(s) for Deck"),
        _ => return Err(PErr::Unimplemented("this deck operand".into())),
    };
    let deck = the_deck(deck)?;
    let cards = deck.population();
    let data = match binop {
        BinOp::Mul => cards.multiply_quantities(&n)?,
        BinOp::FloorDiv => cards.divide_quantities(&n)?,
        _ => Some(cards.modulo_quantities(&n)?),
    };
    match data {
        None => Ok(PValue::Deck(deck)),
        Some(data) => deck_from_data(data),
    }
}

/// `deck.map(repl, star=star)`: each outcome mapped (a mapping leaves unmapped outcomes as they are), then
/// `Deck(results, times=self.quantities())`, so dice among the results are flattened into cards.
pub fn deck_map(deck: PValue, repl: PValue, star: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let PValue::Func(f) = repl else {
        return Err(PErr::Unimplemented("deck.map with this repl".into()));
    };
    // A mapping ignores `star`.
    let star = match f.positional() {
        Some(_) => match star_arg(&star)? {
            Some(star) => star,
            None => f.infer_star(1)?,
        },
        None => false,
    };
    let results = deck
        .outcomes()
        .map(|o| call(&f, star, vec![PValue::Value(o.clone())], &[]))
        .collect::<Result<Vec<_>, _>>()?;
    let times: Vec<PValue> = deck
        .items()
        .iter()
        .map(|(_, q)| PValue::Value(int(q.clone())))
        .collect();
    self::deck(PValue::List(results), PValue::List(times))
}

/// `deck.sequence(deals)`: the possible sequences of `deals` cards dealt one by one, as a die of tuples.
///
/// Python builds them in `_sequence_cache`, starting from `Die([()])`: each step maps every sequence so far to the
/// die of the cards left (`Die(self - curr)`) appended to it. The loop variable is the only binding of `result`,
/// so `deals == 0` raises `UnboundLocalError` (copied, see DIVERGENCES.md).
pub fn deck_sequence(deck: PValue, deals: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let deals = match deals {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        _ => return Err(PErr::Unimplemented("non-int deals".into())),
    };
    if deals < IBig::ZERO {
        return raise(Error::InvalidArgument(
            "The number of cards dealt cannot be negative.".into(),
        ));
    }
    if deals == IBig::ZERO {
        return raise(Error::InvalidArgument(
            "UnboundLocalError: cannot access local variable 'result'".into(),
        ));
    }
    let deals = usize::try_from(deals)
        .ok()
        .filter(|&d| d <= 16)
        .ok_or_else(|| PErr::Unimplemented("a very long deck sequence".into()))?;
    let mut result = Die::from_dict(vec![(Value::tuple(vec![]), Quantity::ONE)])?;
    for _ in 0..deals {
        // `self._sequence_cache[i - 1].map(transition)`: `Die(transitions, quantities)`.
        let mut transitions = Vec::with_capacity(result.len());
        for (curr, _) in result.items() {
            let Value::Tuple(cards) = curr else {
                unreachable!("sequences are tuples")
            };
            // `remaining = icepool.Die(self - curr)`.
            let remaining = deck.counter_op(
                CounterOp::Sub,
                &Counter::from_elements(cards.iter().cloned()),
            )?;
            // `icepool.map(lambda curr, next: curr + (next, ), curr, remaining)`.
            let (outcomes, quantities): (Vec<_>, Vec<_>) = remaining
                .items()
                .iter()
                .map(|(next, q)| {
                    let mut seq = cards.to_vec();
                    seq.push(next.clone());
                    (DieArg::Outcome(Value::tuple(seq)), q.clone())
                })
                .unzip();
            transitions.push(DieArg::Die(Die::new(outcomes, quantities)?));
        }
        let quantities = result.items().iter().map(|(_, q)| q.clone()).collect();
        result = Die::new(transitions, quantities)?;
    }
    Ok(PValue::Die(result))
}

/// `(tuple(deck.items()), deck.size(), len(deck))`.
pub fn deck_info(deck: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    Ok(PValue::Value(Value::tuple(vec![
        items(&deck),
        int(deck.size()),
        int(deck.len()),
    ])))
}

fn items(deck: &Deck<Value>) -> Value {
    Value::tuple(
        deck.items()
            .iter()
            .map(|(o, q)| Value::tuple(vec![o.clone(), int(q.clone())]))
            .collect(),
    )
}

/// `deck._pop_max()` if `max` else `deck._pop_min()`, as `(tuple(popped.items()), quantity)`.
pub fn deck_pop(deck: PValue, max: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let (popped, q) = if super::catalogue::truthy(&max)? {
        deck.pop_max()?
    } else {
        deck.pop_min()?
    };
    Ok(PValue::Value(Value::tuple(vec![items(&popped), int(q)])))
}

/// `getattr(deck, stat)()`: the `Population` statistics, the same as a die's with these quantities.
pub fn deck_stat(deck: PValue, stat: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    statistics::die_stat(PValue::Die(deck.population().clone()), stat)
}

/// `deck.pad_to_denominator(denominator, outcome=outcome)`.
pub fn deck_pad_to_denominator(
    deck: PValue,
    denominator: PValue,
    outcome: PValue,
) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let (PValue::Value(denominator), PValue::Value(outcome)) = (denominator, outcome) else {
        return Err(PErr::Unimplemented("non-value arguments".into()));
    };
    deck_from_data(
        deck.population()
            .pad_to_denominator(&denominator, &outcome)?,
    )
}

/// `deck.deal(hand_size)` for an int: a `Deal`. Anything else is an iterable of hand sizes: a `MultiDeal`.
pub fn deal(deck: PValue, hand_size: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let hand_size = match hand_size {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        hand_sizes => return super::multi_deal::deal(deck, hand_sizes),
    };
    Ok(PValue::Deal(Deal::new(deck, &hand_size)?))
}

fn int_tuple(xs: &[Int]) -> Value {
    Value::tuple(xs.iter().map(int).collect())
}

/// `(tuple(deal.deck().items()), deal.keep_tuple(), deal.hand_size(), deal.keep_size(), deal.denominator(),
/// tuple(deal.outcomes()))`.
pub fn deal_info(deal: PValue) -> Result<PValue, PErr> {
    let deal = the_deal(deal)?;
    Ok(PValue::Value(Value::tuple(vec![
        items(deal.deck()),
        int_tuple(deal.keep_tuple()),
        int(deal.hand_size()),
        int(deal.keep_size()),
        int(deal.denominator()),
        Value::tuple(deal.deck().outcomes().cloned().collect()),
    ])))
}

/// What `deal._make_source().pop(Order(order), outcome)` yields, as `(deck items, keep_tuple, count, weight)`.
pub fn deal_pop(deal: PValue, order: PValue, outcome: PValue) -> Result<PValue, PErr> {
    let deal = the_deal(deal)?;
    let order = match &order {
        PValue::Value(v) => [Order::Ascending, Order::Descending, Order::Any]
            .into_iter()
            .find(|o| *v == int(o.value())),
        _ => None,
    }
    .ok_or_else(|| PErr::Raise(Error::InvalidArgument("not a valid Order".into())))?;
    let PValue::Value(outcome) = outcome else {
        return Err(PErr::Unimplemented("popping a non-value outcome".into()));
    };
    let popped = deal.source().pop(order, &outcome)?;
    Ok(PValue::Value(Value::tuple(
        popped
            .into_iter()
            .map(|(source, count, weight)| {
                Value::tuple(vec![
                    items(source.deck()),
                    int_tuple(source.keep_tuple()),
                    int(count),
                    int(weight),
                ])
            })
            .collect(),
    )))
}

/// The rest of `Deck`'s methods (`deck_method`): the multi-argument forms of the operators, `deck[outcome]`,
/// `keys()`, `values()` and iteration.
pub fn deck_method(deck: PValue, name: PValue, arg: PValue) -> Result<PValue, PErr> {
    let deck = the_deck(deck)?;
    let PValue::Value(Value::Str(name)) = name else {
        return Err(PErr::Unimplemented(
            "a deck method that is not a str".into(),
        ));
    };
    match &*name {
        // `functools.reduce(operator.add, args, initial=self)`: `reduce` takes no keyword arguments before
        // Python 3.14, so these always raise there, even without arguments (DIVERGENCES.md).
        "additive_union" | "difference" | "intersection" | "union" => raise(Error::WrongType(
            "reduce() takes no keyword arguments".into(),
        )),
        // `return self ^ other`.
        "symmetric_difference" => deck_op(PValue::Value(Value::str("^")), PValue::Deck(deck), arg),
        // `self._data[outcome]`: a dict lookup.
        "getitem" => match arg {
            PValue::Value(key) => match deck.population().position(&key) {
                Some(i) => Ok(PValue::Value(int(deck.items()[i].1.clone()))),
                None => raise(Error::MissingKey(format!("{key}"))),
            },
            PValue::List(_) => unhashable_list(),
            _ => Err(PErr::Unimplemented("deck[] of this object".into())),
        },
        "keys" | "iter" => Ok(PValue::Value(Value::tuple(
            deck.outcomes().cloned().collect(),
        ))),
        "values" => Ok(PValue::Value(Value::tuple(
            deck.items().iter().map(|(_, q)| int(q.clone())).collect(),
        ))),
        _ => Err(PErr::Unimplemented(format!("Deck.{name}"))),
    }
}

/// `(repr(deal), str(deal))` for a `Deal` or `MultiDeal`: `str` is a header line, then `str(self.deck())`.
pub fn deal_str(deal: PValue) -> Result<PValue, PErr> {
    let (repr, text) = deal_repr_and_str(&deal)?;
    Ok(PValue::Value(Value::tuple(vec![
        Value::str(&repr),
        Value::str(&text),
    ])))
}

/// `repr()` and `str()` of a `Deal` or `MultiDeal`.
pub(super) fn deal_repr_and_str(deal: &PValue) -> Result<(String, String), PErr> {
    use super::wire::deck_repr;
    match deal {
        PValue::Deal(d) => Ok((
            format!(
                "Deal({}, hand_size={})",
                deck_repr(d.deck())?,
                d.hand_size()
            ),
            deal_text(d.deck(), d.hand_size())?,
        )),
        PValue::MultiDeal(md) => {
            let groups = super::multi_deal::hand_groups(md.hand_groups());
            let header = format!(
                "MultiDeal of hand_groups={}",
                icepool_core::value::pystr::py_repr(&groups)?
            );
            Ok((
                super::multi_deal::repr(md, &deck_repr(md.deck())?)?,
                format!("{header} from deck:\n{}", deck_str(md.deck())?),
            ))
        }
        _ => missing_attribute("only deals have this method"),
    }
}

/// `str(deal)` of a `Deal` of `hand_size` cards (also an inner of a `CompoundKeepGenerator`, by its source).
pub(super) fn deal_text(deck: &Deck<Value>, hand_size: usize) -> Result<String, PErr> {
    Ok(format!(
        "Deal of hand_size={hand_size} from deck:\n{}",
        deck_str(deck)?
    ))
}

/// `str(deck)`: its table.
pub(super) fn deck_str(deck: &Deck<Value>) -> Result<String, PErr> {
    let Formatted::Text(table) =
        icepool_core::value::format_population(deck.population(), "Deck", "", false)?
    else {
        unreachable!("only csv_table asks for a table")
    };
    Ok(table)
}
