//! The repeated forms of `map` (map_tools/function.py): `map(repeat=n)`, `map(repeat='inf')`, `map_and_time`,
//! `mean_time_to_absorb`, and `Die.time_to_sum`, which is built on `map_and_time`.
//!
//! This layer does Python's argument juggling (`transition_and_star`, `Die([first_arg])`, the checks on `repeat`
//! in Python's order) and supplies the transition function: a catalogue callable called with the state and one
//! joint outcome of the extra arguments, its result read as a `Mapped`. The core runs the process.

use super::catalogue::{truthy, Func};
use super::expression::as_expression;
use super::map::{arg_items, call, die_of, expand_arg, star_arg, the_die};
use super::ops::{new_die, unhashable_list, wrong_type};
use super::{raise, PErr, PValue};
use icepool_core::value::{iter_cartesian_product, TransitionCache};
use icepool_core::{Mapped, Target};
use icepool_core::value::BinOp;
use icepool_core::Error;
use icepool_core::{Die, Quantity, Value};

/// A transition function as the core's `TransitionCache` calls it: `transition(state, i)`, `i` the joint
/// outcome of the extra arguments.
type TransitionFn<'a> = dyn FnMut(&Value, usize) -> Result<Mapped<Value>, PErr> + 'a;

fn int(i: i64) -> Value {
    Value::from(dashu_int::IBig::from(i))
}

/// What the transition function returned, as `TransitionCache` reads it.
fn next(result: PValue) -> Result<Mapped<Value>, PErr> {
    let target = |x: PValue| match x {
        PValue::Value(v) => Ok(Target::Outcome(v)),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(Target::Die(d)),
        // `Break(Reroll)`, `Break(Restart)`: `(BREAK, Reroll)` cannot be expanded by `Die()`.
        PValue::Reroll | PValue::Restart => Ok(Target::Reroll),
        // A list is unhashable: `curr_state in self._self_loop_cache` raises.
        PValue::List(_) => unhashable_list(),
        // Tuples holding dice stay tuples here (not tupleized), Again expressions are not evaluated: Python makes
        // odd outcomes of them.
        _ => Err(PErr::Unimplemented(
            "map(repeat) states that are not values or dice".into(),
        )),
    };
    Ok(match result {
        PValue::Reroll => Mapped::Reroll,
        PValue::Restart => Mapped::Restart,
        PValue::Break(outcome) => match *outcome {
            PValue::Value(Value::None) => Mapped::Break(None),
            outcome => Mapped::Break(Some(target(outcome)?)),
        },
        x => target(x)?.into(),
    })
}

/// A `TransitionCache(repl, *extra_args, star=star, **kwargs)`, i.e. `transition_and_star(repl, len(extra_args) +
/// 1, star)`, returning the transition function's arguments: the callable, `star` and the joint outcomes of the
/// extra arguments with their quantities.
type Transition<'a> = (&'a Func, bool, Vec<(Vec<PValue>, Quantity)>);

fn transition_cache<'a>(
    f: &'a PValue,
    extra_args: Vec<PValue>,
    star: &PValue,
) -> Result<Transition<'a>, PErr> {
    let star = star_arg(star)?;
    let (f, star) = match f {
        PValue::Func(f) if f.positional().is_some() => (
            &**f,
            match star {
                Some(star) => star,
                None => f.infer_star(extra_args.len() + 1)?,
            },
        ),
        PValue::Func(_) if !extra_args.is_empty() => {
            return raise(Error::InvalidArgument(
                "If a mapping is provided for repl, len(args) must be 1.".into(),
            ))
        }
        PValue::Func(f) => (&**f, false),
        PValue::Die(_) => return Err(PErr::Unimplemented("a Die as repl".into())),
        _ => return wrong_type("repl must be a callable or a mapping."),
    };
    if extra_args.iter().any(|a| as_expression(a).is_some()) {
        // Only `mean_time_to_absorb` gets here with one (`map` and `map_and_time` expanded it): it is passed to
        // `repl` as-is.
        return Err(PErr::Unimplemented(
            "a multiset expression passed as-is to a transition function".into(),
        ));
    }
    let items: Vec<_> = extra_args.into_iter().map(arg_items).collect();
    Ok((f, star, iter_cartesian_product(&items).collect()))
}

/// Runs `body` with the `TransitionCache` for `(f, star, joint)`: the transition calls `f(state, *extra)`
/// (`f(*state, *extra)` with `star`).
fn with_cache<T>(
    (f, star, joint): Transition<'_>,
    body: impl FnOnce(
        &mut TransitionCache<Value, &mut TransitionFn<'_>>,
    ) -> Result<T, PErr>,
) -> Result<T, PErr> {
    let quantities = joint.iter().map(|(_, q)| q.clone()).collect();
    let mut transition = |state: &Value, i: usize| {
        let mut args = vec![PValue::Value(state.clone())];
        args.extend(joint[i].0.iter().cloned());
        next(call(f, star, args, &[])?)
    };
    let transition: &mut TransitionFn<'_> = &mut transition;
    body(&mut TransitionCache::new(transition, quantities))
}

/// `Die([x])` as a core die.
fn die_of_one(x: PValue) -> Result<Die<Value>, PErr> {
    the_die(new_die(vec![x], &[int(1)])?)
}

/// `range(repeat)`'s view of `repeat`: `None` where it raises (not an int), a negative int is no steps. A count
/// beyond `u64` would run until the process converges either way.
fn range_count(repeat: &PValue) -> Option<u64> {
    match repeat {
        PValue::Value(Value::Int(i)) => Some(u64::try_from(i.clone()).unwrap_or(
            if *i < dashu_int::IBig::ZERO {
                0
            } else {
                u64::MAX
            },
        )),
        PValue::Value(Value::Bool(b)) => Some(*b as u64),
        _ => None,
    }
}

/// `icepool.map(repl, *args, star=star, repeat=repeat)` with a `repeat` that is not `None`.
pub fn map_repeat(f: PValue, args: PValue, star: PValue, repeat: PValue) -> Result<PValue, PErr> {
    let PValue::List(args) = args else {
        return Err(PErr::Unimplemented(
            "map arguments not given as a list".into(),
        ));
    };
    if args.is_empty() {
        return raise(Error::InvalidArgument(
            "If no arguments are given, repeat cannot be used.".into(),
        ));
    }
    let mut args = args
        .into_iter()
        .map(expand_arg)
        .collect::<Result<Vec<_>, _>>()?;
    let extra_args = args.split_off(1);
    let first_arg = args.pop().expect("one argument left");
    let transition = transition_cache(&f, extra_args, &star)?;

    if matches!(&repeat, PValue::Value(Value::Str(s)) if &**s == "inf") {
        // Infinite repeat.
        let initial_die = die_of_one(first_arg)?;
        let (die, _) = with_cache(transition, |cache| {
            icepool_core::value::absorbing_markov_chain(cache, &initial_die)
        })?;
        return Ok(PValue::Die(die));
    }
    // `repeat < 0`, then `repeat == 0`.
    let PValue::Value(r) = &repeat else {
        return Err(PErr::Unimplemented("a repeat that is not a value".into()));
    };
    if r.binary(BinOp::Lt, &int(0))?.truthy()? {
        return raise(Error::InvalidArgument("repeat cannot be negative.".into()));
    }
    if *r == int(0) {
        return new_die(vec![first_arg], &[int(1)]);
    }
    let initial_die = die_of_one(first_arg)?;
    let die = with_cache(transition, |cache| {
        icepool_core::value::map_repeat(cache, &initial_die, range_count(&repeat))
    })?;
    Ok(PValue::Die(die))
}

/// `icepool.map_and_time(repl, initial_state, *extra_args, star=star, repeat=repeat)`: a die of `(state, time)`.
pub fn map_and_time(
    f: PValue,
    initial_state: PValue,
    extra_args: PValue,
    star: PValue,
    repeat: PValue,
) -> Result<PValue, PErr> {
    let PValue::List(extra_args) = extra_args else {
        return Err(PErr::Unimplemented(
            "map arguments not given as a list".into(),
        ));
    };
    let extra_args = extra_args
        .into_iter()
        .map(expand_arg)
        .collect::<Result<Vec<_>, _>>()?;
    let transition = transition_cache(&f, extra_args, &star)?;
    let initial_die = die_of_one(initial_state)?;
    let die = with_cache(transition, |cache| {
        icepool_core::value::map_and_time(cache, &initial_die, range_count(&repeat))
    })?;
    Ok(PValue::Die(die))
}

/// `icepool.mean_time_to_absorb(repl, initial_state, *extra_args, star=star)`: a `Fraction`.
pub fn mean_time_to_absorb(
    f: PValue,
    initial_state: PValue,
    extra_args: PValue,
    star: PValue,
) -> Result<PValue, PErr> {
    let PValue::List(extra_args) = extra_args else {
        return Err(PErr::Unimplemented(
            "map arguments not given as a list".into(),
        ));
    };
    let transition = transition_cache(&f, extra_args, &star)?;
    // `absorbing_markov_chain_impl` makes `Die([initial_state])` itself.
    let initial_die = die_of_one(initial_state)?;
    let (_, time) = with_cache(transition, |cache| {
        icepool_core::value::absorbing_markov_chain(cache, &initial_die)
    })?;
    match time {
        Some(time) => Ok(PValue::Value(Value::Fraction(time))),
        None => raise(Error::NotImplementedUpstream(
            "Restart not implemented for mean_time_to_absorb.".into(),
        )),
    }
}

/// `die.mean_time_to_absorb(repl, *extra_args, star=star)`: the method form, "as `mean_time_to_absorb(repl, self,
/// ...)`", so the same code with the die as the initial state.
pub fn die_mean_time_to_absorb(
    die: PValue,
    f: PValue,
    extra_args: PValue,
    star: PValue,
) -> Result<PValue, PErr> {
    // Only dice have the method (a `DieWithTruth` passes on as the plain die, as `self`).
    let die = PValue::Die(the_die(die)?);
    mean_time_to_absorb(f, die, extra_args, star)
}

/// `die.time_to_sum(target, max_time, dnf)`: the number of rolls until the cumulative sum is greater or equal to
/// the target. If the sum is not reached in `max_time` rolls, the time is `dnf` (default `max_time`;
/// `Reroll` removes those cases).
pub fn time_to_sum(
    die: PValue,
    target: PValue,
    max_time: PValue,
    dnf: PValue,
) -> Result<PValue, PErr> {
    let die = the_die(die)?;
    let PValue::Value(target) = target else {
        return Err(PErr::Unimplemented("a target that is not a value".into()));
    };
    if target.binary(BinOp::Le, &int(0))?.truthy()? {
        return die_of(vec![PValue::Value(int(0))], vec![Quantity::ONE]);
    }
    let max_time = match max_time {
        PValue::Value(Value::None) => {
            let min = die.min_outcome()?;
            if min.binary(BinOp::Le, &int(0))?.truthy()? {
                return raise(Error::InvalidArgument(
                    "max_time must be provided if not all outcomes are positive.".into(),
                ));
            }
            // `(target + self.min_outcome() - 1) // self.min_outcome()`.
            PValue::Value(
                target
                    .binary(BinOp::Add, min)?
                    .binary(BinOp::Sub, &int(1))?
                    .binary(BinOp::FloorDiv, min)?,
            )
        }
        x => x,
    };
    let dnf = match dnf {
        PValue::Value(Value::None) => max_time.clone(),
        x => x,
    };
    // `Die([0]).map_and_time(step, self, repeat=max_time)` with `step = lambda total, roll: min(total + roll,
    // target)`.
    let rolls = die.items();
    let mut step = |total: &Value, i: usize| -> Result<Mapped<Value>, PErr> {
        let sum = total.binary(BinOp::Add, &rolls[i].0)?;
        Ok(Mapped::Outcome(Value::py_min(sum, target.clone())?))
    };
    let step: &mut TransitionFn<'_> = &mut step;
    let mut cache = TransitionCache::new(step, rolls.iter().map(|(_, q)| q.clone()).collect());
    let initial = Die::new(
        vec![icepool_core::value::DieArg::Outcome(int(0))],
        vec![Quantity::ONE],
    )?;
    let result = icepool_core::value::map_and_time(&mut cache, &initial, range_count(&max_time))?;
    // `result.map(get_time)`: `dnf if total < target else time`, star inferred.
    let mut states = Vec::with_capacity(result.len());
    let mut quantities = Vec::with_capacity(result.len());
    for (outcome, q) in result.items() {
        let Value::Tuple(t) = outcome else {
            unreachable!("map_and_time outcomes are (state, time)")
        };
        let reached = !truthy(&PValue::Value(t[0].binary(BinOp::Lt, &target)?))?;
        states.push(if reached {
            PValue::Value(t[1].clone())
        } else {
            dnf.clone()
        });
        quantities.push(q.clone());
    }
    die_of(states, quantities)
}

/// `b = icepool.Break(outcome)`; `(repr(b), str(b), hash(b) == hash(b))` (node `break_str`): `Break({outcome!r})`,
/// `Break({outcome})`, and `__hash__` hashes `(Break, self.outcome)`, which raises for an outcome Python cannot
/// hash.
pub fn break_str(outcome: PValue) -> Result<PValue, PErr> {
    use super::expression::{pvalue_repr, pvalue_str};
    let repr = format!("Break({})", pvalue_repr(&outcome)?);
    let text = format!("Break({})", pvalue_str(&outcome)?);
    match &outcome {
        PValue::Value(v) => v.check_hashable()?,
        PValue::Die(_) | PValue::DieWithTruth(..) => {}
        _ => return Err(PErr::Unimplemented("hash() of this Break outcome".into())),
    }
    Ok(PValue::Value(Value::tuple(vec![
        Value::str(&repr),
        Value::str(&text),
        Value::Bool(true),
    ])))
}
