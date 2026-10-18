//! The nodes that make dice: `d`, `Die(...)` in its forms, `Again` and explode (harness/grammar/basic.py,
//! construction.py, again.py), with the Python argument handling of `Die.__new__` they share with the
//! other nodes (`creation_args.itemize`, `expand_arg`, `iter`).

use super::{negative_times, unhashable_list, wrong_type, zero};
use crate::again::{self};
use crate::map;
use crate::{raise, Args, PErr, PValue};
use dashu_int::IBig;
use dashu_ratio::RBig;
use icepool_core::value::DieArg;
use icepool_core::value::{BinOp, Num};
use icepool_core::Error;
use icepool_core::{Die, Quantity, Value};
use rustc_hash::FxHashMap;
use std::borrow::Cow;

use super::Dispatch;

/// The dice nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "d" => args.with(["sides"], |[sides]| d(sides)),
        "die" | "die_typed" | "die_vectors" | "die_symbols" => {
            args.with(["outcomes"], |[outcomes]| die(outcomes, None))
        }
        "die_times" | "mixture" => args.with(["outcomes", "times"], |[outcomes, times]| {
            die(outcomes, Some(times))
        }),
        "weights" => args.with(["items"], |[items]| weights(items)),
        "Again" => args.with([], |[]| {
            // `icepool.Again = AgainExpression(is_additive=True)`.
            Ok(PValue::Again(std::sync::Arc::new(
                icepool_core::value::AgainExpression::Again { is_additive: true },
            )))
        }),
        "again" => args.with(["outcomes", "depth", "end"], |[outcomes, depth, end]| {
            // `again_depth=None` is the default (depth mode, depth 1).
            let depth = match depth {
                PValue::Value(Value::None) => None,
                d => Some(d),
            };
            die_with_agains(outcomes, None, None, depth, end)
        }),
        "Restart" => args.with([], |[]| Ok(PValue::Restart)),
        "again_count" => args.with(
            ["outcomes", "count", "depth", "end"],
            |[outcomes, count, depth, end]| {
                // `None` is the default for both.
                let some = |x: PValue| match x {
                    PValue::Value(Value::None) => None,
                    x => Some(x),
                };
                die_with_agains(outcomes, None, some(count), some(depth), end)
            },
        ),
        "explode" => args.with(
            ["die", "which", "star", "depth", "end"],
            |[die, which, star, depth, end]| again::explode(die, which, star, depth, end),
        ),
        _ => return Err(args),
    })
}

/// `icepool.d(sides)`: `sides` must be an `int` (a `bool` is one).
pub(crate) fn d(sides: PValue) -> Result<PValue, PErr> {
    let sides = match sides {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        _ => return wrong_type("sides must be an int."),
    };
    // Beyond i64 Python would run out of memory long before finishing.
    let sides =
        i64::try_from(sides).map_err(|_| PErr::Raise(Error::Overflow("sides too large".into())))?;
    Ok(PValue::Die(icepool_core::try_d(sides)?))
}

/// Python's `iter(x)` over a program value: a list, a tuple's elements, a str's characters, a die's outcomes.
pub(crate) fn py_iter(x: PValue) -> Result<Vec<PValue>, PErr> {
    Ok(match x {
        PValue::DieWithTruthError(e, _) => return Err(e.into()),
        PValue::List(xs) | PValue::Tuple(xs) => xs,
        PValue::Value(Value::Tuple(t)) => t.iter().cloned().map(PValue::Value).collect(),
        PValue::Value(Value::Str(s)) => s
            .chars()
            .map(|c| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => {
            d.outcomes().cloned().map(PValue::Value).collect()
        }
        PValue::Value(Value::Vector(v)) => v.data()?.iter().cloned().map(PValue::Value).collect(),
        // A mapping: its keys, in the dict's order.
        PValue::Value(Value::Symbols(s)) => s
            .items()
            .iter()
            .map(|(c, _)| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Value(v) => {
            return wrong_type(format!("'{}' object is not iterable", v.type_name()))
        }
        PValue::Reroll | PValue::Restart => {
            return wrong_type("'RerollType' object is not iterable")
        }
        PValue::Break(_) => return wrong_type("'Break' object is not iterable"),
        PValue::Func(_) => return Err(PErr::Unimplemented("iterating a function".into())),
        PValue::Again(_) => return wrong_type("'AgainExpression' object is not iterable"),
        PValue::Ellipsis => return wrong_type("'ellipsis' object is not iterable"),
        // A deck is a mapping: iterating it gives its outcomes.
        PValue::Deck(d) => d.outcomes().cloned().map(PValue::Value).collect(),
        PValue::Pool(_) | PValue::Deal(_) | PValue::MultiDeal(_) | PValue::Expr(_) => {
            return Err(PErr::Unimplemented("iterating a multiset generator".into()))
        }
        PValue::MultisetFunction(_) | PValue::RawResult(_) => {
            return Err(PErr::Unimplemented(
                "iterating a multiset function result".into(),
            ))
        }
    })
}

/// `expand_arg`'s view of one outcome: a die is mixed in, a hashable value is an outcome, `Reroll` drops out, a
/// tuple holding dice is `tupleize`d, a list is unhashable.
pub(crate) fn die_arg(x: PValue) -> Result<DieArg<Value>, PErr> {
    match x {
        PValue::Value(v) => Ok(DieArg::Outcome(v)),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(DieArg::Die(d)),
        PValue::DieWithTruthError(e, _) => Err(e.into()),
        // Any population is a mapping to `expand_arg`.
        PValue::Deck(d) => Ok(DieArg::Die(d.population().clone())),
        // `arg in icepool.REROLL_TYPES`.
        PValue::Reroll | PValue::Restart => Ok(DieArg::Reroll),
        // `map` unwraps a `Break`; nothing else returns one.
        PValue::Break(_) => Err(PErr::Unimplemented("a Break as an outcome".into())),
        PValue::Tuple(items) => map::tupleize(items),
        PValue::List(_) => unhashable_list(),
        PValue::Func(_) => Err(PErr::Unimplemented("a function as an outcome".into())),
        // `Die(...)` replaces top-level expressions first (`die_with_agains`); one nested in a tuple is not handled.
        PValue::Again(_) => Err(PErr::Unimplemented(
            "an Again expression nested in an outcome".into(),
        )),
        PValue::Ellipsis
        | PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_)
        | PValue::MultisetFunction(_)
        | PValue::RawResult(_) => Err(PErr::Unimplemented(
            "Ellipsis, a pool or a multiset function as an outcome".into(),
        )),
    }
}

/// `icepool.Die(outcomes)` (nodes `die`, `die_typed`) and `icepool.Die(outcomes, times)` (node `die_times`).
fn die(outcomes: PValue, times: Option<PValue>) -> Result<PValue, PErr> {
    die_with_agains(outcomes, times, None, None, PValue::Value(Value::None))
}

/// `icepool.Die(outcomes, times, again_count=, again_depth=, again_end=)`: outcomes that are `Again` expressions
/// are rolled again (population/again.py; `None` arguments are `None` here); without any, the again arguments are
/// ignored.
pub(crate) fn die_with_agains(
    outcomes: PValue,
    times: Option<PValue>,
    again_count: Option<PValue>,
    again_depth: Option<PValue>,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let (objects, times) = itemize(outcomes, times)?;
    if objects.iter().any(|x| matches!(x, PValue::Again(_))) {
        if let Some(count) = again_count {
            if again_depth.is_some() {
                return raise(Error::InvalidArgument(
                    "At most one of again_count and again_depth may be used.".into(),
                ));
            }
            return again::evaluate_agains_using_count(objects, times, count, again_end);
        }
        // The default is depth mode with depth 1.
        let depth = again_depth.unwrap_or(PValue::Value(Value::from(1)));
        return again::evaluate_agains_using_depth(objects, times, depth, again_end);
    }
    new_die(objects, &times)
}

/// The rest of `Die.__new__` once the times are checked to be non-negative numbers and any `Again` is replaced.
pub(crate) fn new_die(objects: Vec<PValue>, times: &[Value]) -> Result<PValue, PErr> {
    // A single die with weight 1 is returned as-is (a `DieWithTruth` keeps its type). Python tests
    // `times[0] == 1`, so `1.0` and `True` count too; they must be caught before `weight` would reject a non-int.
    if let ([d @ (PValue::Die(_) | PValue::DieWithTruth(..))], [t]) = (objects.as_slice(), times) {
        if t.as_num()
            .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ONE)))
        {
            return Ok(d.clone());
        }
    }
    let args: Vec<DieArg<Value>> = objects.into_iter().map(die_arg).collect::<Result<_, _>>()?;
    let quantities = args
        .iter()
        .zip(times)
        .map(|(a, t)| weight(a, t))
        .collect::<Result<_, _>>()?;
    Ok(PValue::Die(Die::new(args, quantities)?))
}

/// `creation_args.itemize(keys, times)`: a sequence of keys and one multiplier per key (a Python number, checked
/// non-negative, but not yet checked to be a valid quantity: see `weight`).
pub(crate) fn itemize(
    keys: PValue,
    times: Option<PValue>,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    // A die is a Mapping: its outcomes become the keys and its quantities multiply the times. Each outcome then has
    // denominator 1, so zero quantities drop out. So is a deck.
    let keys = match keys {
        PValue::Deck(d) => PValue::Die(d.population().clone()),
        keys => keys,
    };
    let (args, base): (Vec<PValue>, Option<Vec<Value>>) = match keys {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => {
            let (a, q) = d
                .items()
                .iter()
                .map(|(o, q)| (PValue::Value(o.clone()), Value::Int(q.clone().into())))
                .unzip();
            (a, Some(q))
        }
        // Any other sequence: each element once. `len()` of an int, float, None... raises TypeError.
        x @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Value(Value::Str(_) | Value::Tuple(_))) => (py_iter(x)?, None),
        PValue::Func(_) => return Err(PErr::Unimplemented("Die(mapping)".into())),
        _ => return wrong_type("Argument appears not to be a mapping or sequence."),
    };
    itemize_times(args, base, times)
}

/// `itemize(mapping, times)` for a mapping given as its keys and values (the values multiply the times).
pub(crate) fn itemize_mapping(
    keys: Vec<PValue>,
    values: Vec<Value>,
    times: PValue,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    itemize_times(keys, Some(values), Some(times))
}

/// The rest of `itemize` once the keys are known: `base` holds a mapping's values.
fn itemize_times(
    args: Vec<PValue>,
    base: Option<Vec<Value>>,
    times: Option<PValue>,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    let times: Vec<Value> = match times {
        None => vec![Value::from(1); args.len()],
        // `isinstance(times, int)`: one factor for every key.
        Some(PValue::Value(t @ (Value::Int(_) | Value::Bool(_)))) => vec![t; args.len()],
        // Otherwise any sequence of the same length (even a str or a die, whose outcomes are then the times).
        Some(t) => {
            let ts = py_iter(t)?;
            if ts.len() != args.len() {
                return raise(Error::InvalidArgument(format!(
                    "The number of times ({}) must equal the number of keys ({}).",
                    ts.len(),
                    args.len()
                )));
            }
            ts.into_iter()
                .map(|t| match t {
                    PValue::Value(v) => Ok(v),
                    _ => wrong_type("'<' not supported between a quantity and 'int'"),
                })
                .collect::<Result<_, _>>()?
        }
    };
    let times = match base {
        None => times,
        Some(q) => q
            .iter()
            .zip(&times)
            .map(|(v, t)| match v {
                Value::Int(q) if *q >= IBig::ZERO => {
                    int_times(&dashu_base::UnsignedAbs::unsigned_abs(q.clone()), t)
                }
                // `v * x` for a mapping value that is not a quantity.
                v => v.binary(BinOp::Mul, t).map_err(PErr::Raise),
            })
            .collect::<Result<_, _>>()?,
    };
    // `any(x < 0 for x in times)`: raises for negatives, and for anything that is not a number.
    for t in &times {
        match t.as_num() {
            Some(n) if n < zero() => return negative_times(),
            Some(_) => {}
            None => {
                return raise(Error::Incomparable {
                    op: "<",
                    left: t.type_name(),
                    right: "int",
                })
            }
        }
    }
    Ok((args, times))
}

/// Python's `q * t` for an int quantity `q`. Only numbers can make a valid quantity; anything else raises later
/// in Python too (at the latest at `x < 0`), so it raises here.
fn int_times(q: &Quantity, t: &Value) -> Result<Value, PErr> {
    let qi = IBig::from(q.clone());
    Ok(match t {
        Value::Int(i) => Value::Int(qi * i),
        Value::Bool(b) => Value::Int(qi * IBig::from(*b as u8)),
        Value::Fraction(r) => Value::Fraction(RBig::from(qi) * r),
        Value::Float(f) => {
            // float(q) rounds correctly and overflows rather than giving inf.
            let qf: f64 = q.to_string().parse().expect("an integer parses as f64");
            if qf.is_infinite() {
                return raise(Error::Overflow("int too large to convert to float".into()));
            }
            Value::Float(qf * f)
        }
        _ => {
            return wrong_type(format!(
                "can't multiply an int quantity by '{}'",
                t.type_name()
            ))
        }
    })
}

/// One non-negative number given as a quantity to `Die`, following what Python does with it in
/// `merge_weights_lcm`: an `int` (or `bool`) is the quantity; another number (`float`, `Fraction`) only survives if
/// it can never reach `math.gcd`: when it is zero, or when the argument is an empty die (denominator 0). Either way
/// it then contributes nothing, exactly like a 0.
fn weight(arg: &DieArg<Value>, t: &Value) -> Result<Quantity, PErr> {
    match t {
        Value::Int(i) => Ok(Quantity::try_from(i.clone()).expect("checked non-negative")),
        Value::Bool(b) => Ok(Quantity::from(*b as u8)),
        _ => {
            let empty_die = matches!(arg, DieArg::Die(d) if d.denominator().is_zero());
            if t.as_num().is_some_and(|n| n == zero()) || empty_die {
                Ok(Quantity::ZERO)
            } else {
                wrong_type(format!(
                    "'{}' object cannot be interpreted as an integer",
                    t.type_name()
                ))
            }
        }
    }
}

/// A Python dict key: a value or a die (dice hash by their items).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Value(Value),
    Die(Die<Value>),
}

/// `{k: v for k, v in items}`: each item unpacks into exactly two; a repeated key (by ==, e.g. True and 1) keeps
/// its first spelling and its last value. Returned in insertion order.
pub(crate) fn dict_items(items: PValue) -> Result<Vec<(Key, PValue)>, PErr> {
    let mut entries: Vec<(Key, PValue)> = Vec::new();
    let mut index: FxHashMap<Key, usize> = FxHashMap::default();
    for item in py_iter(items)? {
        let Ok([k, v]) = <[PValue; 2]>::try_from(py_iter(item)?) else {
            return raise(Error::InvalidArgument(
                "not enough or too many values to unpack (expected 2)".into(),
            ));
        };
        let k = match k {
            PValue::Value(v) => Key::Value(v),
            PValue::Die(d) => Key::Die(d),
            PValue::List(_) => return unhashable_list(),
            _ => return Err(PErr::Unimplemented("this dict key".into())),
        };
        match index.get(&k) {
            Some(&i) => entries[i].1 = v,
            None => {
                index.insert(k.clone(), entries.len());
                entries.push((k, v));
            }
        }
    }
    Ok(entries)
}

/// `icepool.Die({outcome: quantity for outcome, quantity in items})`.
fn weights(items: PValue) -> Result<PValue, PErr> {
    // An `Again` key hashes by its `hash_key`: those dicts take the dict that compares keys one by one.
    let again_key = |item: &PValue| match item {
        PValue::List(kv) | PValue::Tuple(kv) => matches!(kv.first(), Some(PValue::Again(_))),
        _ => false,
    };
    if let PValue::List(xs) | PValue::Tuple(xs) = &items {
        if xs.iter().any(again_key) {
            let (args, values) = crate::pool::dict(items)?.into_iter().unzip();
            return weights_die(args, values);
        }
    }
    let (keys, values): (Vec<Key>, Vec<PValue>) = dict_items(items)?.into_iter().unzip();
    let args: Vec<PValue> = keys
        .into_iter()
        .map(|k| match k {
            Key::Value(v) => PValue::Value(v),
            Key::Die(d) => PValue::Die(d),
        })
        .collect();
    weights_die(args, values)
}

/// `icepool.Die(mapping)` for a dict's distinct keys and its values.
pub(crate) fn weights_die(args: Vec<PValue>, values: Vec<PValue>) -> Result<PValue, PErr> {
    // Die(mapping): itemize multiplies each quantity by 1, which leaves any number as it is, then checks `x < 0`.
    let mut times = Vec::with_capacity(values.len());
    for v in values {
        match v {
            PValue::Value(t) if t.as_num().is_some_and(|n| n < zero()) => return negative_times(),
            PValue::Value(t) if t.as_num().is_some() => times.push(t),
            _ => return wrong_type("'<' not supported between a quantity and 'int'"),
        }
    }
    if args.iter().any(|x| matches!(x, PValue::Again(_))) {
        // `Die.__new__` after `itemize`: depth mode, depth 1, the default end.
        return again::evaluate_agains_using_depth(
            args,
            times,
            PValue::Value(Value::from(1)),
            PValue::Value(Value::None),
        );
    }
    new_die(args, &times)
}
