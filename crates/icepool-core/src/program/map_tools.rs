//! The repeated forms of `map` (map_tools/function.py): `map(repeat=n)`, `map(repeat='inf')`, `map_and_time`,
//! `mean_time_to_absorb`, and `Die.time_to_sum`, which is built on `map_and_time`.
//!
//! This layer does Python's argument juggling (`transition_and_star`, `Die([first_arg])`, the checks on `repeat`
//! in Python's order) and supplies the transition function: a catalogue callable called with the state and one
//! joint outcome of the extra arguments, its result read as a `Next`. The core runs the process.

use super::catalogue::{truthy, Func};
use super::map::{arg_items, call, die_of, star_arg, the_die};
use super::ops::{new_die, type_error};
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::expand::iter_cartesian_product;
use crate::map_tools::{self, Next, Target, TransitionCache};
use crate::value::BinOp;
use crate::{Die, Quantity, Value};

fn int(i: i64) -> Value {
    Value::from(dashu_int::IBig::from(i))
}

/// What the transition function returned, as `TransitionCache` reads it.
fn next(result: PValue) -> Result<Next, PErr> {
    let target = |x: PValue| match x {
        PValue::Value(v) => Ok(Target::Outcome(v)),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(Target::Die(d)),
        // `Break(Reroll)`, `Break(Restart)`: `(BREAK, Reroll)` cannot be expanded by `Die()`.
        PValue::Reroll | PValue::Restart => Ok(Target::Reroll),
        // A list is unhashable: `curr_state in self._self_loop_cache` raises.
        PValue::List(_) => type_error("unhashable type: 'list'"),
        // Tuples holding dice stay tuples here (not tupleized), Again expressions are not evaluated: Python makes
        // odd outcomes of them.
        _ => Err(PErr::Unimplemented(
            "map(repeat) states that are not values or dice".into(),
        )),
    };
    Ok(match result {
        PValue::Reroll => Next::Reroll,
        PValue::Restart => Next::Restart,
        PValue::Break(outcome) => match *outcome {
            PValue::Value(Value::None) => Next::Break(None),
            outcome => Next::Break(Some(target(outcome)?)),
        },
        x => Next::Go(target(x)?),
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
            star.unwrap_or_else(|| f.infer_star(extra_args.len() + 1)),
        ),
        PValue::Func(_) if !extra_args.is_empty() => {
            return raise(Error::Value(
                "If a mapping is provided for repl, len(args) must be 1.".into(),
            ))
        }
        PValue::Func(f) => (&**f, false),
        PValue::Die(_) => return Err(PErr::Unimplemented("a Die as repl".into())),
        _ => return type_error("repl must be a callable or a mapping."),
    };
    if extra_args.iter().any(|a| matches!(a, PValue::Pool(_))) {
        // Python expands a multiset expression into its sorted tuples (`arg.expand()`).
        return Err(PErr::Unimplemented(
            "a pool as a map(repeat) argument".into(),
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
        &mut TransitionCache<&mut dyn FnMut(&Value, usize) -> Result<Next, PErr>>,
    ) -> Result<T, PErr>,
) -> Result<T, PErr> {
    let quantities = joint.iter().map(|(_, q)| q.clone()).collect();
    let mut transition = |state: &Value, i: usize| {
        let mut args = vec![PValue::Value(state.clone())];
        args.extend(joint[i].0.iter().cloned());
        next(call(f, star, args, &[])?)
    };
    let transition: &mut dyn FnMut(&Value, usize) -> Result<Next, PErr> = &mut transition;
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
    let PValue::List(mut args) = args else {
        return Err(PErr::Unimplemented(
            "map arguments not given as a list".into(),
        ));
    };
    if args.is_empty() {
        return raise(Error::Value(
            "If no arguments are given, repeat cannot be used.".into(),
        ));
    }
    if matches!(args[0], PValue::Pool(_)) {
        return Err(PErr::Unimplemented(
            "a pool as a map(repeat) argument".into(),
        ));
    }
    let extra_args = args.split_off(1);
    let first_arg = args.pop().expect("one argument left");
    let transition = transition_cache(&f, extra_args, &star)?;

    if matches!(&repeat, PValue::Value(Value::Str(s)) if &**s == "inf") {
        // Infinite repeat.
        let initial_die = die_of_one(first_arg)?;
        let (die, _) = with_cache(transition, |cache| {
            map_tools::absorbing_markov_chain(cache, &initial_die)
        })?;
        return Ok(PValue::Die(die));
    }
    // `repeat < 0`, then `repeat == 0`.
    let PValue::Value(r) = &repeat else {
        return Err(PErr::Unimplemented("a repeat that is not a value".into()));
    };
    if r.binary(BinOp::Lt, &int(0))?.truthy()? {
        return raise(Error::Value("repeat cannot be negative.".into()));
    }
    if *r == int(0) {
        return new_die(vec![first_arg], &[int(1)]);
    }
    let initial_die = die_of_one(first_arg)?;
    let die = with_cache(transition, |cache| {
        map_tools::map_repeat(cache, &initial_die, range_count(&repeat))
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
    let transition = transition_cache(&f, extra_args, &star)?;
    let initial_die = die_of_one(initial_state)?;
    let die = with_cache(transition, |cache| {
        map_tools::map_and_time(cache, &initial_die, range_count(&repeat))
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
        map_tools::absorbing_markov_chain(cache, &initial_die)
    })?;
    match time {
        Some(time) => Ok(PValue::Value(Value::Fraction(time))),
        None => raise(Error::NotImplemented(
            "Restart not implemented for mean_time_to_absorb.".into(),
        )),
    }
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
                return raise(Error::Value(
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
    let mut step = |total: &Value, i: usize| -> Result<Next, PErr> {
        let sum = total.binary(BinOp::Add, &rolls[i].0)?;
        Ok(Next::Go(Target::Outcome(Value::py_min(
            sum,
            target.clone(),
        )?)))
    };
    let step: &mut dyn FnMut(&Value, usize) -> Result<Next, PErr> = &mut step;
    let mut cache = TransitionCache::new(step, rolls.iter().map(|(_, q)| q.clone()).collect());
    let initial = Die::new(vec![crate::DieArg::Outcome(int(0))], vec![Quantity::ONE])?;
    let result = map_tools::map_and_time(&mut cache, &initial, range_count(&max_time))?;
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
