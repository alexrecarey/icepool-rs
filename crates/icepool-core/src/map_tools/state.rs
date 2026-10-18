//! The states of a repeated map and the outcomes of its transition dice (map_tools/common.py `TransitionType`).
//!
//! Python's transition dice have tuple outcomes `(transition_type, state)` (and `(transition_type, state, time)`
//! for `map_and_time`), which sort by type, then state. A typed state gets the same order from [`Transition`]'s
//! derived `Ord`; `Value` spells them as Python's tuples (value/map_state.rs), so every intermediate die, its
//! order and its quantities are Python's.

use crate::error::{Error, Result};
use crate::outcome::{Outcome, OutcomeOrder};

/// `TransitionType` with the state it applies to. `TransitionType` is an `IntEnum` (`DEFAULT = 0`, `BREAK = 1`,
/// `RESTART = 2`), so the variants are in that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Transition<S> {
    /// The process goes on from this state.
    Default(S),
    /// The process stops at this state: a `Break`, or a state that only leads to itself.
    Break(S),
    /// `(RESTART, None)`: the whole process restarts (pruned at the end).
    Restart,
}

impl<S> Transition<S> {
    /// `Default(state)` or `Break(state)` by `is_break`.
    pub fn new(is_break: bool, state: S) -> Self {
        if is_break {
            Transition::Break(state)
        } else {
            Transition::Default(state)
        }
    }

    /// The state, unless this is a `Restart`.
    pub fn state(&self) -> Option<&S> {
        match self {
            Transition::Default(s) | Transition::Break(s) => Some(s),
            Transition::Restart => None,
        }
    }
}

/// A state of `map(repeat)`, `map_and_time` or the absorbing Markov chain: an outcome, plus the outcome types of
/// the transition dice built over it.
///
/// Implemented for every [`Outcome`] (with [`Transition`] and tuples) and for `Value` (with Python's tuples).
pub trait MapState: OutcomeOrder {
    /// `(transition_type, state)`.
    type Transition: OutcomeOrder;
    /// `(transition_type, state, time)`: `map_and_time`'s transition dice.
    type Timed: OutcomeOrder;
    /// `(state, time)`: `map_and_time`'s result.
    type WithTime: OutcomeOrder;

    /// The outcome for a transition.
    fn transition(transition: Transition<Self>) -> Self::Transition;
    /// The transition an outcome stands for.
    fn read_transition(outcome: &Self::Transition) -> Transition<&Self>;
    /// The outcome for a transition reached at `time`.
    fn timed(transition: Transition<Self>, time: u64) -> Self::Timed;
    /// The transition and time an outcome stands for.
    fn read_timed(outcome: &Self::Timed) -> (Transition<&Self>, u64);
    /// `map_and_time`'s outcome for the transition reached at `time`: `(state, time)`. Python's `Restart` has
    /// the state `None`, which only `Value` can spell.
    fn with_time(transition: Transition<Self>, time: u64) -> Result<Self::WithTime>;

    /// Refuses a state the port cannot treat as Python would (`Value`: one holding a NaN, which Python finds
    /// again by identity). Every state is checked before the transition function is called on it.
    fn check_state(&self) -> Result<()> {
        Ok(())
    }

    /// Whether two equal states are interchangeable: the absorbing Markov chain refuses equal states spelled
    /// differently (`Value`: `1`, `True`, `1.0`), whose result would depend on Python's set order.
    fn same_spelling(&self, _other: &Self) -> bool {
        true
    }
}

impl<S: Outcome> MapState for S {
    type Transition = Transition<S>;
    type Timed = (Transition<S>, u64);
    type WithTime = (S, u64);

    fn transition(transition: Transition<S>) -> Transition<S> {
        transition
    }

    fn read_transition(outcome: &Transition<S>) -> Transition<&S> {
        match outcome {
            Transition::Default(s) => Transition::Default(s),
            Transition::Break(s) => Transition::Break(s),
            Transition::Restart => Transition::Restart,
        }
    }

    fn timed(transition: Transition<S>, time: u64) -> (Transition<S>, u64) {
        (transition, time)
    }

    fn read_timed((transition, time): &(Transition<S>, u64)) -> (Transition<&S>, u64) {
        (S::read_transition(transition), *time)
    }

    fn with_time(transition: Transition<S>, time: u64) -> Result<(S, u64)> {
        match transition {
            Transition::Default(s) | Transition::Break(s) => Ok((s, time)),
            Transition::Restart => Err(Error::Unported(
                "Restart in map_and_time: Python's (None, time) outcome has no typed form".into(),
            )),
        }
    }
}
