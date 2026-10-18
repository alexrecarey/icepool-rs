//! Port of icepool's `map_tools`: the repeated forms of `map` (function.py `map(repeat=n)`, `map_and_time`,
//! `mean_time_to_absorb`; `repeat='inf'` is markov_chain.py), over a `TransitionCache` (core_impl.py), generic over
//! the state type ([`MapState`]).
//!
//! The functions here take a `TransitionCache` over `transition(state, i)` and the initial die: the bindings keep
//! Python's argument juggling (`transition_and_star`, expanding multiset expressions, `Die([first_arg])`, the
//! checks on `repeat`) and supply the transition function; see `core_impl` for how it is called. Typed dice
//! have the plain forms as methods: [`Die::map_repeat`], [`Die::map_to_absorption`], [`Die::map_and_time`],
//! [`Die::mean_time_to_absorb`].

pub mod core_impl;
pub mod markov_chain;
mod state;

pub use core_impl::TransitionCache;
pub use markov_chain::absorbing_markov_chain;
pub use state::{MapState, Transition};

use core_impl::owned;

use crate::counts::OrderedMerge;
use crate::die_map::Mapped;
use crate::error::{Error, Result};
use crate::outcome::ErrInto;
use crate::{Die, DieArg, Quantity, Ratio};

/// `map(repl, first_arg, *extra_args, repeat=repeat)` for a positive int `repeat`, from `initial_die =
/// Die([first_arg])`: the result of a Markov process, stopping early once the entire state distribution has
/// converged to absorbing states.
///
/// `repeat` is `None` where Python's `range(repeat)` raises (a non-int that passed the facade's checks on
/// `repeat`): only after the initial self-loop die is built, as there.
pub fn map_repeat<S, F, E>(
    transition_cache: &mut TransitionCache<S, F>,
    initial_die: &Die<S>,
    repeat: Option<u64>,
) -> std::result::Result<Die<S>, E>
where
    S: MapState,
    F: FnMut(&S, usize) -> std::result::Result<Mapped<S>, E>,
    E: From<Error>,
{
    let mut transition_die = transition_cache.self_loop_die(initial_die)?;
    let repeat = repeat.ok_or_else(not_an_int)?;
    for _ in 0..repeat {
        transition_die = transition_cache.step_transition_die(&transition_die)?;
        if !transition_die
            .outcomes()
            .any(|o| matches!(S::read_transition(o), Transition::Default(_)))
        {
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
pub(crate) fn final_map<S: MapState>(transition_die: &Die<S::Transition>) -> Result<Die<S>> {
    let (states, quantities) = transition_die
        .items()
        .iter()
        .map(|(o, q)| {
            let state = match S::read_transition(o).state() {
                Some(state) => DieArg::Outcome((*state).clone()),
                None => DieArg::Reroll,
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
pub fn map_and_time<S, F, E>(
    transition_cache: &mut TransitionCache<S, F>,
    initial_die: &Die<S>,
    repeat: Option<u64>,
) -> std::result::Result<Die<S::WithTime>, E>
where
    S: MapState,
    F: FnMut(&S, usize) -> std::result::Result<Mapped<S>, E>,
    E: From<Error>,
{
    let mut transition_die = transition_cache.self_loop_die_with_zero_time(initial_die)?;
    let repeat = repeat.ok_or_else(not_an_int)?;
    for _ in 0..repeat {
        transition_die = transition_cache.step_transition_die_with_time(&transition_die)?;
        if !transition_die
            .outcomes()
            .any(|o| matches!(S::read_timed(o).0, Transition::Default(_)))
        {
            break;
        }
    }
    // `transition_die.marginals[1:]`: equal `(state, time)`s (one `DEFAULT`, one a `Break` to it) add up.
    let mut data = OrderedMerge::default();
    for (outcome, quantity) in transition_die.items() {
        let (transition, time) = S::read_timed(outcome);
        data.add(S::with_time(owned(transition), time)?, quantity.clone());
    }
    Ok(Die::from_mapping(data).err_into()?)
}

/// The repeated maps of a typed die, with a transition function of the state alone (Python's `map(f, die,
/// repeat=...)` and friends with no extra arguments). The function returns the next state as a [`Mapped`]:
/// an outcome, a die of them, `Reroll` (the transition is dropped), `Break` (stop here) or `Restart` (start the
/// whole process over).
impl<S: crate::Outcome> Die<S> {
    /// `die.map(f, repeat=repeat)`: `f` applied `repeat` times to the outcomes, stopping early once every state is
    /// absorbed (only leads to itself, or a `Break` reached it).
    ///
    /// ```
    /// use icepool_core::{d, Die, Mapped};
    /// // `d(6).map(lambda x: x + 1 if x < 6 else x, repeat=2)`
    /// let climbed = d(6).map_repeat(|&x| Mapped::Outcome(if x < 6 { x + 1 } else { x }), 2).unwrap();
    /// assert_eq!(climbed, Die::from_weights([(3, 1u32), (4, 1), (5, 1), (6, 3)]));
    /// ```
    pub fn map_repeat(&self, f: impl FnMut(&S) -> Mapped<S>, repeat: u64) -> Result<Self> {
        if repeat == 0 {
            // `Die([first_arg])`.
            return Ok(self.clone());
        }
        map_repeat(&mut typed_cache(f), self, Some(repeat))
    }

    /// `die.map(f, repeat='inf')`: the distribution of the absorbing states the process ends in, in simplest
    /// form (zero quantities dropped). A process with a chance of never ending is an [`Error::InvalidArgument`]
    /// ("Matrix has deficient rank").
    ///
    /// ```
    /// use icepool_core::{d, Die, Mapped};
    /// // `d(6).map(lambda x: x + 1 if x < 6 else x, repeat='inf')`
    /// assert_eq!(d(6).map_to_absorption(|&x| Mapped::Outcome((x + 1).min(6))).unwrap(), Die::single(6));
    /// ```
    pub fn map_to_absorption(&self, f: impl FnMut(&S) -> Mapped<S>) -> Result<Self> {
        Ok(absorbing_markov_chain(&mut typed_cache(f), self)?.0)
    }

    /// `icepool.map_and_time(f, die, repeat=repeat)`: a die of `(state, time)`, where `time` is the number of steps
    /// needed to reach an absorbing state, or `repeat`, whichever is lesser. A `Restart` is an
    /// [`Error::Unported`]: Python's result holds `(None, time)`.
    ///
    /// ```
    /// use icepool_core::{d, Die, Mapped};
    /// // `icepool.map_and_time(lambda x: (x + d(2)).clip(None, 4), 0, repeat=10)`
    /// let walk = Die::single(0).map_and_time(|&x| Mapped::Die((d(2) + x).clip(None, Some(&4))), 10).unwrap();
    /// assert_eq!(walk, Die::from_weights([((4, 2), 4u32), ((4, 3), 10), ((4, 4), 2)]));
    /// ```
    pub fn map_and_time(
        &self,
        f: impl FnMut(&S) -> Mapped<S>,
        repeat: u64,
    ) -> Result<Die<(S, u64)>> {
        map_and_time(&mut typed_cache(f), self, Some(repeat))
    }

    /// `icepool.mean_time_to_absorb(f, die)`: the mean number of steps to reach an absorbing state, exactly.
    /// Python does not implement it for a process that may `Restart` ([`Error::NotImplementedUpstream`]).
    ///
    /// ```
    /// use icepool_core::{d, Die, Mapped};
    /// // `icepool.mean_time_to_absorb(lambda x: (x + d(2)).clip(None, 4), 0)`
    /// let time = Die::single(0).mean_time_to_absorb(|&x| Mapped::Die((d(2) + x).clip(None, Some(&4))));
    /// assert_eq!(time.unwrap().to_string(), "23/8");
    /// ```
    pub fn mean_time_to_absorb(&self, f: impl FnMut(&S) -> Mapped<S>) -> Result<Ratio> {
        match absorbing_markov_chain(&mut typed_cache(f), self)?.1 {
            Some(time) => Ok(time),
            None => Err(Error::NotImplementedUpstream(
                "Restart not implemented for mean_time_to_absorb.".into(),
            )),
        }
    }
}

/// The cache over a typed transition function of the state alone: one extra joint outcome, with quantity 1.
fn typed_cache<S: MapState>(
    mut f: impl FnMut(&S) -> Mapped<S>,
) -> TransitionCache<S, impl FnMut(&S, usize) -> Result<Mapped<S>>> {
    TransitionCache::new(move |state: &S, _| Ok(f(state)), vec![Quantity::ONE])
}

#[cfg(test)]
mod tests {
    //! The repeated maps with transition functions written in Rust, on `Value` states (as the bindings run them)
    //! and on typed ones. The expected dice are Python icepool's answers; the comment above each gives the Python.

    use super::*;
    use crate::value::BinOp;
    use crate::{d, Target, Value};

    fn die(outcomes: impl IntoIterator<Item = Value>) -> Die<Value> {
        let mut data = OrderedMerge::default();
        for o in outcomes {
            data.add(o, Quantity::ONE);
        }
        Die::from_dict(data.into_items()).unwrap()
    }

    fn ints(xs: impl IntoIterator<Item = i64>) -> Die<Value> {
        die(xs.into_iter().map(Value::from))
    }

    /// `(outcome repr, quantity)` pairs.
    fn items<O: crate::OutcomeOrder>(die: &Die<O>) -> Vec<(String, u32)> {
        die.items()
            .iter()
            .map(|(o, q)| (format!("{o:?}"), u32::try_from(q).unwrap()))
            .collect()
    }

    fn expected<O: std::fmt::Debug>(items: &[(O, u32)]) -> Vec<(String, u32)> {
        items.iter().map(|(o, q)| (format!("{o:?}"), *q)).collect()
    }

    /// A typed die from `(outcome, quantity)` pairs.
    fn weights<O: crate::Outcome>(items: &[(O, u32)]) -> Die<O> {
        Die::from_weights(items.iter().cloned())
    }

    /// `lambda x: x + 1 if x < 6 else x`: climbs to 6 and stays there.
    fn climb(x: &Value, _: usize) -> Result<Mapped<Value>> {
        let six = Value::from(6);
        let next = if x.binary(BinOp::Lt, &six)?.truthy()? {
            x.binary(BinOp::Add, &Value::from(1))?
        } else {
            x.clone()
        };
        Ok(Mapped::Outcome(next))
    }

    /// `lambda x: (x + d(2)).clip(None, 4)`: a die of next states, absorbed at 4.
    fn step_d2(x: &Value, _: usize) -> Result<Mapped<Value>> {
        let d2 = ints([1, 2]);
        let sum = die([x.clone()]).binary_operator(&d2, |a, b| a.binary(BinOp::Add, b))?;
        Ok(Mapped::Die(sum.try_clip(None, Some(&Value::from(4)))?))
    }

    /// The typed `step_d2`.
    fn typed_step_d2(&x: &i64) -> Mapped<i64> {
        Mapped::Die((d(2) + x).clip(None, Some(&4)))
    }

    #[test]
    fn map_with_a_repeat_count() {
        // `d(6).map(lambda x: x + 1 if x < 6 else x, repeat=2)`
        let mut cache = TransitionCache::new(climb, vec![Quantity::ONE]);
        let result: Result<_> = map_repeat(&mut cache, &ints(1..=6), Some(2));
        assert_eq!(
            items(&result.unwrap()),
            expected(&[(3, 1), (4, 1), (5, 1), (6, 3)].map(|(o, q)| (Value::from(o), q)))
        );
        // `range(repeat)` raises for a repeat that is not an int.
        let result: Result<_> = map_repeat(&mut cache, &ints(1..=6), None);
        assert!(matches!(result, Err(Error::WrongType(_))));
        // The same on typed states.
        let typed = d(6).map_repeat(|&x| Mapped::Outcome(if x < 6 { x + 1 } else { x }), 2);
        assert_eq!(typed.unwrap(), weights(&[(3, 1), (4, 1), (5, 1), (6, 3)]));
    }

    #[test]
    fn map_to_absorption() {
        // `icepool.map(lambda x: x + 1 if x < 6 else x, d(6), repeat='inf')`
        let mut cache = TransitionCache::new(climb, vec![Quantity::ONE]);
        let result: Result<_> = absorbing_markov_chain(&mut cache, &ints(1..=6));
        assert_eq!(items(&result.unwrap().0), expected(&[(Value::from(6), 1)]));
        // `icepool.mean_time_to_absorb(lambda x: (x + d(2)).clip(None, 4), 0)`
        let mut cache = TransitionCache::new(step_d2, vec![Quantity::ONE]);
        let (absorbed, mean_time) =
            absorbing_markov_chain::<_, _, Error>(&mut cache, &ints([0])).unwrap();
        assert_eq!(items(&absorbed), expected(&[(Value::from(4), 1)]));
        assert_eq!(mean_time.unwrap().to_string(), "23/8");
        // The same on typed states.
        let start = Die::single(0);
        assert_eq!(
            start.map_to_absorption(typed_step_d2).unwrap(),
            Die::single(4)
        );
        assert_eq!(
            start
                .mean_time_to_absorb(typed_step_d2)
                .unwrap()
                .to_string(),
            "23/8"
        );
    }

    #[test]
    fn map_and_time_counts_the_steps() {
        // `icepool.map_and_time(lambda x: (x + d(2)).clip(None, 4), 0, repeat=10)`
        let mut cache = TransitionCache::new(step_d2, vec![Quantity::ONE]);
        let result: Result<_> = map_and_time(&mut cache, &ints([0]), Some(10));
        let pair = |s: i64, t: i64| Value::tuple(vec![Value::from(s), Value::from(t)]);
        let want = [(pair(4, 2), 4), (pair(4, 3), 10), (pair(4, 4), 2)];
        assert_eq!(items(&result.unwrap()), expected(&want));
        // The same on typed states.
        let typed = Die::single(0).map_and_time(typed_step_d2, 10).unwrap();
        assert_eq!(typed, weights(&[((4, 2), 4), ((4, 3), 10), ((4, 4), 2)]));
    }

    #[test]
    fn typed_break_reroll_and_restart() {
        // `d(6).map(lambda x: icepool.Break() if x == 3 else x + 1 if x < 6 else x, repeat=3)`
        let stop_at_3 = |&x: &i64| match x {
            3 => Mapped::Break(None),
            x => Mapped::Outcome(if x < 6 { x + 1 } else { x }),
        };
        assert_eq!(
            d(6).map_repeat(stop_at_3, 3).unwrap(),
            weights(&[(3, 3), (6, 3)])
        );
        // `d(6).map(lambda x: icepool.Restart if x == 1 else min(x + 1, 6), repeat=2)`
        let restart_1 = |&x: &i64| {
            if x == 1 {
                Mapped::Restart
            } else {
                Mapped::Outcome((x + 1).min(6))
            }
        };
        assert_eq!(
            d(6).map_repeat(restart_1, 2).unwrap(),
            weights(&[(4, 1), (5, 1), (6, 3)])
        );
        // `d(6).map(lambda x: icepool.Reroll if x == 2 else min(x + 2, 6), repeat=2)`
        let reroll_2 = |&x: &i64| {
            if x == 2 {
                Mapped::Reroll
            } else {
                Mapped::Outcome((x + 2).min(6))
            }
        };
        assert_eq!(
            d(6).map_repeat(reroll_2, 2).unwrap(),
            weights(&[(2, 1), (5, 1), (6, 4)])
        );
        // `d(4).map(lambda x: icepool.Break(d(2)) if x == 3 else min(x + 1, 4), repeat='inf')`
        let break_to_d2 = |&x: &i64| match x {
            3 => Mapped::Break(Some(Target::Die(d(2)))),
            x => Mapped::Outcome((x + 1).min(4)),
        };
        assert_eq!(
            d(4).map_to_absorption(break_to_d2).unwrap(),
            weights(&[(1, 3), (2, 3), (4, 2)])
        );
        // `icepool.map_and_time(lambda x: icepool.Break(d(2) + 10) if x == 2 else (x + d(2)).clip(None, 4), 0,
        // repeat=5)`
        let break_high = |&x: &i64| match x {
            2 => Mapped::Break(Some(Target::Die(d(2) + 10))),
            x => typed_step_d2(&x),
        };
        let timed = Die::single(0).map_and_time(break_high, 5).unwrap();
        let want = [
            ((4, 3), 2),
            ((11, 2), 2),
            ((11, 3), 1),
            ((12, 2), 2),
            ((12, 3), 1),
        ];
        assert_eq!(timed, weights(&want));
    }

    #[test]
    fn typed_absorption_errors_as_python_raises() {
        // `icepool.mean_time_to_absorb(lambda x: x if x >= 3 else x + d(2), 0)`
        let walk = |&x: &i64| {
            if x >= 3 {
                Mapped::Outcome(x)
            } else {
                Mapped::Die(d(2) + x)
            }
        };
        assert_eq!(
            Die::single(0)
                .mean_time_to_absorb(walk)
                .unwrap()
                .to_string(),
            "9/4"
        );
        // ... with `Restart if x == 1`: NotImplementedError.
        let restart = |&x: &i64| if x == 1 { Mapped::Restart } else { walk(&x) };
        assert!(matches!(
            Die::single(0).mean_time_to_absorb(restart),
            Err(Error::NotImplementedUpstream(_))
        ));
        // `icepool.map(lambda x: x - 1 if x > 0 else x + d(2) - 1, 2, repeat='inf')`: never absorbed.
        let forever = |&x: &i64| {
            if x > 0 {
                Mapped::Outcome(x - 1)
            } else {
                Mapped::Die(d(2) + (x - 1))
            }
        };
        assert!(matches!(
            Die::single(2).map_to_absorption(forever),
            Err(Error::InvalidArgument(_))
        ));
    }
}
