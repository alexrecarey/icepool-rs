//! Port of map_tools/core_impl.py `TransitionCache` (and common.py `TransitionType`): a transition function for
//! `map(repeat)`, `map_and_time` and the absorbing Markov chain, cached per state.
//!
//! The transition function belongs to the facade (a Python callable, a catalogue function, count mode's `step`):
//! the core calls it as `transition(state, i)`, where `i` indexes the joint outcomes of the extra arguments in
//! `iter_cartesian_product` order, so the facade keeps the outcome objects (and unpacks the state for `star`) and
//! the core only sees their quantities. It is called lazily, exactly where Python calls it: `is_self_loop` stops
//! at the first transition that leaves the state, and a state is stepped only when the process reaches it.
//!
//! States are `Value`s. Python looks states up in dicts (`hash` + `==`, the first key spelling kept), which is
//! what an `FxHashMap<Value, _>` does too.

use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::expand::tupleize_dice;
use crate::value::BinOp;
use crate::{Die, DieArg, Quantity, Value};

/// `TransitionType` is an `IntEnum`, so its members sort and hash as these ints.
pub const DEFAULT: i64 = 0;
pub const BREAK: i64 = 1;
// Reroll gets pruned immediately, so doesn't get a transition type.
pub const RESTART: i64 = 2;

pub(crate) fn int(i: i64) -> Value {
    Value::from(dashu_int::IBig::from(i))
}

/// Where a transition goes: one outcome or a die of them.
pub enum Target {
    Outcome(Value),
    Die(Die<Value>),
    /// `Reroll` (`Go(Reroll)` is the same as `Next::Reroll`). `Break(Reroll)`: Python builds `(TransitionType.BREAK, Reroll)`, which `Die()` cannot expand (see
    /// DIVERGENCES.md: a tuple holding `Reroll`).
    Reroll,
}

/// What the transition function returned.
pub enum Next {
    /// `icepool.Reroll`: this transition is dropped.
    Reroll,
    /// `icepool.Restart`: the whole process restarts (conditioning against this sequence of transitions).
    Restart,
    /// `Break(outcome)`: an early exit to `outcome`; `None` (`Break()`) is the current state.
    Break(Option<Target>),
    /// Any other result: the next state (or a die of next states).
    Go(Target),
}

/// The `(transition_type, state)` tuple of a transition die.
pub(crate) fn transition(transition_type: i64, state: Value) -> Value {
    Value::tuple(vec![int(transition_type), state])
}

/// The parts of a transition die's outcome: `(transition_type, state, ...)`.
pub(crate) fn parts(outcome: &Value) -> &[Value] {
    match outcome {
        Value::Tuple(t) => t,
        _ => unreachable!("transition outcomes are (transition_type, state, ...) tuples"),
    }
}

/// Whether a NaN is in `v`. Python's dicts and tuple `==` short-circuit on object *identity*, so the same NaN
/// object found again (a state carried over a step, `Break()` to the current state) is the same key there, while
/// a new NaN object is not: the port cannot tell which is which, so it does not guess.
pub(crate) fn check_nan(v: &Value) -> Result<()> {
    fn has_nan(v: &Value) -> bool {
        match v {
            Value::Float(f) => f.is_nan(),
            Value::Complex(re, im) => re.is_nan() || im.is_nan(),
            Value::Tuple(items) => items.iter().any(has_nan),
            _ => false,
        }
    }
    if has_nan(v) {
        return Err(Error::Unsupported(
            "NaN in map(repeat) states (Python finds the same NaN object again by identity)".into(),
        ));
    }
    Ok(())
}

/// Helper class for caching a transition function for `map(repeat)`.
pub struct TransitionCache<F> {
    transition: F,
    /// The joint quantities of the extra arguments, one per index the transition function takes.
    extra_quantities: Vec<Quantity>,
    cache: FxHashMap<Value, Die<Value>>,
    /// state -> DEFAULT or BREAK
    self_loop_cache: FxHashMap<Value, i64>,
}

impl<F, E> TransitionCache<F>
where
    F: FnMut(&Value, usize) -> std::result::Result<Next, E>,
    E: From<Error>,
{
    pub fn new(transition: F, extra_quantities: Vec<Quantity>) -> Self {
        TransitionCache {
            transition,
            extra_quantities,
            cache: FxHashMap::default(),
            self_loop_cache: FxHashMap::default(),
        }
    }

    /// Returns `BREAK` if the state is a self-loop, or `DEFAULT` otherwise.
    pub fn is_self_loop(&mut self, curr_state: &Value) -> std::result::Result<i64, E> {
        if let Some(&t) = self.self_loop_cache.get(curr_state) {
            return Ok(t);
        }
        check_nan(curr_state)?;
        let mut result = BREAK;
        for i in 0..self.extra_quantities.len() {
            let next_state = match (self.transition)(curr_state, i)? {
                // Ignored.
                Next::Reroll | Next::Go(Target::Reroll) => continue,
                // Might restart, therefore not a self-loop.
                Next::Restart => {
                    result = DEFAULT;
                    break;
                }
                // Break to the current outcome.
                Next::Break(None) => continue,
                // Unwrap Break.
                Next::Break(Some(next)) | Next::Go(next) => next,
            };
            let stays = match next_state {
                // `next_state.probability(curr_state) == 1`: `Fraction(quantity, denominator)`.
                Target::Die(die) => {
                    if die.is_empty() {
                        return Err(Error::ZeroDivision("Fraction(0, 0)".into()).into());
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
                result = DEFAULT;
                break;
            }
        }
        self.self_loop_cache.insert(curr_state.clone(), result);
        Ok(result)
    }

    /// Returns a die whose outcomes are `(transition_type, outcome)` according to whether the states are
    /// self-loops.
    pub fn self_loop_die(&mut self, die: &Die<Value>) -> std::result::Result<Die<Value>, E> {
        self.self_loop_die_with(die, transition)
    }

    /// As `self_loop_die`, but also appends a zero time.
    pub fn self_loop_die_with_zero_time(
        &mut self,
        die: &Die<Value>,
    ) -> std::result::Result<Die<Value>, E> {
        self.self_loop_die_with(die, |t, o| Value::tuple(vec![int(t), o, int(0)]))
    }

    fn self_loop_die_with(
        &mut self,
        die: &Die<Value>,
        make: impl Fn(i64, Value) -> Value,
    ) -> std::result::Result<Die<Value>, E> {
        let mut outcomes = Vec::with_capacity(die.len());
        let mut quantities = Vec::with_capacity(die.len());
        for (o, q) in die.items() {
            outcomes.push(DieArg::Outcome(make(self.is_self_loop(o)?, o.clone())));
            quantities.push(q.clone());
        }
        Ok(Die::new(outcomes, quantities)?)
    }

    /// Computes and caches a single step of the transition function for a single state: a die whose outcomes are
    /// `(transition_type, next_state)`.
    ///
    /// This only works if the input is the same as the output type.
    pub fn step_state(&mut self, curr_state: &Value) -> std::result::Result<Die<Value>, E> {
        if let Some(die) = self.cache.get(curr_state) {
            return Ok(die.clone());
        }
        check_nan(curr_state)?;
        let mut next_states = Vec::with_capacity(self.extra_quantities.len());
        let mut next_quantities = Vec::with_capacity(self.extra_quantities.len());
        for i in 0..self.extra_quantities.len() {
            let next = match (self.transition)(curr_state, i)? {
                // Pruned immediately.
                Next::Reroll | Next::Go(Target::Reroll) => continue,
                // Will prune at end.
                Next::Restart => DieArg::Outcome(transition(RESTART, Value::None)),
                Next::Break(None) => DieArg::Outcome(transition(BREAK, curr_state.clone())),
                Next::Break(Some(Target::Outcome(next))) => {
                    DieArg::Outcome(transition(BREAK, next))
                }
                // `(TransitionType.BREAK, die)` is a tuple holding a die, which `Die()` tupleizes.
                Next::Break(Some(Target::Die(die))) => DieArg::Die(tupleize_dice(&[
                    vec![(int(BREAK), Quantity::ONE)],
                    die.items().to_vec(),
                ])?),
                Next::Break(Some(Target::Reroll)) => {
                    return Err(Error::Type(
                        "AttributeError: 'RerollType' object has no attribute 'values'".into(),
                    )
                    .into())
                }
                // If the next state is a die, we need to conditionally break. This is why we keep the result as a
                // single die rather than separate non-break and break dice.
                Next::Go(Target::Die(die)) => DieArg::Die(self.self_loop_die(&die)?),
                Next::Go(Target::Outcome(next)) => {
                    DieArg::Outcome(transition(self.is_self_loop(&next)?, next))
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
    pub fn step_transition_die(
        &mut self,
        curr_die: &Die<Value>,
    ) -> std::result::Result<Die<Value>, E> {
        let mut next_states = Vec::with_capacity(curr_die.len());
        let mut quantities = Vec::with_capacity(curr_die.len());
        for (outcome, quantity) in curr_die.items() {
            crate::interrupt::poll()?;
            let p = parts(outcome);
            next_states.push(if p[0] == int(DEFAULT) {
                DieArg::Die(self.step_state(&p[1])?)
            } else {
                DieArg::Outcome(outcome.clone())
            });
            quantities.push(quantity.clone());
        }
        Ok(Die::new(next_states, quantities)?)
    }

    /// As `step_transition_die`, but keeps track of the break time in the last element of
    /// `(transition_type, state, time)`.
    pub fn step_transition_die_with_time(
        &mut self,
        curr_die: &Die<Value>,
    ) -> std::result::Result<Die<Value>, E> {
        let mut next_states = Vec::with_capacity(curr_die.len());
        let mut quantities = Vec::with_capacity(curr_die.len());
        for (outcome, quantity) in curr_die.items() {
            crate::interrupt::poll()?;
            let p = parts(outcome);
            next_states.push(if p[0] == int(DEFAULT) {
                // `self.step_state(curr_state) + (time + 1,)`: `Die.__add__` with `Die([(time + 1,)])`.
                let time = Value::tuple(vec![p[2].binary(BinOp::Add, &int(1))?]);
                let time = Die::new(vec![DieArg::Outcome(time)], vec![Quantity::ONE])?;
                let step = self.step_state(&p[1])?;
                DieArg::Die(step.binary_operator(&time, |a, b| a.binary(BinOp::Add, b))?)
            } else {
                DieArg::Outcome(outcome.clone())
            });
            quantities.push(quantity.clone());
        }
        Ok(Die::new(next_states, quantities)?)
    }
}

/// Whether any outcome of a transition die is still `DEFAULT` (`any(transition_type == TransitionType.DEFAULT
/// ...)`): `map(repeat)` stops early once the whole distribution is absorbed.
pub(crate) fn any_default(die: &Die<Value>) -> bool {
    die.outcomes().any(|o| parts(o)[0] == int(DEFAULT))
}
