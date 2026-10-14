//! Pool nodes (harness/grammar/pool.py): building pools, keeping, popping, and the lowest / highest of dice.
//!
//! Evaluating a pool (`.sum()`, `pool[int]`, ...) is the evaluator's job (task K6). Until then the cases that need
//! it say `Unimplemented`, after doing everything Python does before it gets there (so the raises still match).

use dashu_int::IBig;

use super::ops::{binary, itemize, new_die, py_iter, to_die, type_error, unary};
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::keep_tuple::{highest_slice, lowest_slice, middle_slice, KeepIndex, Slice, Tie};
use crate::order::Order;
use crate::value::{BinOp, UnOp};
use crate::{Die, Pool, Value};

fn evaluation_needed<T>(what: &str) -> Result<T, PErr> {
    Err(PErr::Unimplemented(format!(
        "{what} (multiset evaluation, task K6)"
    )))
}

/// A Python int (or bool) argument, exactly. `None` for anything else.
fn int_value(x: &PValue) -> Option<IBig> {
    match x {
        PValue::Value(Value::Int(i)) => Some(i.clone()),
        PValue::Value(Value::Bool(b)) => Some(IBig::from(*b as u8)),
        _ => None,
    }
}

/// An int used as a slice bound or a keep / drop count. Values beyond `i64` saturate: they only ever get clamped
/// to a pool's size.
fn saturating_int(x: &PValue) -> Option<i64> {
    int_value(x)
        .map(|i| i64::try_from(&i).unwrap_or(if i < IBig::ZERO { i64::MIN } else { i64::MAX }))
}

/// An `int | None` argument such as `keep=` or a slice bound.
fn opt_int(x: &PValue, what: &str) -> Result<Option<i64>, PErr> {
    match x {
        PValue::Value(Value::None) => Ok(None),
        x => match saturating_int(x) {
            Some(i) => Ok(Some(i)),
            None => Err(PErr::Unimplemented(format!("a non-int {what}"))),
        },
    }
}

/// An int that must be exact (a keep count, a multiplier).
fn exact_int(x: &PValue, what: &str) -> Result<i64, PErr> {
    int_value(x)
        .and_then(|i| i64::try_from(i).ok())
        .ok_or_else(|| PErr::Unimplemented(format!("{what} that is not a 64-bit int")))
}

fn the_pool(x: PValue) -> Result<Pool<Value>, PErr> {
    match x {
        PValue::Pool(p) => Ok(p),
        // Anything else lacks the pool's methods: AttributeError in Python.
        PValue::Value(_) | PValue::Die(_) | PValue::Reroll | PValue::Ellipsis => {
            type_error("not a pool")
        }
        _ => Err(PErr::Unimplemented("this multiset expression".into())),
    }
}

/// `implicit_convert_to_die`, which refuses an `Again` expression (ImplicitConversionError).
fn convert_to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Again(_) => type_error("Again expression cannot be implicitly converted to a Die."),
        x => to_die(x),
    }
}

/// A count of dice in the pool. Python would build a huge keep tuple; the port refuses beyond `usize`.
fn die_count(t: &Value) -> Result<usize, PErr> {
    match t {
        Value::Int(i) => {
            usize::try_from(i).map_err(|_| PErr::Raise(Error::Overflow("too many dice".into())))
        }
        Value::Bool(b) => Ok(*b as usize),
        // `(1, ) * sum(times)` with a float or Fraction in the sum.
        _ => type_error("can't multiply sequence by non-int"),
    }
}

/// `icepool.Pool(dice, times)`.
pub fn pool(dice: PValue, times: PValue) -> Result<PValue, PErr> {
    let dice = match dice {
        // `Pool(pool)` is the pool itself.
        PValue::Pool(p) if int_value(&times) == Some(IBig::ONE) => return Ok(PValue::Pool(p)),
        PValue::Pool(_) => return Err(PErr::Unimplemented("Pool(pool, times)".into())),
        PValue::Die(_) => {
            return raise(Error::Value(
                "A Pool cannot be constructed with a Die argument.".into(),
            ))
        }
        dice => dice,
    };
    let (dice, times) = itemize(dice, Some(times))?;
    let dice = dice
        .into_iter()
        .map(convert_to_die)
        .collect::<Result<Vec<_>, _>>()?;
    let counts = times.iter().map(die_count).collect::<Result<Vec<_>, _>>()?;
    Ok(PValue::Pool(Pool::new(
        dice.into_iter().zip(counts).collect(),
    )?))
}

/// `Die.pool(rolls)`: `Pool({die: rolls})` for an int, else `Pool({die: len(rolls)})[rolls]`.
pub fn die_pool(die: PValue, rolls: PValue) -> Result<PValue, PErr> {
    let PValue::Die(die) = die else {
        return type_error("pool() needs a Die");
    };
    if let Some(rolls) = int_value(&rolls) {
        return Ok(PValue::Pool(new_die_pool(&die, rolls)?));
    }
    let index = sequence_index(rolls)?;
    let KeepIndex::Seq(ref counts) = index else {
        unreachable!("sequence_index makes a sequence")
    };
    let pool = new_die_pool(&die, IBig::from(counts.len()))?;
    Ok(PValue::Pool(pool.keep(&index)?))
}

/// `Pool({die: rolls})`: `itemize` multiplies the count by 1 and refuses a negative one.
fn new_die_pool(die: &Die<Value>, rolls: IBig) -> Result<Pool<Value>, PErr> {
    if rolls < IBig::ZERO {
        return raise(Error::Value("times cannot have negative values.".into()));
    }
    Ok(Pool::new(vec![(
        die.clone(),
        die_count(&Value::Int(rolls))?,
    )])?)
}

/// A sequence of `int`s and `...` as a keep index.
fn sequence_index(index: PValue) -> Result<KeepIndex, PErr> {
    let items = py_iter(index)?;
    let counts = items
        .iter()
        .map(|x| match x {
            PValue::Ellipsis => Ok(None),
            x => exact_int(x, "a keep count").map(Some),
        })
        .collect::<Result<_, _>>()?;
    Ok(KeepIndex::Seq(counts))
}

/// `pool.keep(index)` / `pool[index]` for an `int` or a sequence of counts.
pub fn pool_keep(pool: PValue, index: PValue) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    match int_value(&index) {
        Some(i) => {
            let i = i64::try_from(i).unwrap_or(i64::MAX);
            // The keep tuple (or its IndexError) first; then Python evaluates it into a Die.
            pool.keep(&KeepIndex::Int(i))?;
            evaluation_needed("pool[int]")
        }
        None => Ok(PValue::Pool(pool.keep(&sequence_index(index)?)?)),
    }
}

/// `pool[start:stop:step]`.
pub fn pool_slice(pool: PValue, start: PValue, stop: PValue, step: PValue) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    let step = !matches!(step, PValue::Value(Value::None));
    // A bound that is neither an int nor None is only looked at when the list is sliced, after the checks for
    // negative counts and for a step.
    let bound = |x: &PValue| match x {
        PValue::Value(Value::None) => Ok(None),
        x => saturating_int(x).map(Some).ok_or(()),
    };
    let slice = match (bound(&start), bound(&stop)) {
        (Ok(start), Ok(stop)) => Slice::new(start, stop),
        _ if pool.has_negative_keeps() || step => Slice::new(None, None),
        _ => {
            return type_error("slice indices must be integers or None or have an __index__ method")
        }
    };
    Ok(PValue::Pool(pool.keep(&KeepIndex::Slice { slice, step })?))
}

/// `pool.highest(keep, drop)` / `pool.lowest(keep, drop)`: keep a slice from the top or the bottom.
pub fn pool_highest_lowest(
    highest: bool,
    pool: PValue,
    keep: PValue,
    drop: PValue,
) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    let (keep, drop) = (opt_int(&keep, "keep")?, opt_int(&drop, "drop")?);
    let slice = if highest {
        highest_slice(keep, drop)?
    } else {
        lowest_slice(keep, drop)?
    };
    Ok(PValue::Pool(
        pool.keep(&KeepIndex::Slice { slice, step: false })?,
    ))
}

/// `pool.middle(keep, tie=tie)`.
pub fn pool_middle(pool: PValue, keep: PValue, tie: PValue) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    let keep = exact_int(&keep, "keep")?;
    let tie = match &tie {
        PValue::Value(Value::Str(s)) => match &**s {
            "error" => Tie::Error,
            "high" => Tie::High,
            "low" => Tie::Low,
            _ => Tie::Invalid,
        },
        _ => Tie::Invalid,
    };
    let slice = middle_slice(pool.keep_size()?, keep, tie)?;
    Ok(PValue::Pool(
        pool.keep(&KeepIndex::Slice { slice, step: false })?,
    ))
}

/// `pool * n` (without a pool, the plain operator): an int multiplies every count. Anything else is `NotImplemented` for the pool, and then for the
/// other operand too (a die's `__rmul__` would make a die of pools: not ported).
pub fn pool_mul(pool: PValue, n: PValue) -> Result<PValue, PErr> {
    if !matches!((&pool, &n), (PValue::Pool(_), _) | (_, PValue::Pool(_))) {
        return binary(BinOp::Mul, pool, n);
    }
    let pool = the_pool(pool)?;
    match n {
        PValue::Die(_) => Err(PErr::Unimplemented("pool * die".into())),
        n if int_value(&n).is_some() => Ok(PValue::Pool(
            pool.multiply_counts(exact_int(&n, "a multiplier")?)?,
        )),
        _ => type_error("unsupported operand type(s) for *: 'Pool'"),
    }
}

/// `-pool` (without a pool, the plain operator).
pub fn pool_neg(pool: PValue) -> Result<PValue, PErr> {
    if !matches!(pool, PValue::Pool(_)) {
        return unary(UnOp::Neg, pool);
    }
    Ok(PValue::Pool(the_pool(pool)?.multiply_counts(-1)?))
}

/// `a + b` (without a pool, the plain operator). For two pools: one pool if every sorted position counts the same.
pub fn pool_add(a: PValue, b: PValue) -> Result<PValue, PErr> {
    if !matches!((&a, &b), (PValue::Pool(_), _) | (_, PValue::Pool(_))) {
        return binary(BinOp::Add, a, b);
    }
    let pools = [the_pool(a)?, the_pool(b)?];
    match Pool::additive_union(&pools)? {
        Some(p) => Ok(PValue::Pool(p)),
        None => Err(PErr::Unimplemented(
            "adding pools with different counts (CompoundKeepGenerator)".into(),
        )),
    }
}

fn int(n: impl Into<IBig>) -> Value {
    Value::Int(n.into())
}

fn int_tuple(xs: &[i64]) -> Value {
    Value::tuple(xs.iter().map(|&x| int(x)).collect())
}

/// `pool._dice` / `source.dice`: a tuple of `(die, count)` pairs.
fn dice_tuple(dice: &[(Die<Value>, usize)]) -> PValue {
    PValue::tuple(
        dice.iter()
            .map(|(d, c)| PValue::tuple(vec![PValue::Die(d.clone()), PValue::Value(int(*c))]))
            .collect(),
    )
}

/// `(keep_tuple(), keep_size(), raw_size(), denominator(), outcomes(), _dice)`.
pub fn pool_info(pool: PValue) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    Ok(PValue::tuple(vec![
        PValue::Value(int_tuple(pool.keep_tuple())),
        PValue::Value(int(pool.keep_size()?)),
        PValue::Value(int(pool.raw_size())),
        PValue::Value(int(pool.denominator())),
        PValue::Value(Value::tuple(pool.outcomes().to_vec())),
        dice_tuple(pool.dice()),
    ]))
}

/// What `pool._make_source().pop(Order(order), outcome)` yields, as `(dice, outcomes, keep_tuple, count, weight)`.
pub fn pool_pop(pool: PValue, order: PValue, outcome: PValue) -> Result<PValue, PErr> {
    let pool = the_pool(pool)?;
    // `Order(order)` looks the value up by equality (so `True` is `Ascending`).
    let order = match &order {
        PValue::Value(v) => [Order::Ascending, Order::Descending, Order::Any]
            .into_iter()
            .find(|o| *v == int(*o as i64)),
        _ => None,
    }
    .ok_or_else(|| PErr::Raise(Error::Value("not a valid Order".into())))?;
    let PValue::Value(outcome) = outcome else {
        return Err(PErr::Unimplemented("popping a non-value outcome".into()));
    };
    let popped = pool.source().pop(order, &outcome)?;
    Ok(PValue::tuple(
        popped
            .into_iter()
            .map(|(source, count, weight)| {
                PValue::tuple(vec![
                    dice_tuple(source.dice()),
                    PValue::Value(Value::tuple(source.outcomes().to_vec())),
                    PValue::Value(int_tuple(source.keep_tuple())),
                    PValue::Value(int(count)),
                    PValue::Value(int(weight)),
                ])
            })
            .collect(),
    ))
}

/// Above this many rolls the port refuses `x ** rolls` (Python would compute for minutes).
const MAX_ROLLS: i64 = 1 << 16;

/// `die.highest(rolls, keep, drop)` / `die.lowest(rolls, keep, drop)`.
pub fn die_highest_lowest(
    highest: bool,
    die: PValue,
    rolls: PValue,
    keep: PValue,
    drop: PValue,
) -> Result<PValue, PErr> {
    let PValue::Die(die) = die else {
        return type_error("highest() / lowest() need a Die");
    };
    let rolls_int = int_value(&rolls).ok_or_else(|| PErr::Unimplemented("non-int rolls".into()))?;
    let (keep, drop) = (opt_int(&keep, "keep")?, opt_int(&drop, "drop")?);
    let index = if highest {
        highest_slice(keep, drop)?
    } else {
        lowest_slice(keep, drop)?
    };
    let rolls = saturating_int(&rolls).expect("an int");
    let (start, stop) = index.canonical(rolls);
    // The single highest / lowest. (With `rolls == 0` the canonical slice is empty, so `_highest_single(0)`, the
    // zero die, is never reached from here.)
    let single = if highest {
        start == rolls - 1 && stop == rolls
    } else {
        start == 0 && stop == 1
    };
    if single {
        if rolls > MAX_ROLLS {
            return raise(Error::Overflow("too many rolls".into()));
        }
        let rolls = rolls as usize;
        let result = if highest {
            die.highest_single(rolls)?
        } else {
            die.lowest_single(rolls)?
        };
        return Ok(PValue::Die(result));
    }
    // `self.pool(rolls)[index].sum()`.
    new_die_pool(&die, rolls_int)?.keep(&KeepIndex::Slice {
        slice: index,
        step: false,
    })?;
    evaluation_needed("pool.sum()")
}

/// `icepool.highest(args, keep=, drop=, default=)` / `icepool.lowest(...)` with the dice in one iterable.
pub fn highest_lowest(
    highest: bool,
    args: PValue,
    keep: PValue,
    drop: PValue,
    default: PValue,
) -> Result<PValue, PErr> {
    let args = match args {
        // `len(args)`: a die has one, but iterating it gives its outcomes.
        args @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Die(_)
        | PValue::Value(Value::Str(_) | Value::Tuple(_))) => py_iter(args)?,
        PValue::Value(v) => {
            return type_error(format!("object of type '{}' has no len()", v.type_name()))
        }
        _ => return Err(PErr::Unimplemented("highest() of this argument".into())),
    };
    if args.is_empty() {
        return match default {
            PValue::Value(Value::None) => raise(Error::Value(
                "highest() / lowest() arg is an empty sequence and no default was provided.".into(),
            )),
            default => new_die(vec![default], &[int(1)]),
        };
    }
    let (keep, drop) = (opt_int(&keep, "keep")?, opt_int(&drop, "drop")?);
    let index = if highest {
        highest_slice(keep, drop)?
    } else {
        lowest_slice(keep, drop)?
    };
    sum_slice(args, index)
}

/// `_sum_slice`: common code for `lowest` and `highest`. Keeps `index` of the sorted rolls of the dice and sums.
fn sum_slice(args: Vec<PValue>, index: Slice) -> Result<PValue, PErr> {
    let dice = args
        .into_iter()
        .map(convert_to_die)
        .collect::<Result<Vec<_>, _>>()?;
    if dice.iter().any(Die::is_empty) {
        return Ok(PValue::Die(Die::from_dict(Vec::new())?));
    }
    // Faster special cases.
    let len = dice.len() as i64;
    let (start, stop) = index.canonical(len);
    // `sum()` starts from the int 0.
    let sum = |dice: Vec<Die<Value>>| {
        dice.into_iter().try_fold(PValue::Value(int(0)), |acc, d| {
            binary(BinOp::Add, acc, PValue::Die(d))
        })
    };
    if start == stop {
        // Zero-size slice. The inputs should support addition.
        let zeros = dice.iter().map(Die::zero).collect::<Result<Vec<_>, _>>();
        // `sum(die.zero() for die in dice)` interleaves: a zero() that raises after an addition that raises is
        // never reached, but both raise either way.
        return sum(zeros?);
    }
    if start == 0 && stop == len {
        // All dice selected.
        return sum(dice);
    }
    if start == 0 && stop == 1 {
        return Ok(PValue::Die(crate::keep::lowest_single(&dice)?));
    }
    if start == len - 1 && stop == len {
        return Ok(PValue::Die(crate::keep::highest_single(&dice)?));
    }
    // Use pool.
    Pool::new(dice.into_iter().map(|d| (d, 1)).collect())?.keep(&KeepIndex::Slice {
        slice: index,
        step: false,
    })?;
    evaluation_needed("pool.sum()")
}
