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
    /// `lambda x: icepool.Restart if x < k else x`
    RestartBelow(PValue),
    /// `lambda x: icepool.Break(v) if x == k else icepool.Break()`
    BreakK(PValue, PValue),
    /// `lambda a, b: icepool.Break() if a == b else a + b`
    BreakEq,
    /// `lambda x, *, k: x + k`
    AddKw,
    /// `lambda total, roll: min(total + roll, cap)`
    CapSum(PValue),
    /// `lambda total, roll: icepool.Break(v) if total >= k else total + roll`
    BreakAt(PValue, PValue),
    /// `lambda total, roll: icepool.Restart if roll == k else min(total + roll, cap)`
    RestartOn(PValue, PValue),
    /// `lambda x, step: x if x <= lo or x >= hi else x + step`
    Walk(PValue, PValue),
    /// `lambda x: x if x <= lo or x >= hi else x + icepool.Die([-1, 1])`
    WalkDie(PValue, PValue),
    /// `lambda x: 1 if x <= 1 else x // 2 if x % 2 == 0 else 3 * x + 1`
    Collatz,
    /// `lambda a, b, roll: (a, b) if a + b >= k else (b, a + roll)`
    PairWalk(PValue),
    /// `lambda outcome, *counts: counts[0] - sum(counts[1:])`
    CountDiff,
    /// `lambda outcome, *counts: sum(counts) if outcome > k else 0`
    CountIfGtK(PValue),
    /// `{k: v for k, v in pairs}`: a mapping, not a callable.
    Mapping(FxHashMap<Key, PValue>),
    /// A callable of the host language (the JS facade's functions), called back through the host.
    Host(std::sync::Arc<dyn super::host::HostFunction>),
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
            "read_roll" | "is_even" | "identity" | "swap" | "add2" | "sum3" | "max2" | "first"
            | "break_eq" | "add_kw" | "collatz" | "count_diff" => 0,
            "add_k" | "mul_k" | "gt_k" | "pair_k" | "plus_d" | "reroll_below"
            | "reroll_in_tuple" | "tuple_d" | "const" | "mapping" | "restart_below" | "cap_sum"
            | "pair_walk" | "count_if_gt_k" => 1,
            "clamp" | "die_if_k" | "break_k" | "break_at" | "restart_on" | "walk" | "walk_die" => 2,
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
            "restart_below" => RestartBelow(next()),
            "break_k" => BreakK(next(), next()),
            "break_eq" => BreakEq,
            "add_kw" => AddKw,
            "cap_sum" => CapSum(next()),
            "break_at" => BreakAt(next(), next()),
            "restart_on" => RestartOn(next(), next()),
            "walk" => Walk(next(), next()),
            "walk_die" => WalkDie(next(), next()),
            "collatz" => Collatz,
            "pair_walk" => PairWalk(next()),
            "count_diff" => CountDiff,
            "count_if_gt_k" => CountIfGtK(next()),
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
            Host(f) => return Some(f.positional()),
            Const(_) => return Some((0, None)),
            CountDiff | CountIfGtK(_) => return Some((1, None)),
            ReadRoll | Swap | Add2 | Max2 | BreakEq | CapSum(_) | BreakAt(..) | RestartOn(..)
            | Walk(..) => 2,
            Sum3 | PairWalk(_) => 3,
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
        self.call_kw(args, vec![])
    }

    /// Calls the function with keyword arguments too (`map`'s `**kwargs`, passed as-is). Only `add_kw` has a
    /// keyword-only parameter (`k`, required); any other name is unexpected (TypeError), unless it names a
    /// positional parameter, which Python would bind: the port does not model that.
    pub fn call_kw(
        &self,
        args: Vec<PValue>,
        kwargs: Vec<(String, PValue)>,
    ) -> Result<PValue, PErr> {
        use Func::*;
        if let Host(f) = self {
            return f.call(args, kwargs);
        }
        let mut k = None;
        for (name, v) in kwargs {
            match (self, name.as_str()) {
                (AddKw, "k") => k = Some(v),
                (
                    _,
                    "x" | "a" | "b" | "c" | "t" | "o" | "roll" | "success_value" | "total" | "step",
                ) => {
                    return Err(PErr::Unimplemented(
                        "a keyword argument naming a positional parameter".into(),
                    ))
                }
                _ => return type_error(format!("got an unexpected keyword argument '{name}'")),
            }
        }
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
        // The `(outcome, *counts)` functions of map_counts. Their counts are ints, for which `sum` is plain
        // addition from 0.
        let sum = |xs: &[PValue]| -> Result<PValue, PErr> {
            xs.iter().try_fold(PValue::Value(Value::from(0)), |acc, x| {
                binary(BinOp::Add, acc, x.clone())
            })
        };
        match self {
            CountDiff => {
                let Some(first) = args.get(1) else {
                    return raise(Error::Index("tuple index out of range".into()));
                };
                return binary(BinOp::Sub, first.clone(), sum(&args[2..])?);
            }
            CountIfGtK(k) => {
                return if truthy(&binary(BinOp::Gt, args[0].clone(), k.clone())?)? {
                    sum(&args[1..])
                } else {
                    Ok(PValue::Value(Value::from(0)))
                };
            }
            _ => {}
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
            // `x % 2 == 0`: whatever `==` returns (a vector compares elementwise).
            IsEven => super::format::eq(binary(BinOp::Mod, x(), int(2))?, int(0), false)?,
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
            RestartBelow(k) => {
                let x = x();
                if truthy(&binary(BinOp::Lt, x.clone(), k.clone())?)? {
                    PValue::Restart
                } else {
                    x
                }
            }
            BreakK(k, v) => {
                if py_eq(&x(), k)? {
                    PValue::Break(Box::new(v.clone()))
                } else {
                    PValue::Break(Box::new(PValue::Value(Value::None)))
                }
            }
            BreakEq => {
                let (a, b) = (x(), x());
                if py_eq(&a, &b)? {
                    PValue::Break(Box::new(PValue::Value(Value::None)))
                } else {
                    binary(BinOp::Add, a, b)?
                }
            }
            AddKw => match k {
                Some(k) => binary(BinOp::Add, x(), k)?,
                None => return type_error("missing 1 required keyword-only argument: 'k'"),
            },
            CapSum(cap) => {
                let sum = binary(BinOp::Add, x(), x())?;
                py_min(sum, cap.clone())?
            }
            BreakAt(k, v) => {
                let (total, roll) = (x(), x());
                if truthy(&binary(BinOp::Ge, total.clone(), k.clone())?)? {
                    PValue::Break(Box::new(v.clone()))
                } else {
                    binary(BinOp::Add, total, roll)?
                }
            }
            RestartOn(k, cap) => {
                let (total, roll) = (x(), x());
                if py_eq(&roll, k)? {
                    PValue::Restart
                } else {
                    py_min(binary(BinOp::Add, total, roll)?, cap.clone())?
                }
            }
            Walk(lo, hi) => {
                let (x, step) = (x(), x());
                if outside(&x, lo, hi)? {
                    x
                } else {
                    binary(BinOp::Add, x, step)?
                }
            }
            WalkDie(lo, hi) => {
                let x = x();
                if outside(&x, lo, hi)? {
                    x
                } else {
                    let step = crate::Die::new(
                        vec![
                            crate::DieArg::Outcome(Value::from(-1)),
                            crate::DieArg::Outcome(Value::from(1)),
                        ],
                        vec![crate::Quantity::ONE, crate::Quantity::ONE],
                    )?;
                    binary(BinOp::Add, x, PValue::Die(step))?
                }
            }
            Collatz => {
                let x = x();
                if truthy(&binary(BinOp::Le, x.clone(), int(1))?)? {
                    int(1)
                } else if py_eq(&binary(BinOp::Mod, x.clone(), int(2))?, &int(0))? {
                    binary(BinOp::FloorDiv, x, int(2))?
                } else {
                    binary(BinOp::Add, binary(BinOp::Mul, int(3), x)?, int(1))?
                }
            }
            PairWalk(k) => {
                let (a, b, roll) = (x(), x(), x());
                if truthy(&binary(
                    BinOp::Ge,
                    binary(BinOp::Add, a.clone(), b.clone())?,
                    k.clone(),
                )?)? {
                    PValue::tuple(vec![a, b])
                } else {
                    let next = binary(BinOp::Add, a, roll)?;
                    PValue::tuple(vec![b, next])
                }
            }
            Mapping(_) | CountDiff | CountIfGtK(_) | Host(_) => unreachable!("handled above"),
        })
    }
}

/// `x <= lo or x >= hi`.
fn outside(x: &PValue, lo: &PValue, hi: &PValue) -> Result<bool, PErr> {
    Ok(truthy(&binary(BinOp::Le, x.clone(), lo.clone())?)?
        || truthy(&binary(BinOp::Ge, x.clone(), hi.clone())?)?)
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
        PValue::Value(v) => v.truthy()?,
        PValue::DieWithTruth(_, truth) | PValue::DieWithTruthError(_, truth) => *truth,
        PValue::Die(_) => {
            return type_error("A `Die` only has a truth value if it is the result of == or !=")
        }
        PValue::List(xs) | PValue::Tuple(xs) => !xs.is_empty(),
        PValue::Reroll
        | PValue::Restart
        | PValue::Break(_)
        | PValue::Func(_)
        | PValue::Ellipsis
        // A `MultisetFunctionEvaluator` is a plain object; a raw result a 3-tuple.
        | PValue::MultisetFunction(_)
        | PValue::RawResult(_) => true,
        // A deck is a mapping: truthy unless empty.
        PValue::Deck(d) => !d.is_empty(),
        PValue::Pool(_) | PValue::Deal(_) | PValue::MultiDeal(_) | PValue::Expr(_) => {
            return Err(PErr::Unimplemented("the truth value of a pool".into()))
        }
        PValue::Again(_) => {
            return raise(Error::Value(
                "An `Again` only has a truth value if it is the result of == or !=.".into(),
            ))
        }
    })
}

/// Python's `a == b` used as a condition.
pub fn py_eq(a: &PValue, b: &PValue) -> Result<bool, PErr> {
    match (a, b) {
        (PValue::Value(a), PValue::Value(b)) => Ok(a.eq_op(b, false)?.truthy()?),
        (PValue::Reroll, PValue::Reroll) | (PValue::Restart, PValue::Restart) => Ok(true),
        (PValue::Reroll | PValue::Restart, PValue::Value(_) | PValue::Reroll | PValue::Restart)
        | (PValue::Value(_), PValue::Reroll | PValue::Restart) => Ok(false),
        _ => Err(PErr::Unimplemented(
            "== with dice (DieWithTruth), lists, functions".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::host::HostFunction;

    #[derive(Debug)]
    struct TwoArgs;

    impl HostFunction for TwoArgs {
        fn positional(&self) -> (usize, Option<usize>) {
            (1, Some(2))
        }
        fn call(&self, args: Vec<PValue>, _: Vec<(String, PValue)>) -> Result<PValue, PErr> {
            Ok(args
                .into_iter()
                .next()
                .unwrap_or(PValue::Value(Value::None)))
        }
    }

    #[test]
    fn host_functions_report_their_own_parameters() {
        // `map` infers `star` from these, so they must be the host's, not a default.
        let f = Func::Host(std::sync::Arc::new(TwoArgs));
        assert_eq!(f.positional(), Some((1, Some(2))));
    }
}
