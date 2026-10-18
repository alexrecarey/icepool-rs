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
///
/// ```
/// use icepool_core::value::Transition;
/// assert_eq!(Transition::new(true, 3), Transition::Break(3));
/// assert!(Transition::Default(9) < Transition::Break(1)); // by type first, as Python's tuples
/// assert_eq!(Transition::<i64>::Restart.state(), None);
/// ```
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
    ///
    /// ```
    /// use icepool_core::value::Transition;
    /// assert_eq!(Transition::new(false, 'a'), Transition::Default('a'));
    /// assert_eq!(Transition::new(true, 'a'), Transition::Break('a'));
    /// ```
    pub fn new(is_break: bool, state: S) -> Self {
        if is_break {
            Transition::Break(state)
        } else {
            Transition::Default(state)
        }
    }

    /// The state, unless this is a `Restart`.
    ///
    /// ```
    /// use icepool_core::value::Transition;
    /// assert_eq!(Transition::Break(4).state(), Some(&4));
    /// assert_eq!(Transition::<i64>::Restart.state(), None);
    /// ```
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
///
/// ```
/// use icepool_core::value::{MapState, Transition};
/// // A typed state's transition dice have `Transition` outcomes.
/// assert_eq!(<i64 as MapState>::transition(Transition::Break(3)), Transition::Break(3));
/// assert_eq!(i64::read_transition(&Transition::Default(2)), Transition::Default(&2));
/// assert_eq!(i64::with_time(Transition::Break(3), 2).unwrap(), (3, 2));
/// ```
pub trait MapState: OutcomeOrder {
    /// `(transition_type, state)`.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::Value;
    /// let typed: <i64 as MapState>::Transition = Transition::Default(5);
    /// assert_eq!(typed, Transition::Default(5));
    /// // Python's `(TransitionType.BREAK, 5)`, an `IntEnum` in a tuple: `(1, 5)`.
    /// assert_eq!(Value::transition(Transition::Break(Value::from(5))).to_string(), "(1, 5)");
    /// ```
    type Transition: OutcomeOrder;
    /// `(transition_type, state, time)`: `map_and_time`'s transition dice.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// let timed: <i64 as MapState>::Timed = i64::timed(Transition::Break(5), 2);
    /// assert_eq!(timed, (Transition::Break(5), 2));
    /// ```
    type Timed: OutcomeOrder;
    /// `(state, time)`: `map_and_time`'s result.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// let with_time: <i64 as MapState>::WithTime = i64::with_time(Transition::Break(5), 2).unwrap();
    /// assert_eq!(with_time, (5, 2));
    /// ```
    type WithTime: OutcomeOrder;

    /// The outcome for a transition.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::Value;
    /// // Python: `(TransitionType.RESTART, None)`.
    /// assert_eq!(Value::transition(Transition::Restart).to_string(), "(2, None)");
    /// assert_eq!(i64::transition(Transition::Restart), Transition::Restart);
    /// ```
    fn transition(transition: Transition<Self>) -> Self::Transition;
    /// The transition an outcome stands for.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::Value;
    /// let outcome = Value::tuple(vec![Value::from(0), Value::str("a")]); // `(DEFAULT, 'a')`
    /// assert_eq!(Value::read_transition(&outcome), Transition::Default(&Value::str("a")));
    /// ```
    fn read_transition(outcome: &Self::Transition) -> Transition<&Self>;
    /// The outcome for a transition reached at `time`.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::Value;
    /// // Python: `(TransitionType.DEFAULT, 'a', 3)`.
    /// let expected = Value::tuple(vec![Value::from(0), Value::str("a"), Value::from(3)]);
    /// assert_eq!(Value::timed(Transition::Default(Value::str("a")), 3), expected);
    /// ```
    fn timed(transition: Transition<Self>, time: u64) -> Self::Timed;
    /// The transition and time an outcome stands for.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::Value;
    /// let outcome = Value::timed(Transition::Break(Value::from(7)), 3);
    /// assert_eq!(Value::read_timed(&outcome), (Transition::Break(&Value::from(7)), 3));
    /// assert_eq!(i64::read_timed(&(Transition::Default(7), 1)), (Transition::Default(&7), 1));
    /// ```
    fn read_timed(outcome: &Self::Timed) -> (Transition<&Self>, u64);
    /// `map_and_time`'s outcome for the transition reached at `time`: `(state, time)`. Python's `Restart` has
    /// the state `None`, which only `Value` can spell.
    ///
    /// ```
    /// use icepool_core::value::{MapState, Transition};
    /// use icepool_core::{Error, Value};
    /// // Python: a `Restart` reached at time 2 is `(None, 2)`.
    /// assert_eq!(Value::with_time(Transition::Restart, 2).unwrap().to_string(), "(None, 2)");
    /// assert!(matches!(i64::with_time(Transition::Restart, 2), Err(Error::Unported(_))));
    /// ```
    fn with_time(transition: Transition<Self>, time: u64) -> Result<Self::WithTime>;

    /// Refuses a state the port cannot treat as Python would (`Value`: one holding a NaN, which Python finds
    /// again by identity). Every state is checked before the transition function is called on it.
    ///
    /// ```
    /// use icepool_core::value::MapState;
    /// use icepool_core::Value;
    /// assert!(3i64.check_state().is_ok());
    /// assert!(Value::Float(1.5).check_state().is_ok());
    /// // A NaN state is refused: Python finds the same NaN object again by identity, which the port cannot.
    /// assert!(Value::Float(f64::NAN).check_state().is_err());
    /// ```
    fn check_state(&self) -> Result<()> {
        Ok(())
    }

    /// Whether two equal states are interchangeable: the absorbing Markov chain refuses equal states spelled
    /// differently (`Value`: `1`, `True`, `1.0`), whose result would depend on Python's set order.
    ///
    /// ```
    /// use icepool_core::value::MapState;
    /// use icepool_core::Value;
    /// assert!(3i64.same_spelling(&3));
    /// // `1 == True`, but they are not interchangeable as states.
    /// assert!(!Value::from(1).same_spelling(&Value::Bool(true)));
    /// assert!(Value::from(1).same_spelling(&Value::from(1)));
    /// ```
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
