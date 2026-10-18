//! Port of icepool's `map_tools`: the repeated forms of `map` (function.py `map(repeat=n)`, `map_and_time`,
//! `mean_time_to_absorb`; `repeat='inf'` is markov_chain.py), over a `TransitionCache` (core_impl.py).
//!
//! The facade keeps the argument juggling (`transition_and_star`, expanding multiset expressions, `Die([first_arg])`,
//! the checks on `repeat`) and supplies the transition function; see `core_impl` for how it is called.

pub mod core_impl;
pub mod markov_chain;

pub use core_impl::{Next, Target, TransitionCache};
pub use markov_chain::absorbing_markov_chain;

use core_impl::{any_default, parts, RESTART};

use crate::error::Error;
use crate::value::builtins::{getitem, Index};
use crate::{Die, DieArg, Value};

/// `map(repl, first_arg, *extra_args, repeat=repeat)` for a positive int `repeat`, from `initial_die =
/// Die([first_arg])`: the result of a Markov process, stopping early once the entire state distribution has
/// converged to absorbing states.
///
/// `repeat` is `None` where Python's `range(repeat)` raises (a non-int that passed the facade's checks on
/// `repeat`): only after the initial self-loop die is built, as there.
pub fn map_repeat<F, E>(
    transition_cache: &mut TransitionCache<F>,
    initial_die: &Die<Value>,
    repeat: Option<u64>,
) -> std::result::Result<Die<Value>, E>
where
    F: FnMut(&Value, usize) -> std::result::Result<Next, E>,
    E: From<Error>,
{
    let mut transition_die = transition_cache.self_loop_die(initial_die)?;
    let repeat = repeat.ok_or_else(not_an_int)?;
    for _ in 0..repeat {
        transition_die = transition_cache.step_transition_die(&transition_die)?;
        if !any_default(&transition_die) {
            break;
        }
    }
    Ok(final_map(&transition_die)?)
}

fn not_an_int() -> Error {
    Error::WrongType("'repeat' object cannot be interpreted as an integer".into())
}

/// `transition_die.map(final_map, star=True)`: each `(transition_type, state)` becomes its state, except that
/// `Restart`ed ones become `Reroll`.
pub(crate) fn final_map(transition_die: &Die<Value>) -> crate::Result<Die<Value>> {
    let (states, quantities) = transition_die
        .items()
        .iter()
        .map(|(o, q)| {
            let p = parts(o);
            let state = if p[0] == core_impl::int(RESTART) {
                DieArg::Reroll
            } else {
                DieArg::Outcome(p[1].clone())
            };
            (state, q.clone())
        })
        .unzip();
    Die::new(states, quantities)
}

/// `map_and_time(repl, initial_state, *extra_args, repeat=repeat)` from `initial_die = Die([initial_state])`:
/// a die of `(state, time)`, where `time` is the number of repeats needed to reach an absorbing outcome (an
/// outcome that only leads to itself), or `repeat`, whichever is lesser.
///
/// `repeat` as for `map_repeat` (a negative one is `Some(0)`: `range` is then empty).
pub fn map_and_time<F, E>(
    transition_cache: &mut TransitionCache<F>,
    initial_die: &Die<Value>,
    repeat: Option<u64>,
) -> std::result::Result<Die<Value>, E>
where
    F: FnMut(&Value, usize) -> std::result::Result<Next, E>,
    E: From<Error>,
{
    let mut transition_die = transition_cache.self_loop_die_with_zero_time(initial_die)?;
    let repeat = repeat.ok_or_else(not_an_int)?;
    for _ in 0..repeat {
        transition_die = transition_cache.step_transition_die_with_time(&transition_die)?;
        if !any_default(&transition_die) {
            break;
        }
    }
    // `transition_die.marginals[1:]`.
    let slice = Index::Slice(core_impl::int(1), Value::None, Value::None);
    Ok(transition_die.unary_operator(|o| getitem(o, &slice))?)
}

#[cfg(test)]
mod tests {
    //! The repeated maps with transition functions written in Rust. The expected dice are Python icepool's answers;
    //! the comment above each gives the Python.

    use super::*;
    use crate::value::BinOp;
    use crate::Quantity;

    type Result<T> = crate::Result<T>;

    fn die(outcomes: impl IntoIterator<Item = Value>) -> Die<Value> {
        let mut data = crate::counts::OrderedMerge::default();
        for o in outcomes {
            data.add(o, Quantity::ONE);
        }
        Die::from_dict(data.into_items()).unwrap()
    }

    fn ints(xs: impl IntoIterator<Item = i64>) -> Die<Value> {
        die(xs.into_iter().map(Value::from))
    }

    /// `(outcome repr, quantity)` pairs.
    fn items(die: &Die<Value>) -> Vec<(String, u32)> {
        die.items()
            .iter()
            .map(|(o, q)| (o.to_string(), u32::try_from(q).unwrap()))
            .collect()
    }

    /// `lambda x: x + 1 if x < 6 else x`: climbs to 6 and stays there.
    fn climb(x: &Value, _: usize) -> Result<Next> {
        let six = Value::from(6);
        let next = if x.binary(BinOp::Lt, &six)?.truthy()? {
            x.binary(BinOp::Add, &Value::from(1))?
        } else {
            x.clone()
        };
        Ok(Next::Go(Target::Outcome(next)))
    }

    /// `lambda x: (x + d(2)).clip(None, 4)`: a die of next states, absorbed at 4.
    fn step_d2(x: &Value, _: usize) -> Result<Next> {
        let d2 = ints([1, 2]);
        let sum = die([x.clone()]).binary_operator(&d2, |a, b| a.binary(BinOp::Add, b))?;
        Ok(Next::Go(Target::Die(
            sum.try_clip(None, Some(&Value::from(4)))?,
        )))
    }

    #[test]
    fn map_with_a_repeat_count() {
        // `d(6).map(lambda x: x + 1 if x < 6 else x, repeat=2)`
        let mut cache = TransitionCache::new(climb, vec![Quantity::ONE]);
        let result: Result<_> = map_repeat(&mut cache, &ints(1..=6), Some(2));
        let expected = [("3", 1), ("4", 1), ("5", 1), ("6", 3)].map(|(o, q)| (o.to_string(), q));
        assert_eq!(items(&result.unwrap()), expected);
        // `range(repeat)` raises for a repeat that is not an int.
        let result: Result<_> = map_repeat(&mut cache, &ints(1..=6), None);
        assert!(matches!(result, Err(Error::WrongType(_))));
    }

    #[test]
    fn map_to_absorption() {
        // `icepool.map(lambda x: x + 1 if x < 6 else x, d(6), repeat='inf')`
        let mut cache = TransitionCache::new(climb, vec![Quantity::ONE]);
        let result: Result<_> = absorbing_markov_chain(&mut cache, &ints(1..=6));
        assert_eq!(items(&result.unwrap().0), [("6".to_string(), 1)]);
        // `icepool.mean_time_to_absorb(lambda x: (x + d(2)).clip(None, 4), 0)`
        let mut cache = TransitionCache::new(step_d2, vec![Quantity::ONE]);
        let (absorbed, mean_time) =
            absorbing_markov_chain::<_, Error>(&mut cache, &ints([0])).unwrap();
        assert_eq!(items(&absorbed), [("4".to_string(), 1)]);
        assert_eq!(mean_time.unwrap().to_string(), "23/8");
    }

    #[test]
    fn map_and_time_counts_the_steps() {
        // `icepool.map_and_time(lambda x: (x + d(2)).clip(None, 4), 0, repeat=10)`
        let mut cache = TransitionCache::new(step_d2, vec![Quantity::ONE]);
        let result: Result<_> = map_and_time(&mut cache, &ints([0]), Some(10));
        let expected =
            [("(4, 2)", 4), ("(4, 3)", 10), ("(4, 4)", 2)].map(|(o, q)| (o.to_string(), q));
        assert_eq!(items(&result.unwrap()), expected);
    }
}
