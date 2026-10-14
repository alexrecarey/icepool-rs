//! The grammar nodes (harness/grammar/*.py), each unpacking its Python-style arguments and calling the core.

use std::borrow::Cow;

use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use super::again::{self, AgainOp};
use super::catalogue::Func;
use super::map;
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
        "die_times" | "mixture" => {
            let [outcomes, times] = args.exactly(["outcomes", "times"])?;
            die(outcomes, Some(times))
        }
        "weights" => {
            let [items] = args.exactly(["items"])?;
            weights(items)
        }
        "fn" => {
            let [name, params] = args.exactly(["name", "params"])?;
            Ok(PValue::Func(Func::new(name, py_iter(params)?)?.into()))
        }
        "Reroll" => {
            let [] = args.exactly([])?;
            Ok(PValue::Reroll)
        }
        "Again" => {
            let [] = args.exactly([])?;
            Ok(PValue::Again(std::sync::Arc::new(
                crate::again::Again::Again,
            )))
        }
        "again" => {
            let [outcomes, depth, end] = args.exactly(["outcomes", "depth", "end"])?;
            // `again_depth=None` is the default (depth mode, depth 1).
            let depth = match depth {
                PValue::Value(Value::None) => None,
                d => Some(d),
            };
            die_with_agains(outcomes, None, depth, end)
        }
        "explode" => {
            let [die, which, star, depth, end] =
                args.exactly(["die", "which", "star", "depth", "end"])?;
            again::explode(die, which, star, depth, end)
        }
        "matmul" => {
            let [n, die] = args.exactly(["n", "die"])?;
            matmul(n, die)
        }
        "tuple" => {
            let [items] = args.exactly(["items"])?;
            Ok(PValue::tuple(py_iter(items)?))
        }
        "map" => {
            let [f, args, star] = args.exactly(["f", "args", "star"])?;
            map::map(f, args, star)
        }
        "reroll" | "filter" => {
            let [die, which, star, depth] = args.exactly(["die", "which", "star", "depth"])?;
            map::reroll(op == "filter", die, which, star, depth)
        }
        "split" => {
            let [die, which, star] = args.exactly(["die", "which", "star"])?;
            map::split(die, which, star)
        }
        "clip" | "truncate" => {
            let [die, min, max] = args.exactly(["die", "min", "max"])?;
            map::clip_or_truncate(op == "clip", die, min, max)
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
/// for a list, which is unhashable; `Reroll` gives the empty die).
fn to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Die(d) => Ok(d),
        x => Ok(Die::new(vec![die_arg(x)?], vec![Quantity::ONE])?),
    }
}

/// `a <op> b` in Python. With a die on the left, `Die.__op__` converts the right operand. With a die only on the
/// right, the left operand's own `__op__` returns `NotImplemented` (no outcome type knows about dice), so Python
/// calls the die's reflected method: `__radd__` and friends convert the left operand and keep the order
/// (`3 - die` is `Die([3]) - die`), while a comparison reflects (`3 < die` is `die > 3`).
pub(super) fn binary(op: BinOp, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let apply = |x: &Die<Value>, op: BinOp, y: &Die<Value>| {
        x.binary_operator(y, |p, q| p.binary(op, q))
            .map(PValue::Die)
            .map_err(PErr::from)
    };
    match (a, b) {
        // `AgainExpression` defines every operator (and dice return NotImplemented for it), so an expression on
        // either side builds a bigger expression. A comparison with the expression on the right reflects.
        (a @ PValue::Again(_), b) => Ok(again::apply(AgainOp::Binary(op), vec![a, b])),
        // Except that `str % x` is printf-style formatting, which runs before the expression's `__rmod__`. The
        // expression is neither a tuple nor a mapping, so it is the one argument: without a conversion specifier
        // it is left over. (With one, `%s` would format its repr, which holds an address.)
        (PValue::Value(Value::Str(s)), PValue::Again(_)) if op == BinOp::Mod => {
            if s.contains('%') {
                Err(PErr::Unimplemented(
                    "str % Again with conversion specifiers".into(),
                ))
            } else {
                type_error("not all arguments converted during string formatting")
            }
        }
        (a, b @ PValue::Again(_)) => Ok(match op.reflected_comparison() {
            Some(reflected) => again::apply(AgainOp::Binary(reflected), vec![b, a]),
            None => again::apply(AgainOp::Binary(op), vec![a, b]),
        }),
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
        _ => Err(PErr::Unimplemented(
            "operators on lists, Reroll, tuples of dice, functions".into(),
        )),
    }
}

/// `-a`, `+a`, `~a`, `abs(a)`, `math.trunc(a)`, `math.floor(a)`, `math.ceil(a)`: `Die.unary_operator` on a die.
pub(super) fn unary(op: UnOp, a: PValue) -> Result<PValue, PErr> {
    match a {
        // `AgainExpression` has `-`, `+`, `~` and `abs`, but no `__trunc__`, `__floor__`, `__ceil__`.
        a @ PValue::Again(_) if matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Invert | UnOp::Abs) => {
            Ok(again::apply(AgainOp::Unary(op), vec![a]))
        }
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.unary(op))?)),
        PValue::Value(v) => Ok(PValue::Value(v.unary(op)?)),
        _ => type_error("bad operand type for a unary operator"),
    }
}

/// `Die._sum_all(k)` recurses once per roll down to the largest sum already in the die's `_sum_cache`, so a big
/// enough step raises RecursionError (Python's limit is 1000 frames; at top level `Die([n]) @ d1` fails from about
/// n = 985). Recursing this deep always works, whatever the caller's stack...
const SAFE_RECURSION: u64 = 800;
/// ...and this deep never does.
const SURE_RECURSION_ERROR: u64 = 1000;

/// Whether Python's `_sum_all` would hit the recursion limit computing these counts in order, given the cache it
/// fills as it goes. A step between the two limits depends on the caller's stack, which the port cannot know, so
/// it is unimplemented. (The port assumes the right die's `_sum_cache` starts empty, but it lives on the die
/// object, and `icepool.d` caches its dice, so a Python process that summed `d(6)` before has it warm. Only a
/// recursion past the limit can tell the difference.)
fn check_sum_all_recursion(counts: &[(i64, Quantity)]) -> Result<(), PErr> {
    // The largest positive roll count cached so far (1 is a base case, cached for free).
    let mut cached = 1;
    for (count, _) in counts {
        let k = count.unsigned_abs();
        if k > cached {
            let depth = k - cached + u64::from(*count < 0);
            if depth >= SURE_RECURSION_ERROR {
                return raise(Error::Value(
                    "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
                ));
            }
            if depth > SAFE_RECURSION {
                return Err(PErr::Unimplemented(
                    "n @ die close to Python's recursion limit".into(),
                ));
            }
            cached = k;
        }
    }
    Ok(())
}

/// `n @ die`: roll `n`, then roll `die` that many times and sum the outcomes. An int `n` has no `@`, so Python
/// calls `Die.__rmatmul__`, which converts it with `implicit_convert_to_die`: `Die([n]) @ die`. With an `Again`
/// expression on either side (dice return NotImplemented for it) the result is a bigger expression.
pub(super) fn matmul(n: PValue, die: PValue) -> Result<PValue, PErr> {
    let (n, die) = match (n, die) {
        (n @ PValue::Again(_), die) | (n, die @ PValue::Again(_)) => {
            return Ok(again::apply(AgainOp::MatMul, vec![n, die]))
        }
        (PValue::Die(n), die) => (n, to_die(die)?),
        (n, PValue::Die(die)) => (to_die(n)?, die),
        _ => return type_error("unsupported operand type(s) for @"),
    };
    // `max(abs(self.min_outcome()), abs(self.max_outcome()))`, then `_sum_all(die_count)` for each outcome.
    let mut counts = Vec::with_capacity(n.len());
    let mut kinds = Vec::with_capacity(n.len());
    for (outcome, quantity) in n.items() {
        let (count, kind) = die_count(outcome)?;
        counts.push((count, quantity.clone()));
        kinds.push(kind);
    }
    check_sum_all_recursion(&counts)?;
    if kinds.iter().all(|k| *k == CountKind::Int) {
        return Ok(PValue::Die(die.matmul(&counts)?));
    }
    // Some count is a float (with an integral value: others raised above). `_sum_all` treats it as the equal
    // int, but the factor `den**(max_abs_die_count - abs(die_count))` is then a float, and so is every quantity it
    // scales. (A Fraction exponent keeps the factor an int: `int ** Fraction` with a non-negative integral
    // exponent is `int ** int`.) `Die(data)` drops a non-int quantity that is zero and raises for any
    // other, so the result is the int terms alone, unless a non-int term adds anything non-zero to an outcome.
    // `max` returns the first of two equal values: abs(min_outcome) unless abs(max_outcome) is greater.
    let (first, last) = (counts[0].0, counts[counts.len() - 1].0);
    let max_float = if last.unsigned_abs() > first.unsigned_abs() {
        kinds[kinds.len() - 1]
    } else {
        kinds[0]
    } == CountKind::Float;
    let max_abs = first.unsigned_abs().max(last.unsigned_abs());
    let denominator = die.denominator();
    let mut data: Vec<(Value, Quantity, bool)> = Vec::new();
    let mut index: FxHashMap<Value, usize> = FxHashMap::default();
    for ((term, kind), (count, _)) in die
        .matmul_terms(&counts)?
        .into_iter()
        .zip(&kinds)
        .zip(&counts)
    {
        let float_factor = max_float || *kind == CountKind::Float;
        if float_factor {
            // `int ** float` converts the int first, and overflows rather than giving inf.
            let exponent = (max_abs - count.unsigned_abs()) as i32;
            let base = crate::value::int_to_f64(&IBig::from(denominator.clone()))?;
            if !base.powi(exponent).is_finite() {
                return raise(Error::Overflow("int ** float: result too large".into()));
            }
        }
        for (outcome, quantity) in term {
            match index.get(&outcome) {
                Some(&i) => {
                    data[i].1 += quantity;
                    data[i].2 |= float_factor;
                }
                None => {
                    index.insert(outcome.clone(), data.len());
                    data.push((outcome, quantity, float_factor));
                }
            }
        }
    }
    let mut items = Vec::with_capacity(data.len());
    for (outcome, quantity, float) in data {
        if float && !quantity.is_zero() {
            return type_error("'float' object cannot be interpreted as an integer");
        }
        items.push((outcome, quantity));
    }
    Ok(PValue::Die(Die::from_dict(items)?))
}

/// Whether a count in `n @ die` is a float in Python, which makes its factor a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountKind {
    Int,
    Float,
}

/// One outcome of the left die of `@` as a count of rolls.
fn die_count(outcome: &Value) -> Result<(i64, CountKind), PErr> {
    let (int, kind) = match outcome {
        Value::Int(i) => (i.clone(), CountKind::Int),
        Value::Bool(b) => (IBig::from(*b as u8), CountKind::Int),
        // `_sum_all` subtracts 1 until it reaches 0 or 1, negating on the way below 0, so a count that is not
        // integral (or not finite) recurses until Python raises RecursionError.
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 => (
            IBig::from_str_radix(&format!("{f:.0}"), 10).expect("an integral float"),
            CountKind::Float,
        ),
        Value::Fraction(r) if r.denominator().is_one() => (r.numerator().clone(), CountKind::Int),
        Value::Float(_) | Value::Fraction(_) => {
            return raise(Error::Value(
                "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
            ))
        }
        v => return type_error(format!("bad operand type for abs(): '{}'", v.type_name())),
    };
    // Beyond i64, `_sum_all` recurses past Python's limit.
    let count = i64::try_from(int).map_err(|_| {
        PErr::Raise(Error::Value(
            "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
        ))
    })?;
    Ok((count, kind))
}

/// `round(a, ndigits)`. For a die, `Die.round` rounds each outcome, so a bad `ndigits` only raises if there is an
/// outcome to round.
fn round(a: PValue, ndigits: PValue) -> Result<PValue, PErr> {
    // A die or a list as ndigits is not an int, so rounding any outcome with it raises; but `Die.round` on an
    // empty die rounds nothing and returns it. A str stands in: `Value::round` raises for any non-int ndigits.
    let nd = match &ndigits {
        PValue::Value(v) => v.clone(),
        _ => Value::str(""),
    };
    match a {
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.round(&nd))?)),
        PValue::Value(v) => Ok(PValue::Value(v.round(&nd)?)),
        _ => type_error("type doesn't define __round__ method"),
    }
}

pub(super) fn type_error<T>(msg: impl Into<String>) -> Result<T, PErr> {
    raise(Error::Type(msg.into()))
}

fn negative_times<T>() -> Result<T, PErr> {
    raise(Error::Value("times cannot have negative values.".into()))
}

fn zero() -> Num<'static> {
    Num::Int(Cow::Owned(IBig::ZERO))
}

/// `icepool.d(sides)`: `sides` must be an `int` (a `bool` is one).
pub(super) fn d(sides: PValue) -> Result<PValue, PErr> {
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
pub(super) fn py_iter(x: PValue) -> Result<Vec<PValue>, PErr> {
    Ok(match x {
        PValue::List(xs) | PValue::Tuple(xs) => xs,
        PValue::Value(Value::Tuple(t)) => t.iter().cloned().map(PValue::Value).collect(),
        PValue::Value(Value::Str(s)) => s
            .chars()
            .map(|c| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Die(d) => d.outcomes().cloned().map(PValue::Value).collect(),
        PValue::Value(v) => {
            return type_error(format!("'{}' object is not iterable", v.type_name()))
        }
        PValue::Reroll => return type_error("'RerollType' object is not iterable"),
        PValue::Func(_) => return Err(PErr::Unimplemented("iterating a function".into())),
        PValue::Again(_) => return type_error("'AgainExpression' object is not iterable"),
    })
}

/// `expand_arg`'s view of one outcome: a die is mixed in, a hashable value is an outcome, `Reroll` drops out, a
/// tuple holding dice is `tupleize`d, a list is unhashable.
pub(super) fn die_arg(x: PValue) -> Result<DieArg<Value>, PErr> {
    match x {
        PValue::Value(v) => Ok(DieArg::Outcome(v)),
        PValue::Die(d) => Ok(DieArg::Die(d)),
        PValue::Reroll => Ok(DieArg::Reroll),
        PValue::Tuple(items) => map::tupleize(items),
        PValue::List(_) => type_error("unhashable type: 'list'"),
        PValue::Func(_) => Err(PErr::Unimplemented("a function as an outcome".into())),
        // `Die(...)` replaces top-level expressions first (`die_with_agains`); one nested in a tuple is not handled.
        PValue::Again(_) => Err(PErr::Unimplemented(
            "an Again expression nested in an outcome".into(),
        )),
    }
}

/// `icepool.Die(outcomes)` (nodes `die`, `die_typed`) and `icepool.Die(outcomes, times)` (node `die_times`).
fn die(outcomes: PValue, times: Option<PValue>) -> Result<PValue, PErr> {
    die_with_agains(outcomes, times, None, PValue::Value(Value::None))
}

/// `icepool.Die(outcomes, times, again_depth=again_depth, again_end=again_end)`: outcomes that are `Again`
/// expressions are rolled again (population/again.py, depth mode); without any, `again_depth` and `again_end` are
/// ignored.
pub(super) fn die_with_agains(
    outcomes: PValue,
    times: Option<PValue>,
    again_depth: Option<PValue>,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let (objects, times) = itemize(outcomes, times)?;
    if objects.iter().any(|x| matches!(x, PValue::Again(_))) {
        // Count mode (`again_count`) is not ported yet; the default is depth mode with depth 1.
        let depth = again_depth.unwrap_or(PValue::Value(Value::from(1)));
        return again::evaluate_agains_using_depth(objects, times, depth, again_end);
    }
    new_die(objects, &times)
}

/// The rest of `Die.__new__` once the times are checked to be non-negative numbers and any `Again` is replaced.
pub(super) fn new_die(objects: Vec<PValue>, times: &[Value]) -> Result<PValue, PErr> {
    let args: Vec<DieArg<Value>> = objects.into_iter().map(die_arg).collect::<Result<_, _>>()?;
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
fn itemize(keys: PValue, times: Option<PValue>) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    // A die is a Mapping: its outcomes become the keys and its quantities multiply the times. Each outcome then has
    // denominator 1, so zero quantities drop out.
    let (args, base): (Vec<PValue>, Option<Vec<Quantity>>) = match keys {
        PValue::Die(d) => {
            let (a, q) = d
                .items()
                .iter()
                .map(|(o, q)| (PValue::Value(o.clone()), q.clone()))
                .unzip();
            (a, Some(q))
        }
        // Any other sequence: each element once. `len()` of an int, float, None... raises TypeError.
        x @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Value(Value::Str(_) | Value::Tuple(_))) => (py_iter(x)?, None),
        PValue::Func(_) => return Err(PErr::Unimplemented("Die(mapping)".into())),
        _ => return type_error("Argument appears not to be a mapping or sequence."),
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Value(Value),
    Die(Die<Value>),
}

/// `{k: v for k, v in items}`: each item unpacks into exactly two; a repeated key (by ==, e.g. True and 1) keeps
/// its first spelling and its last value. Returned in insertion order.
pub(super) fn dict_items(items: PValue) -> Result<Vec<(Key, PValue)>, PErr> {
    let mut entries: Vec<(Key, PValue)> = Vec::new();
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
    let (keys, values): (Vec<Key>, Vec<PValue>) = dict_items(items)?.into_iter().unzip();
    let args: Vec<PValue> = keys
        .into_iter()
        .map(|k| match k {
            Key::Value(v) => PValue::Value(v),
            Key::Die(d) => PValue::Die(d),
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
