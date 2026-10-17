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
