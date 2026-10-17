//! Pool nodes (harness/grammar/pool.py): building pools, keeping, popping, and the lowest / highest of dice.
//!
//! Evaluating a pool (`pool[int]`, the pool path of `highest` / `lowest`) goes through the evaluators
//! (program/evaluate.rs).

use dashu_int::IBig;

use super::evaluate::run;
use super::expression::{
    as_expression, as_tuple_expression, binary_op, counts, into_pvalue, keepable_mixture,
    map_mixture, multiset_keep, multiset_keep_seq, the_expression, tuple_slice, KeepIndexArg,
    MExpr,
};
use super::ops::{
    binary, itemize, itemize_mapping, missing_attribute, new_die, py_iter, to_die, unary,
    unhashable_list, wrong_type,
};
use super::{raise, PErr, PValue};
use crate::compound_keep::CompoundKeep;
use crate::error::Error;
use crate::evaluator::basic::SumEvaluator;
use crate::evaluator::keep::KeepEvaluator;
use crate::keep_tuple::{highest_slice, lowest_slice, middle_slice, KeepIndex, Slice, Tie};
use crate::operator::MultisetKeepIndex;
use crate::order::Order;
use crate::pool::MultisetSource;
use crate::value::{BinOp, UnOp};
use crate::{Deal, Die, Int, Pool, Value};

/// `pool.sum()`.
fn pool_sum(pool: Pool<Value>) -> Result<PValue, PErr> {
    run(SumEvaluator, &[MExpr::Gen(KeepGen::Pool(pool))])
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

/// An int that must be exact (a keep count, a multiplier): Python's ints are unbounded, and so are counts.
fn exact_int(x: &PValue, what: &str) -> Result<Int, PErr> {
    int_value(x)
        .map(Int::from)
        .ok_or_else(|| PErr::Unimplemented(format!("{what} that is not an int")))
}

pub(super) fn the_pool(x: PValue) -> Result<Pool<Value>, PErr> {
    match x {
        PValue::Pool(p) => Ok(p),
        // Anything else lacks the pool's methods: AttributeError in Python.
        PValue::Value(_) | PValue::Die(_) | PValue::Reroll | PValue::Ellipsis => {
            wrong_type("not a pool")
        }
        _ => Err(PErr::Unimplemented("this multiset expression".into())),
    }
}

/// A `KeepGenerator`: a pool, a deal or a `CompoundKeepGenerator`. The keep-like operations and the evaluations
/// treat them alike.
#[derive(Clone, Debug)]
pub enum KeepGen {
    Pool(Pool<Value>),
    Deal(Deal<Value>),
    Compound(CompoundKeep<Value>),
}

impl KeepGen {
    /// `KeepGenerator.keep` for a keep tuple without negative counts.
    pub(super) fn keep(&self, index: &KeepIndex) -> Result<Self, Error> {
        Ok(match self {
            KeepGen::Pool(p) => KeepGen::Pool(p.keep(index)?),
            KeepGen::Deal(d) => KeepGen::Deal(d.keep(index)?),
            KeepGen::Compound(c) => KeepGen::Compound(c.keep(index)?),
        })
    }

    pub(super) fn multiply_counts(&self, constant: &Int) -> Self {
        match self {
            KeepGen::Pool(p) => KeepGen::Pool(p.multiply_counts(constant)),
            KeepGen::Deal(d) => KeepGen::Deal(d.multiply_counts(constant)),
            KeepGen::Compound(c) => KeepGen::Compound(c.multiply_counts(constant)),
        }
    }

    pub(super) fn keep_size(&self) -> Int {
        match self {
            KeepGen::Pool(p) => p.keep_size(),
            KeepGen::Deal(d) => d.keep_size(),
            KeepGen::Compound(c) => c.keep_size(),
        }
    }

    pub(super) fn has_negative_keeps(&self) -> bool {
        match self {
            KeepGen::Pool(p) => p.has_negative_keeps(),
            KeepGen::Deal(d) => d.has_negative_keeps(),
            KeepGen::Compound(c) => c.has_negative_keeps(),
        }
    }

    /// `_make_source`.
    pub(super) fn source(&self) -> Result<MultisetSource<Value>, Error> {
        Ok(match self {
            KeepGen::Pool(p) => MultisetSource::Pool(p.source()),
            KeepGen::Deal(d) => MultisetSource::Deal(d.source()),
            KeepGen::Compound(c) => MultisetSource::CompoundKeep(c.source()?),
        })
    }
}

/// A keep-like operation (`keep`, `[]`, `lowest`, `highest`, `middle`) on an expression: a keep generator without
/// negative keeps stays one; anything else falls back to the less-capable `MultisetKeep` expression.
fn keep_slice(e: MExpr, slice: Slice) -> Result<PValue, PErr> {
    if let Some(inners) = keepable_mixture(&e) {
        return map_mixture(inners, |inner| keep_slice(the_expression(inner)?, slice));
    }
    let result = match &e {
        MExpr::Gen(g) if !g.has_negative_keeps() => {
            MExpr::Gen(g.keep(&KeepIndex::Slice { slice, step: false })?)
        }
        _ => {
            let bound = |x: Option<i64>| PValue::Value(x.map_or(Value::None, Value::from));
            let given = [
                bound(slice.start),
                bound(slice.stop),
                PValue::Value(Value::None),
            ];
            multiset_keep(
                e,
                MultisetKeepIndex::Slice {
                    start: slice.start,
                    stop: slice.stop,
                    step: false,
                },
                false,
                KeepIndexArg::Slice(Box::new(given)),
            )
        }
    };
    Ok(into_pvalue(result))
}

/// `implicit_convert_to_die`, which refuses an `Again` expression (ImplicitConversionError).
fn convert_to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Again(_) => wrong_type("Again expression cannot be implicitly converted to a Die."),
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
        _ => wrong_type("can't multiply sequence by non-int"),
    }
}

/// `icepool.Pool(dice, times)`.
pub fn pool(dice: PValue, times: PValue) -> Result<PValue, PErr> {
    let (dice, times) = match dice {
        // `Pool(pool)` is the pool itself (`times == 1`, so also `True` or `1.0`).
        PValue::Pool(p) if matches!(&times, PValue::Value(t) if *t == Value::from(1)) => {
            return Ok(PValue::Pool(p))
        }
        // Otherwise the mapping `{die: quantity for die, quantity in dice._dice}`: the keep tuple and the dice put
        // in zero times are forgotten.
        PValue::Pool(p) => {
            let (keys, values) = p
                .dice()
                .iter()
                .map(|(d, c)| (PValue::Die(d.clone()), int(*c)))
                .unzip();
            itemize_mapping(keys, values, times)?
        }
        PValue::Die(_) => {
            return raise(Error::InvalidArgument(
                "A Pool cannot be constructed with a Die argument.".into(),
            ))
        }
        dice => itemize(dice, Some(times))?,
    };
    new_pool(dice, times)
}

/// `Pool(dict(items), times)`: `items` a sequence of `[key, count]` pairs.
pub fn pool_dict(items: PValue, times: PValue) -> Result<PValue, PErr> {
    let (keys, values): (Vec<PValue>, Vec<PValue>) = dict(items)?.into_iter().unzip();
    let values = values
        .into_iter()
        .map(|v| match v {
            PValue::Value(v) => Ok(v),
            _ => Err(PErr::Unimplemented(
                "a pool count that is not a value".into(),
            )),
        })
        .collect::<Result<_, _>>()?;
    let (dice, times) = itemize_mapping(keys, values, times)?;
    new_pool(dice, times)
}

/// Python's `dict(items)` over `[key, value]` pairs: equal keys merge, keeping the first key and the last value, in
/// insertion order. Keys are dice, outcomes or `Again` expressions (a die never equals an outcome as a dict key;
/// an expression hashes by its `hash_key`, so it only ever meets another expression with an equal one).
pub(super) fn dict(items: PValue) -> Result<Vec<(PValue, PValue)>, PErr> {
    let mut result: Vec<(PValue, PValue)> = Vec::new();
    for item in py_iter(items)? {
        let pair = py_iter(item)?;
        let [key, value]: [PValue; 2] = pair.try_into().map_err(|p: Vec<_>| {
            PErr::Raise(Error::InvalidArgument(format!(
                "dictionary update sequence element has length {}; 2 is required",
                p.len()
            )))
        })?;
        let key = match key {
            key @ (PValue::Die(_) | PValue::Value(_) | PValue::Again(_)) => key,
            // A `DieWithTruth` hashes and compares as its die (`hash_key`), which needs its data; the pool only
            // ever uses the die's data.
            PValue::DieWithTruth(d, _) => PValue::Die(d),
            PValue::DieWithTruthError(e, _) => return raise(e),
            PValue::List(_) => return unhashable_list(),
            _ => return Err(PErr::Unimplemented("this dict key".into())),
        };
        let same = |k: &PValue| match (k, &key) {
            (PValue::Die(a), PValue::Die(b)) => Ok(a == b),
            (PValue::Value(a), PValue::Value(b)) => Ok(a == b),
            (PValue::Again(a), PValue::Again(b)) => super::again::hash_key_eq(a, b),
            _ => Ok(false),
        };
        let mut found = None;
        for (i, (k, _)) in result.iter().enumerate() {
            if same(k)? {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(i) => result[i].1 = value,
            None => result.push((key, value)),
        }
    }
    Ok(result)
}

/// `icepool.d_pool(die_sizes)` (`z` false) or `icepool.z_pool(die_sizes)`; `mapping`: `die_sizes` is
/// `dict(die_sizes)`, sizes to how many dice of that size.
pub fn standard_pool(z: bool, die_sizes: PValue, mapping: bool) -> Result<PValue, PErr> {
    let sizes = if mapping {
        let sizes = dict(die_sizes)?;
        if sizes.is_empty() {
            return empty_standard_pool();
        }
        // `itertools.chain.from_iterable([k] * v for k, v in die_sizes.items())`.
        let mut list = Vec::new();
        for (k, v) in sizes {
            let n = match int_value(&v) {
                Some(n) if n < IBig::ZERO => 0,
                Some(n) => u64::try_from(&n).unwrap_or(u64::MAX),
                None => return wrong_type("can't multiply sequence by non-int"),
            };
            // Python's `MemoryError` for a list no memory holds (the pool size limit comes after).
            let n = crate::memory::check_len::<PValue, _>(
                (list.len() as u64).checked_add(n),
                "d_pool(die_sizes)",
            )? - list.len();
            list.extend(std::iter::repeat_n(k, n));
        }
        list
    } else {
        // `if not die_sizes` (also true for `0`, `''`, `None`...); a truthy non-sequence then fails to iterate.
        if !super::catalogue::truthy(&die_sizes)? {
            return empty_standard_pool();
        }
        py_iter(die_sizes)?
    };
    if sizes.is_empty() {
        // Only from a mapping whose counts are all <= 0: `Pool([])`.
        return Ok(PValue::Pool(Pool::new(Vec::new())?));
    }
    let dice = sizes
        .into_iter()
        .map(|x| {
            // `icepool.z(x)` checks its argument as `d(x)` does, and is `d(x) - 1`.
            let die = to_die(super::ops::d(x)?)?;
            let die = if z {
                die.unary_operator(|o| o.binary(BinOp::Sub, &Value::from(1)))?
            } else {
                die
            };
            Ok((die, 1))
        })
        .collect::<Result<Vec<_>, PErr>>()?;
    Ok(PValue::Pool(Pool::new(dice)?))
}

/// `Pool({icepool.Die([0]): 0})`: if empty, the pool will be considered to consist of zero zeros.
fn empty_standard_pool() -> Result<PValue, PErr> {
    Ok(PValue::Pool(Pool::new(vec![(
        Die::from_dict(vec![(Value::from(0), crate::Quantity::ONE)])?,
        0,
    )])?))
}

/// The end of `Pool.__new__`, after `itemize`.
fn new_pool(dice: Vec<PValue>, times: Vec<Value>) -> Result<PValue, PErr> {
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
    let die = match die {
        // A `DieWithTruth` (the result of `==`) is a die too.
        PValue::Die(die) | PValue::DieWithTruth(die, _) => die,
        PValue::DieWithTruthError(e, _) => return raise(e),
        _ => return wrong_type("pool() needs a Die"),
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
        return raise(Error::InvalidArgument(
            "times cannot have negative values.".into(),
        ));
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

/// `pool.keep(index)` / `pool[index]` for an `int` or a sequence of counts. On an expression other than a keep
/// generator without negative keeps: `MultisetExpression.keep`, i.e. the keep evaluator with that index, or a
/// `MultisetKeep`.
pub fn pool_keep(pool: PValue, index: PValue) -> Result<PValue, PErr> {
    let e = the_expression(pool)?;
    if let Some(inners) = keepable_mixture(&e) {
        if int_value(&index).is_none() {
            // An inner with negative keeps makes a `MultisetKeep` whose hash key holds the index itself: a list
            // cannot be hashed into the mixture's dict (`add_inner`).
            return map_mixture(inners, |inner| pool_keep(inner, index.clone()));
        }
        // Each inner's `keep(int)` is a die; `keep_evaluator.evaluate(MultisetMixture({die: weight}), index=None)`
        // then asks the first die for `_has_parameter` (an empty mixture evaluates to an empty die).
        for (inner, _) in inners {
            pool_keep(into_pvalue(inner.clone()), index.clone())?;
        }
        if inners.is_empty() {
            return Ok(PValue::Die(Die::new(Vec::new(), Vec::new())?));
        }
        return missing_attribute("'Die' object has no attribute '_has_parameter'");
    }
    match (&e, int_value(&index)) {
        (MExpr::Gen(g), Some(i)) if !g.has_negative_keeps() => {
            let i = i64::try_from(i).unwrap_or(i64::MAX);
            // The keep tuple (or its IndexError) first; then Python evaluates it into a Die.
            let kept = g.keep(&KeepIndex::Int(i))?;
            run(KeepEvaluator { index: None }, &[MExpr::Gen(kept)])
        }
        (MExpr::Gen(g), None) if !g.has_negative_keeps() => {
            Ok(into_pvalue(MExpr::Gen(g.keep(&sequence_index(index)?)?)))
        }
        (_, Some(i)) => run(
            KeepEvaluator {
                index: Some(i.into()),
            },
            &[e],
        ),
        (_, None) => {
            let list = matches!(index, PValue::List(_));
            let given = KeepIndexArg::Given(index.clone());
            Ok(into_pvalue(multiset_keep(
                e,
                multiset_keep_seq(index)?,
                list,
                given,
            )))
        }
    }
}

/// `pool[start:stop:step]`.
pub fn pool_slice(pool: PValue, start: PValue, stop: PValue, step: PValue) -> Result<PValue, PErr> {
    // A multiset tuple expression's `__getitem__`: some of its multisets.
    if as_tuple_expression(&pool).is_some() {
        return tuple_slice(pool, start, stop, step);
    }
    let e = the_expression(pool)?;
    if let Some(inners) = keepable_mixture(&e) {
        return map_mixture(inners, |inner| {
            pool_slice(inner, start.clone(), stop.clone(), step.clone())
        });
    }
    let step_arg = step.clone();
    let step = !matches!(step, PValue::Value(Value::None));
    let pool = match &e {
        MExpr::Gen(g) if !g.has_negative_keeps() => g,
        // `MultisetKeep(index=slice)`: the bounds are only looked at when evaluated.
        _ => {
            let bound = |x: &PValue| match x {
                PValue::Value(Value::None) => Ok(None),
                x => saturating_int(x).map(Some).ok_or_else(|| {
                    PErr::Unimplemented("a MultisetKeep slice bound that is not an int".into())
                }),
            };
            let index = MultisetKeepIndex::Slice {
                start: bound(&start)?,
                stop: bound(&stop)?,
                step,
            };
            let given = KeepIndexArg::Slice(Box::new([start, stop, step_arg]));
            return Ok(into_pvalue(multiset_keep(e, index, false, given)));
        }
    };
    // A bound that is neither an int nor None is only looked at when the list is sliced, after the checks for
    // negative counts and for a step.
    let bound = |x: &PValue| match x {
        PValue::Value(Value::None) => Ok(None),
        x => saturating_int(x).map(Some).ok_or(()),
    };
    let slice = match (bound(&start), bound(&stop)) {
        (Ok(start), Ok(stop)) => Slice::new(start, stop),
        _ if step => Slice::new(None, None),
        _ => {
            return wrong_type("slice indices must be integers or None or have an __index__ method")
        }
    };
    Ok(into_pvalue(MExpr::Gen(
        pool.keep(&KeepIndex::Slice { slice, step })?,
    )))
}

/// `pool.highest(keep, drop)` / `pool.lowest(keep, drop)`: keep a slice from the top or the bottom.
pub fn pool_highest_lowest(
    highest: bool,
    pool: PValue,
    keep: PValue,
    drop: PValue,
) -> Result<PValue, PErr> {
    let e = the_expression(pool)?;
    let (keep, drop) = (opt_int(&keep, "keep")?, opt_int(&drop, "drop")?);
    let slice = if highest {
        highest_slice(keep, drop)?
    } else {
        lowest_slice(keep, drop)?
    };
    keep_slice(e, slice)
}

/// `pool.middle(keep, tie=tie)`.
pub fn pool_middle(pool: PValue, keep: PValue, tie: PValue) -> Result<PValue, PErr> {
    // Only keep generators have `middle`.
    let pool = match the_expression(pool)? {
        MExpr::Gen(g) => g,
        _ => return missing_attribute("object has no attribute 'middle'"),
    };
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
    let slice = middle_slice(&pool.keep_size(), &keep, tie)?;
    keep_slice(MExpr::Gen(pool), slice)
}

/// `die.middle(rolls, keep, tie=tie)`: `self.pool(rolls).middle(keep, tie=tie).sum()`.
pub fn die_middle(die: PValue, rolls: PValue, keep: PValue, tie: PValue) -> Result<PValue, PErr> {
    let pool = die_pool(die, rolls)?;
    super::evaluate::evaluation("pool_sum", pool_middle(pool, keep, tie)?)
}

/// `die.keep(rolls, index)`, with `index` an int, a sequence of counts or `None`. An int `rolls` needs an index: an
/// int index is `self.pool(rolls).keep(index)` (a Die), anything else `self.pool(rolls).keep(index).sum()`. Without
/// an index, `rolls` is itself the sequence of counts: `self.pool(len(rolls)).keep(rolls).sum()`.
pub fn die_keep(die: PValue, rolls: PValue, index: PValue) -> Result<PValue, PErr> {
    if !matches!(die, PValue::Die(_)) {
        return missing_attribute("object has no attribute 'keep'");
    }
    let no_index = matches!(index, PValue::Value(Value::None));
    if int_value(&rolls).is_some() {
        if no_index {
            return raise(Error::InvalidArgument(
                "If the number of rolls is an integer, an index argument must be provided.".into(),
            ));
        }
        let pool = die_pool(die, rolls)?;
        if int_value(&index).is_some() {
            return pool_keep(pool, index);
        }
        return super::evaluate::evaluation("pool_sum", pool_keep(pool, index)?);
    }
    if !no_index {
        return raise(Error::InvalidArgument(
            "Only one index sequence can be given.".into(),
        ));
    }
    let len = py_iter(rolls.clone())?.len();
    let pool = die_pool(die, PValue::Value(int(len)))?;
    super::evaluate::evaluation("pool_sum", pool_keep(pool, rolls)?)
}

/// `die.keep(rolls, slice(start, stop, step))`: `self.pool(rolls)[start:stop:step].sum()`. A non-int `rolls` with
/// an index raises.
pub fn die_keep_slice(
    die: PValue,
    rolls: PValue,
    start: PValue,
    stop: PValue,
    step: PValue,
) -> Result<PValue, PErr> {
    if !matches!(die, PValue::Die(_)) {
        return missing_attribute("object has no attribute 'keep'");
    }
    if int_value(&rolls).is_none() {
        return raise(Error::InvalidArgument(
            "Only one index sequence can be given.".into(),
        ));
    }
    let pool = die_pool(die, rolls)?;
    super::evaluate::evaluation("pool_sum", pool_slice(pool, start, stop, step)?)
}

/// `icepool.middle(args, keep=keep, tie=tie, default=default)` with the dice (or outcomes) in one iterable.
pub fn middle(args: PValue, keep: PValue, tie: PValue, default: PValue) -> Result<PValue, PErr> {
    // `len(args) == 0`, before anything else looks at the arguments.
    let empty = match &args {
        PValue::List(a) | PValue::Tuple(a) => a.is_empty(),
        _ => return Err(PErr::Unimplemented("middle() of this argument".into())),
    };
    if empty {
        return match default {
            PValue::Value(Value::None) => raise(Error::InvalidArgument(
                "middle() arg is an empty sequence and no default was provided.".into(),
            )),
            default => new_die(vec![default], &[int(1)]),
        };
    }
    // `icepool.Pool(args).middle(keep, tie=tie).sum()`.
    let pool = pool(args, PValue::Value(int(1)))?;
    super::evaluate::evaluation("pool_sum", pool_middle(pool, keep, tie)?)
}

/// `pool * n` (without a pool, the plain operator): an int multiplies every count. Anything else is `NotImplemented` for the pool, and then for the
/// other operand too (a die's `__rmul__` would make a die of pools: not ported).
pub fn pool_mul(pool: PValue, n: PValue) -> Result<PValue, PErr> {
    // A multiset tuple expression has no `*`, and an int none for it.
    if as_tuple_expression(&pool).is_some() || as_tuple_expression(&n).is_some() {
        return wrong_type("unsupported operand type(s) for *");
    }
    match (as_expression(&pool).is_some(), as_expression(&n).is_some()) {
        (true, _) => counts(pool, "mul", PValue::Value(Value::None), n),
        // `n * pool`: the int returns NotImplemented, then `pool.__rmul__(n)`, which is `pool * n`.
        (false, true) => counts(n, "mul", PValue::Value(Value::None), pool),
        (false, false) => binary(BinOp::Mul, pool, n),
    }
}

/// `-pool` (without a pool, the plain operator).
pub fn pool_neg(pool: PValue) -> Result<PValue, PErr> {
    if as_tuple_expression(&pool).is_some() {
        return wrong_type("bad operand type for unary -");
    }
    if as_expression(&pool).is_none() {
        return unary(UnOp::Neg, pool);
    }
    counts(
        pool,
        "neg",
        PValue::Value(Value::None),
        PValue::Value(Value::None),
    )
}

/// `a + b` (without a multiset expression, the plain operator). For two pools: one pool if every sorted position
/// counts the same, else a `CompoundKeepGenerator`.
pub fn pool_add(a: PValue, b: PValue) -> Result<PValue, PErr> {
    let multiset = |x: &PValue| as_expression(x).is_some() || as_tuple_expression(x).is_some();
    if !multiset(&a) && !multiset(&b) {
        return binary(BinOp::Add, a, b);
    }
    binary_op("+", a, b)
}

fn int(n: impl Into<IBig>) -> Value {
    Value::Int(n.into())
}

fn int_tuple(xs: &[Int]) -> Value {
    Value::tuple(xs.iter().map(int).collect())
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
        PValue::Value(int(pool.keep_size())),
        PValue::Value(int(pool.raw_size())),
        PValue::Value(int(pool.denominator()?)),
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
    .ok_or_else(|| PErr::Raise(Error::InvalidArgument("not a valid Order".into())))?;
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

/// `die.highest(rolls, keep, drop)` / `die.lowest(rolls, keep, drop)`.
pub fn die_highest_lowest(
    highest: bool,
    die: PValue,
    rolls: PValue,
    keep: PValue,
    drop: PValue,
) -> Result<PValue, PErr> {
    let PValue::Die(die) = die else {
        return wrong_type("highest() / lowest() need a Die");
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
        // `x ** rolls` of every cumulative quantity: the core refuses results no memory holds (`MemoryError`), and
        // a die whose quantities are all 0 or 1 answers for any number of rolls, as Python does.
        let rolls = usize::try_from(rolls).unwrap_or(usize::MAX);
        let result = if highest {
            die.highest_single(rolls)?
        } else {
            die.lowest_single(rolls)?
        };
        return Ok(PValue::Die(result));
    }
    // `self.pool(rolls)[index].sum()`.
    pool_sum(new_die_pool(&die, rolls_int)?.keep(&KeepIndex::Slice {
        slice: index,
        step: false,
    })?)
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
            return wrong_type(format!("object of type '{}' has no len()", v.type_name()))
        }
        _ => return Err(PErr::Unimplemented("highest() of this argument".into())),
    };
    if args.is_empty() {
        return match default {
            PValue::Value(Value::None) => raise(Error::InvalidArgument(
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
    pool_sum(
        Pool::new(dice.into_iter().map(|d| (d, 1)).collect())?.keep(&KeepIndex::Slice {
            slice: index,
            step: false,
        })?,
    )
}
