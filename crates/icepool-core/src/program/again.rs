//! `icepool.Again` in the program layer (population/again.py): building `AgainExpression`s with Python's
//! operators, `Die(..., again_depth=, again_end=)` in depth mode, and `Die.explode`.
//!
//! The expression itself is the core's closed enum (`crate::again::Again`), over this layer's operations and
//! objects; evaluating it calls back into the same Python operator dispatch as the operator nodes (`ops::binary`),
//! so `6 + Again` with `Again` replaced by a die is exactly `6 + die`.

use std::sync::Arc;

use dashu_int::IBig;

use super::map::{select, the_die};
use super::ops::{self, new_die, type_error};
use super::{raise, PErr, PValue};
use crate::again::{Again, AgainArg};
use crate::error::Error;
use crate::value::{BinOp, UnOp};
use crate::{Die, Value};

/// The functions an `AgainExpression` can apply: Python's `operator.add` & co.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgainOp {
    Binary(BinOp),
    Unary(UnOp),
    MatMul,
}

pub type AgainExpr = Again<AgainOp, PValue>;

/// An argument of an expression: `_evaluate_arg` resolves those that are themselves `Again` expressions.
fn arg(x: PValue) -> AgainArg<AgainOp, PValue> {
    match x {
        PValue::Again(a) => AgainArg::Again(a),
        x => AgainArg::Other(x),
    }
}

/// `AgainExpression(function, *args)`.
pub fn apply(op: AgainOp, args: Vec<PValue>) -> PValue {
    PValue::Again(Arc::new(Again::Apply(
        op,
        args.into_iter().map(arg).collect(),
    )))
}

/// Calls the function of an `AgainExpression` on its evaluated arguments.
fn call(op: &AgainOp, args: Vec<PValue>) -> Result<PValue, PErr> {
    let mut args = args.into_iter();
    let mut next = || args.next().expect("an operation gets its arguments");
    match *op {
        AgainOp::Binary(op) => ops::binary(op, next(), next()),
        AgainOp::Unary(op) => ops::unary(op, next()),
        AgainOp::MatMul => ops::matmul(next(), next()),
    }
}

/// A non-negative int `again_depth` (or `depth` of `explode`). Python would loop for ages on a huge one.
fn depth_arg(depth: &PValue, name: &str) -> Result<u64, PErr> {
    let depth = match depth {
        PValue::Value(Value::Int(i)) => i.clone(),
        PValue::Value(Value::Bool(b)) => IBig::from(*b as u8),
        PValue::Value(Value::Float(_) | Value::Fraction(_)) => {
            return Err(PErr::Unimplemented(format!("non-int {name}")))
        }
        _ => return type_error(format!("'<' not supported between {name} and 'int'")),
    };
    if depth < IBig::ZERO {
        return raise(Error::Value(format!("{name} cannot be negative.")));
    }
    match u64::try_from(depth) {
        Ok(d) if d <= 10_000 => Ok(d),
        _ => Err(PErr::Unimplemented(format!("huge {name}"))),
    }
}

/// `compute_not_again_die(outcomes, times).zero_outcome()`: the default `again_end`.
fn zero_outcome(objects: &[PValue], times: &[Value]) -> Result<PValue, PErr> {
    // `outcome not in icepool.REROLL_TYPES` compares with `==`: a die equals `Reroll` (which converts to the empty
    // die) only if it is empty itself.
    let (outcomes, times): (Vec<PValue>, Vec<Value>) = objects
        .iter()
        .zip(times)
        .filter(|(x, _)| match x {
            PValue::Reroll | PValue::Again(_) => false,
            PValue::Die(d) => !d.is_empty(),
            _ => true,
        })
        .map(|(x, t)| (x.clone(), t.clone()))
        .unzip();
    if outcomes.is_empty() {
        // `icepool.Die(*zip(*filtered))` with nothing left calls `Die()`.
        return type_error("Die.__new__() missing 1 required positional argument: 'outcomes'");
    }
    let not_again_die = match new_die(outcomes, &times)? {
        PValue::Die(d) => d,
        _ => unreachable!("new_die returns a die"),
    };
    Ok(PValue::Value(not_again_die.zero()?.min_outcome()?.clone()))
}

/// `evaluate_agains_using_depth(outcomes, times, again_depth, again_end)` for itemized `Die` arguments.
pub fn evaluate_agains_using_depth(
    objects: Vec<PValue>,
    times: Vec<Value>,
    again_depth: PValue,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let again_depth = depth_arg(&again_depth, "again_depth")?;
    let again_end = match again_end {
        PValue::Value(Value::None) => Some(zero_outcome(&objects, &times)?),
        PValue::Reroll => None,
        end => Some(end),
    };
    let outcomes: Vec<_> = objects.into_iter().map(arg).collect();
    crate::again::evaluate_agains_using_depth(
        &outcomes,
        again_depth,
        again_end.as_ref(),
        &PValue::Reroll,
        |objects| new_die(objects, &times),
        call,
    )
}

/// `die.explode(which, star=star, depth=depth, end=end)`: the selected outcomes (by default the max outcome) are
/// rolled again and added to the total, at most `depth` more times; past that, `end` stands for the next roll
/// (by default a zero; `Reroll` makes one extra final roll, rerolling until a non-exploding outcome is reached).
pub fn explode(
    die: PValue,
    which: PValue,
    star: PValue,
    depth: PValue,
    end: PValue,
) -> Result<PValue, PErr> {
    let die: Die<Value> = the_die(die)?;
    let selected = match which {
        PValue::Value(Value::None) => {
            die.min_outcome()?;
            let mut m = vec![false; die.len()];
            *m.last_mut().expect("not empty") = true;
            m
        }
        which => select(&die, &which, &star)?,
    };
    let depth_value = depth_arg(&depth, "depth")?;
    if depth_value == 0 {
        return Ok(PValue::Die(die));
    }
    // `self.map(map_final, again_depth=depth, again_end=end)`: every item, zero quantities included.
    let mut objects = Vec::with_capacity(die.len());
    let mut times = Vec::with_capacity(die.len());
    for ((outcome, quantity), &explodes) in die.items().iter().zip(&selected) {
        let outcome = PValue::Value(outcome.clone());
        objects.push(if explodes {
            ops::binary(BinOp::Add, outcome, PValue::Again(Arc::new(Again::Again)))?
        } else {
            outcome
        });
        times.push(Value::from(IBig::from(quantity.clone())));
    }
    if objects.iter().any(|x| matches!(x, PValue::Again(_))) {
        evaluate_agains_using_depth(objects, times, depth, end)
    } else {
        new_die(objects, &times)
    }
}
