//! The grammar nodes (harness/grammar/*.py), each unpacking its Python-style arguments and calling the core.

use std::borrow::Cow;

use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use super::{raise, Args, PErr, PValue};
use crate::creation_args::DieArg;
use crate::error::Error;
use crate::value::Num;
use crate::{Die, Quantity, Value};

pub fn call(op: &str, args: Args) -> Result<PValue, PErr> {
    match op {
        "d" => {
            let [sides] = args.exactly(["sides"])?;
            d(sides)
        }
        "die" | "die_typed" => {
            let [outcomes] = args.exactly(["outcomes"])?;
            die(outcomes, None)
        }
        "die_times" => {
            let [outcomes, times] = args.exactly(["outcomes", "times"])?;
            die(outcomes, Some(times))
        }
        "weights" => {
            let [items] = args.exactly(["items"])?;
            weights(items)
        }
        _ => Err(PErr::Unimplemented(op.into())),
    }
}

fn type_error<T>(msg: impl Into<String>) -> Result<T, PErr> {
    raise(Error::Type(msg.into()))
}

fn negative_times<T>() -> Result<T, PErr> {
    raise(Error::Value("times cannot have negative values.".into()))
}

fn zero() -> Num<'static> {
    Num::Int(Cow::Owned(IBig::ZERO))
}

/// `icepool.d(sides)`: `sides` must be an `int` (a `bool` is one).
fn d(sides: PValue) -> Result<PValue, PErr> {
    let sides = match sides {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        _ => return type_error("sides must be an int."),
    };
    // Beyond i64 Python would run out of memory long before finishing.
    let sides =
        i64::try_from(sides).map_err(|_| PErr::Raise(Error::Overflow("sides too large".into())))?;
    Ok(PValue::Die(crate::d(sides)?))
}

/// Python's `iter(x)` over a program value: a list, a tuple's elements, a str's characters, a die's outcomes.
fn py_iter(x: PValue) -> Result<Vec<PValue>, PErr> {
    Ok(match x {
        PValue::List(xs) => xs,
        PValue::Value(Value::Tuple(t)) => t.iter().cloned().map(PValue::Value).collect(),
        PValue::Value(Value::Str(s)) => s
            .chars()
            .map(|c| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Die(d) => d.outcomes().cloned().map(PValue::Value).collect(),
        PValue::Value(v) => {
            return type_error(format!("'{}' object is not iterable", v.type_name()))
        }
    })
}

/// `expand_arg`'s view of one outcome: a die is mixed in, a hashable value is an outcome, a list is unhashable.
fn die_arg(x: PValue) -> Result<DieArg<Value>, PErr> {
    match x {
        PValue::Value(v) => Ok(DieArg::Outcome(v)),
        PValue::Die(d) => Ok(DieArg::Die(d)),
        PValue::List(_) => type_error("unhashable type: 'list'"),
    }
}

/// `icepool.Die(outcomes)` (nodes `die`, `die_typed`) and `icepool.Die(outcomes, times)` (node `die_times`).
fn die(outcomes: PValue, times: Option<PValue>) -> Result<PValue, PErr> {
    let (args, times) = itemize(outcomes, times)?;
    new_die(args, &times)
}

/// The rest of `Die.__new__` once the times are checked to be non-negative numbers.
fn new_die(args: Vec<DieArg<Value>>, times: &[Value]) -> Result<PValue, PErr> {
    // A single die with weight 1 is returned as-is. Python tests `times[0] == 1`, so `1.0` and `True` count too;
    // they must be caught before `weight` would reject a non-int.
    if let ([DieArg::Die(d)], [t]) = (args.as_slice(), times) {
        if t.as_num()
            .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ONE)))
        {
            return Ok(PValue::Die(d.clone()));
        }
    }
    let quantities = args
        .iter()
        .zip(times)
        .map(|(a, t)| weight(a, t))
        .collect::<Result<_, _>>()?;
    Ok(PValue::Die(Die::new(args, quantities)?))
}

/// `creation_args.itemize(keys, times)`: a sequence of keys and one multiplier per key (a Python number, checked
/// non-negative, but not yet checked to be a valid quantity: see `weight`).
fn itemize(keys: PValue, times: Option<PValue>) -> Result<(Vec<DieArg<Value>>, Vec<Value>), PErr> {
    // A die is a Mapping: its outcomes become the keys and its quantities multiply the times. Each outcome then has
    // denominator 1, so zero quantities drop out.
    let (args, base): (Vec<DieArg<Value>>, Option<Vec<Quantity>>) = match keys {
        PValue::Die(d) => {
            let (a, q) = d
                .items()
                .iter()
                .map(|(o, q)| (DieArg::Outcome(o.clone()), q.clone()))
                .unzip();
            (a, Some(q))
        }
        // Any other sequence: each element once. `len()` of an int, float, None... raises TypeError.
        x @ (PValue::List(_) | PValue::Value(Value::Str(_) | Value::Tuple(_))) => (
            py_iter(x)?
                .into_iter()
                .map(die_arg)
                .collect::<Result<_, _>>()?,
            None,
        ),
        PValue::Value(_) => return type_error("Argument appears not to be a mapping or sequence."),
    };
    let times: Vec<Value> = match times {
        None => vec![Value::from(1); args.len()],
        // `isinstance(times, int)`: one factor for every key.
        Some(PValue::Value(t @ (Value::Int(_) | Value::Bool(_)))) => vec![t; args.len()],
        // Otherwise any sequence of the same length (even a str or a die, whose outcomes are then the times).
        Some(t) => {
            let ts = py_iter(t)?;
            if ts.len() != args.len() {
                return raise(Error::Value(format!(
                    "The number of times ({}) must equal the number of keys ({}).",
                    ts.len(),
                    args.len()
                )));
            }
            ts.into_iter()
                .map(|t| match t {
                    PValue::Value(v) => Ok(v),
                    _ => type_error("'<' not supported between a quantity and 'int'"),
                })
                .collect::<Result<_, _>>()?
        }
    };
    let times = match base {
        None => times,
        Some(q) => q
            .iter()
            .zip(&times)
            .map(|(q, t)| int_times(q, t))
            .collect::<Result<_, _>>()?,
    };
    // `any(x < 0 for x in times)`: raises for negatives, and for anything that is not a number.
    for t in &times {
        match t.as_num() {
            Some(n) if n < zero() => return negative_times(),
            Some(_) => {}
            None => {
                return type_error(format!(
                    "'<' not supported between instances of '{}' and 'int'",
                    t.type_name()
                ))
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
            return type_error(format!(
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
                type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    t.type_name()
                ))
            }
        }
    }
}

/// A Python dict key: a value or a die (dice hash by their items).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Value(Value),
    Die(Die<Value>),
}

/// `icepool.Die({outcome: quantity for outcome, quantity in items})`.
fn weights(items: PValue) -> Result<PValue, PErr> {
    // The dict comprehension: each item unpacks into exactly two; a repeated key (by ==, e.g. True and 1) keeps its
    // first spelling and its last quantity.
    let mut keys: Vec<Key> = Vec::new();
    let mut values: Vec<PValue> = Vec::new();
    let mut index: FxHashMap<Key, usize> = FxHashMap::default();
    for item in py_iter(items)? {
        let Ok([k, v]) = <[PValue; 2]>::try_from(py_iter(item)?) else {
            return raise(Error::Value(
                "not enough or too many values to unpack (expected 2)".into(),
            ));
        };
        let k = match k {
            PValue::Value(v) => Key::Value(v),
            PValue::Die(d) => Key::Die(d),
            PValue::List(_) => return type_error("unhashable type: 'list'"),
        };
        match index.get(&k) {
            Some(&i) => values[i] = v,
            None => {
                index.insert(k.clone(), keys.len());
                keys.push(k);
                values.push(v);
            }
        }
    }
    let args: Vec<DieArg<Value>> = keys
        .into_iter()
        .map(|k| match k {
            Key::Value(v) => DieArg::Outcome(v),
            Key::Die(d) => DieArg::Die(d),
        })
        .collect();
    // Die(mapping): itemize multiplies each quantity by 1, which leaves any number as it is, then checks `x < 0`.
    let mut times = Vec::with_capacity(values.len());
    for v in values {
        match v {
            PValue::Value(t) if t.as_num().is_some_and(|n| n < zero()) => return negative_times(),
            PValue::Value(t) if t.as_num().is_some() => times.push(t),
            _ => return type_error("'<' not supported between a quantity and 'int'"),
        }
    }
    new_die(args, &times)
}
