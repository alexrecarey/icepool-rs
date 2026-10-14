//! The named functions of harness/catalogue/functions.py (Rust side), for `map`, `reroll`, `filter`, `split`.
//!
//! Each entry is a Python closure there (`add_k(3)` is `lambda x: x + 3`); here it is a variant holding the same
//! parameters, evaluated at call time exactly as the closure would (so `d(n)` with a bad `n` only raises when that
//! branch runs). The bodies use the program layer's Python operators, so every outcome type behaves as in Python.

use rustc_hash::FxHashMap;

use super::ops::{binary, d, dict_items, type_error, Key};
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::value::BinOp;
use crate::Value;

#[derive(Debug)]
pub enum Func {
    /// `lambda x: x + k`
    AddK(PValue),
    /// `lambda x: x * k`
    MulK(PValue),
    /// `lambda x: min(max(x, lo), hi)`
    Clamp(PValue, PValue),
    /// khepri's `read_roll(roll, success_value)` (harness/core/khepri/f2f.py).
    ReadRoll,
    /// `lambda x: x % 2 == 0`
    IsEven,
    /// `lambda x: x > k`
    GtK(PValue),
    /// `lambda x: x`
    Identity,
    /// `lambda a, b: (b, a)`
    Swap,
    /// `lambda a, b: a + b`
    Add2,
    /// `lambda a, b, c: a + b + c`
    Sum3,
    /// `lambda a, b: max(a, b)`
    Max2,
    /// `lambda t: t[0]`
    First,
    /// `lambda x: (x, k)`
    PairK(PValue),
    /// `lambda x: icepool.d(n) if x == k else x`
    DieIfK(PValue, PValue),
    /// `lambda x: x + icepool.d(n)`
    PlusD(PValue),
    /// `lambda x: icepool.Reroll if x < k else x`
    RerollBelow(PValue),
    /// `lambda x: (x, icepool.Reroll) if x == k else (x, x)`
    RerollInTuple(PValue),
    /// `lambda x: (x, icepool.d(n))`
    TupleD(PValue),
    /// `lambda *args: v`
    Const(PValue),
    /// `{k: v for k, v in pairs}`: a mapping, not a callable.
    Mapping(FxHashMap<Key, PValue>),
}

impl Func {
    /// `FUNCTIONS[name](*params)`.
    pub fn new(name: PValue, params: Vec<PValue>) -> Result<Func, PErr> {
        use Func::*;
        let PValue::Value(Value::Str(name)) = name else {
            return raise(Error::Key("catalogue names are strs".into()));
        };
        // The factory's own signature (`add_k(k)`, `clamp(lo, hi)`, ...).
        let want = match &*name {
            "read_roll" | "is_even" | "identity" | "swap" | "add2" | "sum3" | "max2" | "first" => 0,
            "add_k" | "mul_k" | "gt_k" | "pair_k" | "plus_d" | "reroll_below"
            | "reroll_in_tuple" | "tuple_d" | "const" | "mapping" => 1,
            "clamp" | "die_if_k" => 2,
            _ => return raise(Error::Key(format!("no catalogue function {name:?}"))),
        };
        if params.len() != want {
            return type_error(format!(
                "{name}() takes {want} parameters, {} given",
                params.len()
            ));
        }
        let mut p = params.into_iter();
        let mut next = || p.next().expect("parameter count checked");
        Ok(match &*name {
            "add_k" => AddK(next()),
            "mul_k" => MulK(next()),
            "clamp" => Clamp(next(), next()),
            "read_roll" => ReadRoll,
            "is_even" => IsEven,
            "gt_k" => GtK(next()),
            "identity" => Identity,
            "swap" => Swap,
            "add2" => Add2,
            "sum3" => Sum3,
            "max2" => Max2,
            "first" => First,
            "pair_k" => PairK(next()),
            "die_if_k" => DieIfK(next(), next()),
            "plus_d" => PlusD(next()),
            "reroll_below" => RerollBelow(next()),
            "reroll_in_tuple" => RerollInTuple(next()),
            "tuple_d" => TupleD(next()),
            "const" => Const(next()),
            "mapping" => Mapping(dict_items(next())?.into_iter().collect()),
            _ => unreachable!("names checked above"),
        })
    }

    /// `count_positional_parameters`: (required, total), total `None` for `*args`. `None` for a mapping, which is
    /// not callable.
    pub fn positional(&self) -> Option<(usize, Option<usize>)> {
        use Func::*;
        let n = match self {
            Mapping(_) => return None,
            Const(_) => return Some((0, None)),
            ReadRoll | Swap | Add2 | Max2 => 2,
            Sum3 => 3,
            _ => 1,
        };
        Some((n, Some(n)))
    }

    /// `infer_star`: whether the first argument should be unpacked, given `arg_count` arguments.
    pub fn infer_star(&self, arg_count: usize) -> bool {
        self.positional()
            .is_some_and(|(required, _)| required > arg_count)
    }

    /// Calls the function. A mapping is `mapping.get(o, o)` (`transition_and_star`).
    pub fn call(&self, args: Vec<PValue>) -> Result<PValue, PErr> {
        use Func::*;
        if let Mapping(m) = self {
            let [o] = <[PValue; 1]>::try_from(args).expect("a mapping takes one argument");
            let key = match &o {
                PValue::Value(v) => Key::Value(v.clone()),
                PValue::Die(d) => Key::Die(d.clone()),
                _ => return Err(PErr::Unimplemented("this mapping key".into())),
            };
            return Ok(m.get(&key).cloned().unwrap_or(o));
        }
        let (required, total) = self.positional().expect("not a mapping");
        if args.len() < required || total.is_some_and(|t| args.len() > t) {
            return type_error(format!(
                "takes {required} positional arguments but {} were given",
                args.len()
            ));
        }
        let mut a = args.into_iter();
        let mut x = || a.next().expect("argument count checked");
        let int = |i: i64| PValue::Value(Value::from(i));
        Ok(match self {
            AddK(k) => binary(BinOp::Add, x(), k.clone())?,
            MulK(k) => binary(BinOp::Mul, x(), k.clone())?,
            Clamp(lo, hi) => {
                let m = py_max(x(), lo.clone())?;
                py_min(m, hi.clone())?
            }
            ReadRoll => read_roll(x(), x())?,
            IsEven => {
                let r = binary(BinOp::Mod, x(), int(2))?;
                PValue::Value(Value::Bool(py_eq(&r, &int(0))?))
            }
            GtK(k) => binary(BinOp::Gt, x(), k.clone())?,
            Identity => x(),
            Swap => {
                let (a, b) = (x(), x());
                PValue::tuple(vec![b, a])
            }
            Add2 => binary(BinOp::Add, x(), x())?,
            Sum3 => {
                let ab = binary(BinOp::Add, x(), x())?;
                binary(BinOp::Add, ab, x())?
            }
            Max2 => py_max(x(), x())?,
            First => first(x())?,
            PairK(k) => PValue::tuple(vec![x(), k.clone()]),
            DieIfK(k, n) => {
                let x = x();
                if py_eq(&x, k)? {
                    d(n.clone())?
                } else {
                    x
                }
            }
            PlusD(n) => binary(BinOp::Add, x(), d(n.clone())?)?,
            RerollBelow(k) => {
                let x = x();
                if truthy(&binary(BinOp::Lt, x.clone(), k.clone())?)? {
                    PValue::Reroll
                } else {
                    x
                }
            }
            RerollInTuple(k) => {
                let x = x();
                if py_eq(&x, k)? {
                    PValue::tuple(vec![x, PValue::Reroll])
                } else {
                    PValue::tuple(vec![x.clone(), x])
                }
            }
            TupleD(n) => PValue::tuple(vec![x(), d(n.clone())?]),
            Const(v) => v.clone(),
            Mapping(_) => unreachable!("handled above"),
        })
    }
}

/// khepri's `read_roll`: one d20 read against a success value: 0 = miss, 1-19 = success (higher beats lower),
/// 20 = Critical. Over 20, the excess is added to the roll: those that reach 20 or more become Criticals.
fn read_roll(mut roll: PValue, mut success_value: PValue) -> Result<PValue, PErr> {
    let twenty = || PValue::Value(Value::from(20));
    if truthy(&binary(BinOp::Gt, success_value.clone(), twenty())?)? {
        let excess = binary(BinOp::Sub, success_value, twenty())?;
        roll = binary(BinOp::Add, roll, excess)?;
        success_value = twenty();
    }
    if py_eq(&roll, &success_value)? || truthy(&binary(BinOp::Gt, roll.clone(), twenty())?)? {
        return Ok(twenty());
    }
    Ok(
        if truthy(&binary(BinOp::Lt, roll.clone(), success_value)?)? {
            roll
        } else {
            PValue::Value(Value::from(0))
        },
    )
}

/// Builtin `max(a, b)`: `b` replaces `a` only if `b > a`.
fn py_max(a: PValue, b: PValue) -> Result<PValue, PErr> {
    Ok(if truthy(&binary(BinOp::Gt, b.clone(), a.clone())?)? {
        b
    } else {
        a
    })
}

/// Builtin `min(a, b)`: `b` replaces `a` only if `b < a`.
fn py_min(a: PValue, b: PValue) -> Result<PValue, PErr> {
    Ok(if truthy(&binary(BinOp::Lt, b.clone(), a.clone())?)? {
        b
    } else {
        a
    })
}

/// `t[0]`.
fn first(t: PValue) -> Result<PValue, PErr> {
    let index_error = || raise(Error::Index("index out of range".into()));
    match t {
        PValue::Value(Value::Tuple(items)) => match items.first() {
            Some(v) => Ok(PValue::Value(v.clone())),
            None => index_error(),
        },
        PValue::Value(Value::Str(s)) => match s.chars().next() {
            Some(c) => Ok(PValue::Value(Value::str(c.encode_utf8(&mut [0; 4])))),
            None => index_error(),
        },
        PValue::Value(v) => type_error(format!("'{}' object is not subscriptable", v.type_name())),
        _ => Err(PErr::Unimplemented("indexing this object".into())),
    }
}

/// Python's `bool(x)`. A die has no truth value (unless it came from `==` / `!=`, which the port does not model
/// yet), so `if die:` raises.
pub fn truthy(x: &PValue) -> Result<bool, PErr> {
    Ok(match x {
        PValue::Value(v) => match v {
            Value::None => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != dashu_int::IBig::ZERO,
            Value::Float(f) => *f != 0.0,
            Value::Fraction(r) => *r != dashu_ratio::RBig::ZERO,
            Value::Str(s) => !s.is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Complex(re, im) => *re != 0.0 || *im != 0.0,
        },
        PValue::Die(_) => {
            return type_error("A `Die` only has a truth value if it is the result of == or !=")
        }
        PValue::List(xs) | PValue::Tuple(xs) => !xs.is_empty(),
        PValue::Reroll | PValue::Func(_) => true,
    })
}

/// Python's `a == b` used as a condition.
pub fn py_eq(a: &PValue, b: &PValue) -> Result<bool, PErr> {
    match (a, b) {
        (PValue::Value(a), PValue::Value(b)) => Ok(a == b),
        (PValue::Reroll, PValue::Reroll) => Ok(true),
        (PValue::Reroll, PValue::Value(_)) | (PValue::Value(_), PValue::Reroll) => Ok(false),
        _ => Err(PErr::Unimplemented(
            "== with dice (DieWithTruth), lists, functions".into(),
        )),
    }
}
