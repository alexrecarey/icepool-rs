//! The grammar nodes (harness/grammar/*.py), each unpacking its Python-style arguments and calling the core.

use std::borrow::Cow;

use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use super::{raise, Args, PErr, PValue};
use crate::creation_args::DieArg;
use crate::error::Error;
use crate::value::{BinOp, Num, UnOp};
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
        "round" => {
            let [a, ndigits] = args.exactly(["a", "ndigits"])?;
            round(a, ndigits)
        }
        _ => {
            if let Some(bin) = binary_op(op) {
                let [a, b] = args.exactly(["a", "b"])?;
                binary(bin, a, b)
            } else if let Some(un) = unary_op(op) {
                let [a] = args.exactly(["a"])?;
                unary(un, a)
            } else {
                Err(PErr::Unimplemented(op.into()))
            }
        }
    }
}

/// The binary operator nodes (harness/grammar/basic.py, operators.py), named after Python's `operator` module.
fn binary_op(op: &str) -> Option<BinOp> {
    use BinOp::*;
    Some(match op {
        "add" => Add,
        "sub" => Sub,
        "mul" => Mul,
        "truediv" => TrueDiv,
        "floordiv" => FloorDiv,
        "mod" => Mod,
        "pow" => Pow,
        "lshift" => LShift,
        "rshift" => RShift,
        "and" => And,
        "or" => Or,
        "xor" => Xor,
        "lt" => Lt,
        "le" => Le,
        "gt" => Gt,
        "ge" => Ge,
        _ => return None,
    })
}

fn unary_op(op: &str) -> Option<UnOp> {
    use UnOp::*;
    Some(match op {
        "neg" => Neg,
        "pos" => Pos,
        "invert" => Invert,
        "abs" => Abs,
        "trunc" => Trunc,
        "floor" => Floor,
        "ceil" => Ceil,
        _ => return None,
    })
}

/// `implicit_convert_to_die`: a die stays as it is, anything else becomes `Die([x])` (which raises for `None`, and
/// for a list, which is unhashable).
fn to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Die(d) => Ok(d),
        PValue::Value(v) => Ok(Die::new(vec![DieArg::Outcome(v)], vec![Quantity::ONE])?),
        PValue::List(_) => type_error("unhashable type: 'list'"),
    }
}

/// `a <op> b` in Python. With a die on the left, `Die.__op__` converts the right operand. With a die only on the
/// right, the left operand's own `__op__` returns `NotImplemented` (no outcome type knows about dice), so Python
/// calls the die's reflected method: `__radd__` and friends convert the left operand and keep the order
/// (`3 - die` is `Die([3]) - die`), while a comparison reflects (`3 < die` is `die > 3`).
fn binary(op: BinOp, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let apply = |x: &Die<Value>, op: BinOp, y: &Die<Value>| {
        x.binary_operator(y, |p, q| p.binary(op, q))
            .map(PValue::Die)
            .map_err(PErr::from)
    };
    match (a, b) {
        (PValue::Die(a), b) => apply(&a, op, &to_die(b)?),
        // The exception: `str % die` is printf-style formatting with the die as the argument, and a die is a
        // Mapping, which formatting accepts even when the format string never uses it.
        (PValue::Value(Value::Str(s)), PValue::Die(_)) if op == BinOp::Mod => {
            Ok(PValue::Value(crate::value::ops::str_format_mapping(&s)?))
        }
        (a, PValue::Die(b)) => match op.reflected_comparison() {
            Some(reflected) => apply(&b, reflected, &to_die(a)?),
            None => apply(&to_die(a)?, op, &b),
        },
        (PValue::Value(a), PValue::Value(b)) => Ok(PValue::Value(a.binary(op, &b)?)),
        _ => Err(PErr::Unimplemented("operators on lists".into())),
    }
}

/// `-a`, `+a`, `~a`, `abs(a)`, `math.trunc(a)`, `math.floor(a)`, `math.ceil(a)`: `Die.unary_operator` on a die.
fn unary(op: UnOp, a: PValue) -> Result<PValue, PErr> {
    match a {
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.unary(op))?)),
        PValue::Value(v) => Ok(PValue::Value(v.unary(op)?)),
        PValue::List(_) => type_error("bad operand type for a unary operator: 'list'"),
    }
}

/// `round(a, ndigits)`. For a die, `Die.round` rounds each outcome, so a bad `ndigits` only raises if there is an
/// outcome to round.
fn round(a: PValue, ndigits: PValue) -> Result<PValue, PErr> {
    let PValue::Value(nd) = ndigits else {
        return type_error("ndigits must be an int or None");
    };
    match a {
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.round(&nd))?)),
        PValue::Value(v) => Ok(PValue::Value(v.round(&nd)?)),
        PValue::List(_) => type_error("type list doesn't define __round__ method"),
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
