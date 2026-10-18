//! Port of map_tools/core_impl.py `TransitionCache`: a transition function for `map(repeat)`, `map_and_time` and
//! the absorbing Markov chain, cached per state.
//!
//! The transition function belongs to the caller (a Rust closure, a Python callable, a catalogue function, count
//! mode's `step`): the core calls it as `transition(state, i)`, where `i` indexes the joint outcomes of the extra
//! arguments in `iter_cartesian_product` order, so the caller keeps the outcome objects (and unpacks the state
//! for `star`) and the core only sees their quantities. It is called lazily, exactly where Python calls it:
//! `is_self_loop` stops at the first transition that leaves the state, and a state is stepped only when the
//! process reaches it.
//!
//! States are looked up in hash maps, as Python looks them up in dicts (`hash` + `==`, the first key spelling
//! kept).

use rustc_hash::FxHashMap;

use super::state::{MapState, Transition};
use crate::creation_args::DieArg;
use crate::die_map::{Mapped, Target};
use crate::error::{Error, Result};
use crate::outcome::ErrInto;
use crate::{Die, Quantity};

/// Helper class for caching a transition function for `map(repeat)`.
///
/// For language bindings, which wrap their callable as `transition(state, i)` and hand the cache to
/// [`map_repeat`](crate::value::map_repeat) & co.:
///
/// ```
/// use icepool_core::value::{map_repeat, TransitionCache};
/// use icepool_core::{d, Die, Error, Mapped, Quantity};
/// // `d(3).map(lambda x: min(x + 1, 3), repeat=1)`
/// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
/// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
/// let result = map_repeat(&mut cache, &d(3), Some(1)).unwrap();
/// assert_eq!(result, Die::from_weights([(2, 1u32), (3, 2)]));
/// ```
pub struct TransitionCache<S: MapState, F> {
    transition: F,
    /// The joint quantities of the extra arguments, one per index the transition function takes.
    extra_quantities: Vec<Quantity>,
    cache: FxHashMap<S, Die<S::Transition>>,
    /// state -> whether it is a self-loop (`BREAK`) or not (`DEFAULT`)
    self_loop_cache: FxHashMap<S, bool>,
}

/// The transition is a closure: only the states cached so far print.
impl<S: MapState, F> std::fmt::Debug for TransitionCache<S, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransitionCache")
            .field("cached_states", &self.cache.len())
            .finish_non_exhaustive()
    }
}

impl<S, F, E> TransitionCache<S, F>
where
    S: MapState,
    F: FnMut(&S, usize) -> std::result::Result<Mapped<S>, E>,
    E: From<Error>,
{
    /// A cache over `transition(state, i)`, `i` running over `extra_quantities` (one `1` for no extra arguments).
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{Die, Error, Mapped, Quantity};
    /// // `d6.map(lambda x, y: x + y, d2)` passes one extra argument: two joint outcomes, `i` = 0 or 1.
    /// let ys = [1, 2];
    /// let add = |&x: &i64, i: usize| Ok::<_, Error>(Mapped::Outcome(x + ys[i]));
    /// let mut cache = TransitionCache::new(add, vec![Quantity::ONE; 2]);
    /// assert_eq!(cache.step_state(&10).unwrap(), Die::from_iter([Transition::Default(11), Transition::Default(12)]));
    /// ```
    pub fn new(transition: F, extra_quantities: Vec<Quantity>) -> Self {
        TransitionCache {
            transition,
            extra_quantities,
            cache: FxHashMap::default(),
            self_loop_cache: FxHashMap::default(),
        }
    }

    /// Whether the state is a self-loop (Python returns `TransitionType.BREAK` for one, `DEFAULT` otherwise).
    ///
    /// ```
    /// use icepool_core::value::TransitionCache;
    /// use icepool_core::{Error, Mapped, Quantity};
    /// // Python (`TransitionCache(lambda x: min(x + 1, 3), star=False)`): `is_self_loop(3)` is BREAK, `(2)` DEFAULT.
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// assert_eq!(cache.is_self_loop(&3), Ok(true));
    /// assert_eq!(cache.is_self_loop(&2), Ok(false));
    /// ```
    pub fn is_self_loop(&mut self, curr_state: &S) -> std::result::Result<bool, E> {
        if let Some(&t) = self.self_loop_cache.get(curr_state) {
            return Ok(t);
        }
        curr_state.check_state()?;
        let mut result = true;
        for i in 0..self.extra_quantities.len() {
            let next_state = match (self.transition)(curr_state, i)? {
                // Ignored.
                Mapped::Reroll => continue,
                // Might restart, therefore not a self-loop.
                Mapped::Restart => {
                    result = false;
                    break;
                }
                // Break to the current outcome.
                Mapped::Break(None) => continue,
                // Unwrap Break.
                Mapped::Break(Some(next)) => next,
                Mapped::Outcome(next) => Target::Outcome(next),
                Mapped::Die(next) => Target::Die(next),
            };
            let stays = match next_state {
                // `next_state.probability(curr_state) == 1`: `Fraction(quantity, denominator)`.
                Target::Die(die) => {
                    if die.is_empty() {
                        return Err(Error::DivisionByZero("Fraction(0, 0)".into()).into());
                    }
                    let quantity = die
                        .items()
                        .iter()
                        .find(|(o, _)| o == curr_state)
                        .map_or(Quantity::ZERO, |(_, q)| q.clone());
                    quantity == die.denominator()
                }
                Target::Outcome(next) => next == *curr_state,
                // `Reroll == curr_state`.
                Target::Reroll => false,
            };
            if !stays {
                result = false;
                break;
            }
        }
        self.self_loop_cache.insert(curr_state.clone(), result);
        Ok(result)
    }

    /// Returns a die whose outcomes are `(transition_type, outcome)` according to whether the states are
    /// self-loops.
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{d, Die, Error, Mapped, Quantity};
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// // Python: `{(DEFAULT, 1): 1, (DEFAULT, 2): 1, (BREAK, 3): 1}`.
    /// let expected = Die::from_iter([Transition::Default(1), Transition::Default(2), Transition::Break(3)]);
    /// assert_eq!(cache.self_loop_die(&d(3)).unwrap(), expected);
    /// ```
    pub fn self_loop_die(&mut self, die: &Die<S>) -> std::result::Result<Die<S::Transition>, E> {
        self.self_loop_die_with(die, S::transition)
    }

    /// As `self_loop_die`, but also appends a zero time.
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{d, Die, Error, Mapped, Quantity};
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// // Python: `{(DEFAULT, 1, 0): 1, (DEFAULT, 2, 0): 1, (BREAK, 3, 0): 1}`.
    /// let expected =
    ///     Die::from_iter([(Transition::Default(1), 0), (Transition::Default(2), 0), (Transition::Break(3), 0)]);
    /// assert_eq!(cache.self_loop_die_with_zero_time(&d(3)).unwrap(), expected);
    /// ```
    pub fn self_loop_die_with_zero_time(
        &mut self,
        die: &Die<S>,
    ) -> std::result::Result<Die<S::Timed>, E> {
        self.self_loop_die_with(die, |t| S::timed(t, 0))
    }

    fn self_loop_die_with<T: crate::OutcomeOrder>(
        &mut self,
        die: &Die<S>,
        make: impl Fn(Transition<S>) -> T,
    ) -> std::result::Result<Die<T>, E> {
        let mut outcomes = Vec::with_capacity(die.len());
        let mut quantities = Vec::with_capacity(die.len());
        for (o, q) in die.items() {
            let transition = Transition::new(self.is_self_loop(o)?, o.clone());
            outcomes.push(DieArg::Outcome(make(transition)));
            quantities.push(q.clone());
        }
        Ok(Die::new(outcomes, quantities)?)
    }

    /// Computes and caches a single step of the transition function for a single state: a die whose outcomes are
    /// `(transition_type, next_state)`.
    ///
    /// This only works if the input is the same as the output type.
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{Die, Error, Mapped, Quantity};
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// // Python: `step_state(1)` is `{(DEFAULT, 2): 1}`; `step_state(3)` is `{(BREAK, 3): 1}`, 3 leading only to
    /// // itself.
    /// assert_eq!(cache.step_state(&1).unwrap(), Die::single(Transition::Default(2)));
    /// assert_eq!(cache.step_state(&3).unwrap(), Die::single(Transition::Break(3)));
    /// ```
    pub fn step_state(&mut self, curr_state: &S) -> std::result::Result<Die<S::Transition>, E> {
        if let Some(die) = self.cache.get(curr_state) {
            return Ok(die.clone());
        }
        curr_state.check_state()?;
        let mut next_states = Vec::with_capacity(self.extra_quantities.len());
        let mut next_quantities = Vec::with_capacity(self.extra_quantities.len());
        for i in 0..self.extra_quantities.len() {
            let next = match (self.transition)(curr_state, i)? {
                // Pruned immediately.
                Mapped::Reroll => continue,
                // Will prune at end.
                Mapped::Restart => DieArg::Outcome(S::transition(Transition::Restart)),
                Mapped::Break(None) => {
                    DieArg::Outcome(S::transition(Transition::Break(curr_state.clone())))
                }
                Mapped::Break(Some(Target::Outcome(next))) => {
                    DieArg::Outcome(S::transition(Transition::Break(next)))
                }
                // `(TransitionType.BREAK, die)` is a tuple holding a die, which `Die()` tupleizes: one break per
                // outcome of the die.
                Mapped::Break(Some(Target::Die(die))) => DieArg::Die(relabel(&die, |o| {
                    S::transition(Transition::Break(o.clone()))
                })?),
                // `(TransitionType.BREAK, Reroll)` is a tuple holding `Reroll`, which `Die()` cannot expand (see
                // DIVERGENCES.md).
                Mapped::Break(Some(Target::Reroll)) => {
                    return Err(Error::MissingAttribute(
                        "'RerollType' object has no attribute 'values'".into(),
                    )
                    .into())
                }
                // If the next state is a die, we need to conditionally break. This is why we keep the result as a
                // single die rather than separate non-break and break dice.
                Mapped::Die(die) => DieArg::Die(self.self_loop_die(&die)?),
                Mapped::Outcome(next) => {
                    let is_self_loop = self.is_self_loop(&next)?;
                    DieArg::Outcome(S::transition(Transition::new(is_self_loop, next)))
                }
            };
            next_states.push(next);
            next_quantities.push(self.extra_quantities[i].clone());
        }
        let result = Die::new(next_states, next_quantities)?;
        self.cache.insert(curr_state.clone(), result.clone());
        Ok(result)
    }

    /// Advances the `(transition_type, state)` die by one time step: the `DEFAULT` states take a step, the others
    /// stay as they are.
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{d, Die, Error, Mapped, Quantity};
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// let start = cache.self_loop_die(&d(3)).unwrap();
    /// // Python: `{(DEFAULT, 2): 1, (BREAK, 3): 2}`.
    /// let expected = Die::from_weights([(Transition::Default(2), 1u32), (Transition::Break(3), 2)]);
    /// assert_eq!(cache.step_transition_die(&start).unwrap(), expected);
    /// ```
    pub fn step_transition_die(
        &mut self,
        curr_die: &Die<S::Transition>,
    ) -> std::result::Result<Die<S::Transition>, E> {
        let mut next_states = Vec::with_capacity(curr_die.len());
        let mut quantities = Vec::with_capacity(curr_die.len());
        for (outcome, quantity) in curr_die.items() {
            crate::interrupt::poll()?;
            next_states.push(match S::read_transition(outcome) {
                Transition::Default(state) => DieArg::Die(self.step_state(state)?),
                _ => DieArg::Outcome(outcome.clone()),
            });
            quantities.push(quantity.clone());
        }
        Ok(Die::new(next_states, quantities)?)
    }

    /// As `step_transition_die`, but keeps track of the break time in the last element of
    /// `(transition_type, state, time)`.
    ///
    /// ```
    /// use icepool_core::value::{Transition, TransitionCache};
    /// use icepool_core::{d, Die, Error, Mapped, Quantity};
    /// let step = |&x: &i64, _: usize| Ok::<_, Error>(Mapped::Outcome((x + 1).min(3)));
    /// let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
    /// let start = cache.self_loop_die_with_zero_time(&d(3)).unwrap();
    /// // Python: `{(DEFAULT, 2, 1): 1, (BREAK, 3, 0): 1, (BREAK, 3, 1): 1}`: 3 broke at time 0; 1 and 2 took a step.
    /// let expected =
    ///     Die::from_iter([(Transition::Default(2), 1), (Transition::Break(3), 0), (Transition::Break(3), 1)]);
    /// assert_eq!(cache.step_transition_die_with_time(&start).unwrap(), expected);
    /// ```
    pub fn step_transition_die_with_time(
        &mut self,
        curr_die: &Die<S::Timed>,
    ) -> std::result::Result<Die<S::Timed>, E> {
        let mut next_states = Vec::with_capacity(curr_die.len());
        let mut quantities = Vec::with_capacity(curr_die.len());
        for (outcome, quantity) in curr_die.items() {
            crate::interrupt::poll()?;
            next_states.push(match S::read_timed(outcome) {
                // `self.step_state(curr_state) + (time + 1,)`: each outcome gets the time appended.
                (Transition::Default(state), time) => {
                    let step = self.step_state(state)?;
                    DieArg::Die(relabel(&step, |t| {
                        S::timed(owned(S::read_transition(t)), time + 1)
                    })?)
                }
                _ => DieArg::Outcome(outcome.clone()),
            });
            quantities.push(quantity.clone());
        }
        Ok(Die::new(next_states, quantities)?)
    }
}

/// The die with each outcome replaced by `f` of it, quantities (zero ones too) and order unchanged up to the new
/// outcomes' sort: `f` must keep distinct outcomes distinct, as appending to a tuple does.
pub(crate) fn relabel<A: crate::OutcomeOrder, B: crate::OutcomeOrder>(
    die: &Die<A>,
    f: impl Fn(&A) -> B,
) -> Result<Die<B>> {
    let items = die.items().iter().map(|(o, q)| (f(o), q.clone())).collect();
    Die::from_dict(items).err_into()
}

/// An owned copy of a borrowed transition.
pub(crate) fn owned<S: Clone>(transition: Transition<&S>) -> Transition<S> {
    match transition {
        Transition::Default(s) => Transition::Default(s.clone()),
        Transition::Break(s) => Transition::Break(s.clone()),
        Transition::Restart => Transition::Restart,
    }
}
