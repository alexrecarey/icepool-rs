//! `icepool.Again` in the program layer (population/again.py): building `AgainExpression`s with Python's
//! operators, `Die(..., again_depth=, again_count=, again_end=)` in both modes, and `Die.explode`.
//!
//! The expression itself is the core's closed enum (`icepool_core::Again`), over this layer's operations and
//! objects; evaluating it calls back into the same Python operator dispatch as the operator nodes (`ops::binary`),
//! so `6 + Again` with `Again` replaced by a die is exactly `6 + die`.

use std::sync::Arc;

use dashu_int::IBig;

use super::map::{select, the_die};
use super::ops::{self, new_die, wrong_type};
use super::{raise, PErr, PValue};
use icepool_core::value::{AgainExpression, AgainArg, StepOutcome};
use icepool_core::value::{BinOp, UnOp};
use icepool_core::Error;
use icepool_core::{Die, Quantity, Value};
use icepool_core::value::DieArg;

/// The functions an `AgainExpression` can apply: Python's `operator.add` & co.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgainOp {
    Binary(BinOp),
    Unary(UnOp),
    MatMul,
    /// `operator.eq` (`ne`: `operator.ne`), as `__eq__` / `__ne__` build it: with the expression's `truth_value`,
    /// the only kind of expression that has one. The truth value is not part of the `hash_key`.
    Eq {
        ne: bool,
        truth: bool,
    },
}

impl AgainOp {
    /// `self._func == other._func` in `hash_key`: the same `operator` function (the truth value aside).
    fn same_function(&self, other: &AgainOp) -> bool {
        match (self, other) {
            (AgainOp::Eq { ne: a, .. }, AgainOp::Eq { ne: b, .. }) => a == b,
            (a, b) => a == b,
        }
    }
}

pub type AgainExpr = AgainExpression<AgainOp, PValue>;

/// An argument of an expression: `_evaluate_arg` resolves those that are themselves `Again` expressions.
fn arg(x: PValue) -> AgainArg<AgainOp, PValue> {
    match x {
        PValue::Again(a) => AgainArg::Again(a),
        x => AgainArg::Other(x),
    }
}

/// `AgainExpression(function, *args)`, as the operator dunders build it: with `is_additive` (compatible with
/// `again_count`) set as Python sets it. `+` (`__add__` / `__radd__`) is additive if every expression operand is;
/// `x @ expr` (`__rmatmul__`) if the expression is and `x` is not an expression, and a population `x` only if
/// `x.min_outcome() >= 0` (evaluated, so raising for an empty die, whatever the expression); anything else is not.
pub fn apply(op: AgainOp, args: Vec<PValue>) -> Result<PValue, PErr> {
    let additive = |x: &PValue| match x {
        PValue::Again(a) => a.is_additive(),
        _ => true,
    };
    let is_additive = match (op, args.as_slice()) {
        (AgainOp::Binary(BinOp::Add), args) => args.iter().all(additive),
        // `Again @ x` is `AgainExpression.__matmul__`: not additive.
        (AgainOp::MatMul, [PValue::Again(_), _]) => false,
        (AgainOp::MatMul, [PValue::Die(d) | PValue::DieWithTruth(d, _), e]) => {
            let non_negative = d
                .min_outcome()?
                .binary(BinOp::Ge, &Value::from(0))?
                .truthy()?;
            additive(e) && non_negative
        }
        (AgainOp::MatMul, [_, e]) => additive(e),
        _ => false,
    };
    Ok(PValue::Again(Arc::new(AgainExpression::Apply {
        function: op,
        args: args.into_iter().map(arg).collect(),
        is_additive,
    })))
}

/// Calls the function of an `AgainExpression` on its evaluated arguments.
fn call(op: &AgainOp, args: Vec<PValue>) -> Result<PValue, PErr> {
    let mut args = args.into_iter();
    let mut next = || args.next().expect("an operation gets its arguments");
    match *op {
        AgainOp::Binary(op) => ops::binary(op, next(), next()),
        AgainOp::Unary(op) => ops::unary(op, next()),
        AgainOp::MatMul => ops::matmul(next(), next()),
        AgainOp::Eq { ne, .. } => super::format::eq(next(), next(), ne),
    }
}

/// `a == b` (`ne`: `a != b`) with an expression on at least one side. Dice and plain values return
/// `NotImplemented` for an expression, and `==` reflects to itself, so the expression's `__eq__` runs with it as
/// `self`: `AgainExpression(operator.eq, self, other, truth_value=...)`. The truth value is whether the two
/// `hash_key`s are equal (`!=`: differ) when `other` is an expression too, else `False` (`!=`: `True`).
pub fn eq(a: PValue, b: PValue, ne: bool) -> Result<PValue, PErr> {
    let (this, other) = match (a, b) {
        (a @ PValue::Again(_), b) => (a, b),
        (a, b) => (b, a),
    };
    let truth = match (&this, &other) {
        (PValue::Again(x), PValue::Again(y)) => hash_key_eq(x, y)? != ne,
        _ => ne,
    };
    Ok(PValue::Again(Arc::new(AgainExpression::Apply {
        function: AgainOp::Eq { ne, truth },
        args: vec![arg(this), arg(other)],
        is_additive: false,
    })))
}

/// `bool(expr)`: only an expression from `==` or `!=` has a truth value.
pub fn truth_value(x: &AgainExpr) -> Result<bool, PErr> {
    match x {
        AgainExpression::Apply {
            function: AgainOp::Eq { truth, .. },
            ..
        } => Ok(*truth),
        _ => raise(Error::InvalidArgument(
            "An `Again` only has a truth value if it is the result of == or !=.".into(),
        )),
    }
}

/// `x.hash_key == y.hash_key`, the tuples `(AgainExpression, self._func, self._args)`: Python compares the
/// functions, then the argument tuples (lengths first, then element by element up to the first unequal one,
/// each with `==` and `bool`). Two expressions in there compare by their own `__eq__`, so by `hash_key` again;
/// an expression and anything else are unequal (the other side returns `NotImplemented`, and the expression's
/// truth value is `False`).
///
/// Expressions nest (from JS, as deep as its calls make them): a level each (`value::recursion`).
pub(super) fn hash_key_eq(x: &AgainExpr, y: &AgainExpr) -> Result<bool, PErr> {
    let _level = icepool_core::value::recursion::level()?;
    match (x, y) {
        // `_func` is `None` and `_args` is `()` for both.
        (AgainExpression::Again { .. }, AgainExpression::Again { .. }) => Ok(true),
        (
            AgainExpression::Apply {
                function: f,
                args: a,
                ..
            },
            AgainExpression::Apply {
                function: g,
                args: b,
                ..
            },
        ) => {
            if !f.same_function(g) || a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                let equal = match (a, b) {
                    (AgainArg::Again(a), AgainArg::Again(b)) => hash_key_eq(a, b)?,
                    (AgainArg::Other(a), AgainArg::Other(b)) => {
                        // Tuple `==` tries identity before `==`, so the same NaN object is equal to itself; the
                        // port cannot tell one NaN object from another. That only matters with a NaN on both
                        // sides: one side's NaN is never identical to anything on the other, so `==` decides.
                        if has_nan(a) && has_nan(b) {
                            return Err(PErr::Unimplemented(
                                "NaN in the hash_key of an Again expression".into(),
                            ));
                        }
                        super::catalogue::truthy(&super::format::eq(a.clone(), b.clone(), false)?)?
                    }
                    _ => false,
                };
                if !equal {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Whether a NaN is in a plain value or among a die's outcomes.
fn has_nan(x: &PValue) -> bool {
    fn value(v: &Value) -> bool {
        match v {
            Value::Float(f) => f.is_nan(),
            Value::Complex(re, im) => re.is_nan() || im.is_nan(),
            Value::Tuple(items) => items.iter().any(value),
            _ => false,
        }
    }
    match x {
        PValue::Value(v) => value(v),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => d.outcomes().any(value),
        PValue::Tuple(xs) | PValue::List(xs) => xs.iter().any(has_nan),
        _ => false,
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
        _ => return wrong_type(format!("'<' not supported between {name} and 'int'")),
    };
    if depth < IBig::ZERO {
        return raise(Error::InvalidArgument(format!(
            "{name} cannot be negative."
        )));
    }
    match u64::try_from(depth) {
        Ok(d) if d <= 10_000 => Ok(d),
        _ => Err(PErr::Unimplemented(format!("huge {name}"))),
    }
}

/// `compute_not_again_die(outcomes, times)`: a die of the outcomes that are not `Again` expressions.
fn not_again_die(objects: &[PValue], times: &[Value]) -> Result<Die<Value>, PErr> {
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
        return wrong_type("Die.__new__() missing 1 required positional argument: 'outcomes'");
    }
    the_die(new_die(outcomes, &times)?)
}

/// `die.zero_outcome()`: `die.zero().outcomes()[0]`.
fn zero_outcome(die: &Die<Value>) -> Result<Value, PErr> {
    Ok(die.zero()?.min_outcome()?.clone())
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
        PValue::Value(Value::None) => Some(PValue::Value(zero_outcome(&not_again_die(
            &objects, &times,
        )?)?)),
        PValue::Reroll => None,
        // Every outcome that is an expression raises this, so the first one does.
        PValue::Restart => {
            return raise(Error::InvalidArgument(
                "Restart is not compatible with again_depth.".into(),
            ))
        }
        end => Some(end),
    };
    let outcomes: Vec<_> = objects.into_iter().map(arg).collect();
    icepool_core::value::evaluate_agains_using_depth(
        &outcomes,
        again_depth,
        again_end.as_ref(),
        &PValue::Reroll,
        |objects| new_die(objects, &times),
        call,
    )
}

/// `evaluate_agains_using_count(outcomes, times, again_count, again_end)` for itemized `Die` arguments: each
/// `Again` is rolled again (up to `again_count` rolls in total), the extra rolls past that count as `again_end`
/// (by default a zero; `Reroll`: the outcomes that are not `Again`; `Restart`: the whole roll restarts).
pub fn evaluate_agains_using_count(
    objects: Vec<PValue>,
    times: Vec<Value>,
    again_count: PValue,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let again_count = depth_arg(&again_count, "again_count")?;
    let not_again_die = not_again_die(&objects, &times)?;
    let zero = zero_outcome(&not_again_die)?;
    let again_end_die = match again_end {
        PValue::Restart => None,
        end => {
            let end = match end {
                PValue::Value(Value::None) => PValue::Value(zero.clone()),
                PValue::Reroll => PValue::Die(not_again_die.clone()),
                end => end,
            };
            Some(the_die(new_die(vec![end], &[Value::from(1)])?)?)
        }
    };
    let int = |i: i64| PValue::Value(Value::from(IBig::from(i)));
    // `make_step_outcome` makes `Reroll`, `Restart` and empty dice `Reroll` (fixed upstream, see DIVERGENCES.md):
    // they drop out of the step die, so that roll is rerolled rather than ending a chain.
    let (objects, times): (Vec<PValue>, Vec<Value>) = objects
        .into_iter()
        .zip(times)
        .filter(|(x, _)| match x {
            PValue::Reroll | PValue::Restart => false,
            PValue::Die(d) => !d.is_empty(),
            _ => true,
        })
        .unzip();
    let outcomes: Vec<_> = objects.into_iter().map(arg).collect();
    let step_objects = icepool_core::value::step_outcomes(
        &outcomes,
        &PValue::Value(zero.clone()),
        &int(1),
        &int(0),
        call,
        |step| {
            // `icepool.tupleize(...)`: a tuple holding a die becomes a die of tuples in `Die(...)`, as here.
            Ok(match step {
                StepOutcome::NotAgain => {
                    PValue::tuple(vec![PValue::Value(zero.clone()), int(1), int(-1)])
                }
                StepOutcome::Again { flat, count } => {
                    PValue::tuple(vec![flat, int(0), ops::binary(BinOp::Sub, count, int(1))?])
                }
            })
        },
    )?;
    let step_die = the_die(new_die(step_objects, &times)?)?;
    // `n @ die` with an int `n` is `Die([n]) @ die` (`Die.__rmatmul__`).
    let matmul = |n: &Value, die: &Die<Value>| {
        let n = Die::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
        icepool_core::value::matmul_dice(&n, die)
    };
    Ok(PValue::Die(
        icepool_core::value::evaluate_agains_using_count(
            &step_die,
            &not_again_die,
            &zero,
            again_count,
            again_end_die.as_ref(),
            matmul,
        )?,
    ))
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
            ops::binary(
                BinOp::Add,
                outcome,
                PValue::Again(Arc::new(AgainExpression::Again { is_additive: true })),
            )?
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
