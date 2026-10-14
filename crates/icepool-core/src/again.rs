//! Port of icepool `population/again.py`: the `Again` expression, which indicates that the die should be rolled
//! again, usually with some operation applied, and its evaluation in `again_depth` mode.
//!
//! In Python, `Again` is an `AgainExpression` whose operators build bigger expressions (`6 + Again`,
//! `Again @ d6`, ...); `Die(...)` then replaces each `Again` with the die rolled again. Here an expression is a
//! closed enum over two type parameters, so the core stays static: `F` is the operation (the facade's operator
//! enum, playing the role of Python's `operator.add` & co.) and `L` the facade's object type (an outcome, a die,
//! `Reroll`). The facade supplies how to apply an operation and how to build a die.
//!
//! Both modes are here: depth mode (`evaluate_agains_using_depth`), generic over the facade's types, and count
//! mode (`again_count`): its expression-level part (`is_additive`, `_again_count`, the step outcomes) is generic
//! too, and its Markov chain (`map(step, step_die, repeat=again_count + 1)`) runs over `Die<Value>`.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::creation_args::DieArg;
use crate::error::{Error, Result};
use crate::value::BinOp;
use crate::{Die, Quantity, Value};

/// An expression indicating that the die should be rolled again (`AgainExpression`).
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum Again<F, L> {
    /// The `Again` symbol itself: represents the again roll directly.
    Again,
    /// `function(*args)`, where any args that are themselves `Again` expressions resolve first. These are
    /// considered to be at the same level for purposes of `again_depth`. `is_additive`: whether the expression is
    /// compatible with `again_count`; the facade sets it as the Python dunder that built the expression would
    /// (`+` of additive expressions, `x @ Again` for a non-negative `x`; see `AgainExpression`).
    Apply {
        function: F,
        args: Vec<AgainArg<F, L>>,
        is_additive: bool,
    },
}

/// One argument of an `Again::Apply`: another expression, or any other object, passed as-is.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum AgainArg<F, L> {
    Again(Arc<Again<F, L>>),
    Other(L),
}

impl<F, L: Clone> Again<F, L> {
    /// `_evaluate`: recursively replaces the `Again` placeholders with the provided die (or other object), applying
    /// each function to its evaluated arguments, innermost first and arguments left to right, as Python does.
    pub fn evaluate<E>(
        &self,
        die: &L,
        apply: &impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
    ) -> std::result::Result<L, E> {
        match self {
            Again::Again => Ok(die.clone()),
            Again::Apply { function, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        AgainArg::Again(a) => a.evaluate(die, apply),
                        AgainArg::Other(x) => Ok(x.clone()),
                    })
                    .collect::<std::result::Result<_, _>>()?;
                apply(function, args)
            }
        }
    }

    /// `is_additive`: the bare `Again` is (`Again = AgainExpression(is_additive=True)`).
    pub fn is_additive(&self) -> bool {
        match self {
            Again::Again => true,
            Again::Apply { is_additive, .. } => *is_additive,
        }
    }

    /// `_again_count`: counts the total number of Agains in the expression, by applying the expression's
    /// functions to `one` for each `Again` and `zero` for every other argument (so `Again + Again` is `1 + 1`, and
    /// `d6 @ Again` is `d6 @ 1`). Only works on additive expressions.
    pub fn again_count<E>(
        &self,
        one: &L,
        zero: &L,
        apply: &impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
    ) -> std::result::Result<L, E> {
        match self {
            Again::Again => Ok(one.clone()),
            Again::Apply { function, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        AgainArg::Again(a) => a.again_count(one, zero, apply),
                        AgainArg::Other(_) => Ok(zero.clone()),
                    })
                    .collect::<std::result::Result<_, _>>()?;
                apply(function, args)
            }
        }
    }
}

/// `evaluate_agains_using_depth`: the die for outcomes (one per element of the facade's itemized `Die(outcomes,
/// times)` arguments) some of which are `Again` expressions, rolling again at most `again_depth` times.
///
/// The innermost level replaces every `Again` expression using `again_end`: `None` means `Reroll` (the whole
/// expression becomes `reroll`, so those outcomes drop out), `Some(end)` evaluates the expression with `end` in
/// place of `Again` (the facade resolves Python's default `None` to the zero outcome first). Each further level
/// replaces `Again` with the die built by the level below: `Die(outcomes, times, again_depth=0, again_end=tail)`.
///
/// `die` builds `Die(objects, times)` from one object per outcome; `apply` applies one operation.
pub fn evaluate_agains_using_depth<F, L: Clone, E>(
    outcomes: &[AgainArg<F, L>],
    again_depth: u64,
    again_end: Option<&L>,
    reroll: &L,
    die: impl Fn(Vec<L>) -> std::result::Result<L, E>,
    apply: impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
) -> std::result::Result<L, E> {
    let replace = |end: Option<&L>| -> std::result::Result<Vec<L>, E> {
        outcomes
            .iter()
            .map(|outcome| match (outcome, end) {
                (AgainArg::Again(a), Some(end)) => a.evaluate(end, &apply),
                (AgainArg::Again(_), None) => Ok(reroll.clone()),
                // A tuple or simple arg that is not Again.
                (AgainArg::Other(x), _) => Ok(x.clone()),
            })
            .collect()
    };
    let mut tail = die(replace(again_end)?)?;
    for _ in 0..again_depth {
        tail = die(replace(Some(&tail))?)?;
    }
    Ok(tail)
}

/// One element of count mode's step die before the facade tupleizes it (`make_step_outcome`, "add_flat,
/// add_not_again, add_again").
pub enum StepOutcome<L> {
    /// Any outcome that is not an `Again` expression (a `Reroll` or a die too, as in Python):
    /// `tupleize(zero, 1, -1)`.
    NotAgain,
    /// An additive expression: `tupleize(flat, 0, count - 1)`, with `flat = outcome._evaluate(zero)` and
    /// `count = outcome._again_count()`.
    Again { flat: L, count: L },
}

/// `[make_step_outcome(outcome) for outcome in outcomes]` of `evaluate_agains_using_count`: `zero` is the
/// not-again die's zero outcome (what `Again` evaluates to), `one` and `nought` the ints `_again_count` counts
/// with. `make` tupleizes each one, in order, as Python does.
pub fn step_outcomes<F, L: Clone, E: From<Error>>(
    outcomes: &[AgainArg<F, L>],
    zero: &L,
    one: &L,
    nought: &L,
    apply: impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
    mut make: impl FnMut(StepOutcome<L>) -> std::result::Result<L, E>,
) -> std::result::Result<Vec<L>, E> {
    outcomes
        .iter()
        .map(|outcome| match outcome {
            AgainArg::Again(a) => {
                if !a.is_additive() {
                    return Err(Error::Value(
                        "again_count mode cannot be used with a non-additive AgainExpression."
                            .into(),
                    )
                    .into());
                }
                let flat = a.evaluate(zero, &apply)?;
                let count = a.again_count(one, nought, &apply)?;
                make(StepOutcome::Again { flat, count })
            }
            AgainArg::Other(_) => make(StepOutcome::NotAgain),
        })
        .collect()
}

/// map_tools `TransitionType` (an `IntEnum`, so its members sort and hash as these ints). `Reroll` gets pruned
/// immediately, so doesn't get a transition type.
const DEFAULT: i64 = 0;
const BREAK: i64 = 1;
const RESTART: i64 = 2;

/// What `step` returns: a state `(flat, not_again, again, index)` or `Restart`.
enum Step {
    State(Value),
    Restart,
}

/// The Markov chain of count mode: `map(step, step_die, star=True, repeat=again_count + 1)` from the state
/// `(zero, 0, 1, 0)`, done as `map(repeat=n)` does it (`TransitionCache`), so every intermediate `Die` (and so
/// every quantity) is Python's: a transition die of `(transition_type, state)` tuples, stepped `n` times (stopping
/// early once no state is `DEFAULT`), then `final_map` (`Restart`ed states become `Reroll`).
struct CountChain<'a> {
    step_die: &'a Die<Value>,
    /// `again_count + 1`.
    repeat: Value,
    restart: bool,
    /// `TransitionCache._self_loop_cache`.
    self_loop: FxHashMap<Value, i64>,
}

fn int(i: i64) -> Value {
    Value::from(dashu_int::IBig::from(i))
}

/// Whether a NaN is in `v`. Python's `==` inside a tuple short-circuits on object identity, so the same NaN object
/// (the zero of a die holding inf is `inf * 0`, shared by every `(zero, 1, -1)` step outcome) makes equal tuples
/// that merge in a die, and new NaN objects do not: the port cannot know which is which.
fn has_nan(v: &Value) -> bool {
    match v {
        Value::Float(f) => f.is_nan(),
        Value::Complex(re, im) => re.is_nan() || im.is_nan(),
        Value::Tuple(items) => items.iter().any(has_nan),
        _ => false,
    }
}

fn check_nan(v: &Value) -> Result<()> {
    if has_nan(v) {
        return Err(Error::Unsupported(
            "NaN in again_count states (Python merges the same NaN object by identity)".into(),
        ));
    }
    Ok(())
}

impl CountChain<'_> {
    /// `step(flat, not_again, again, index, roll)`.
    fn step(&self, state: &Value, roll: &Value) -> Result<Step> {
        let unpack = |v: &Value, n: usize| match v {
            Value::Tuple(items) if items.len() == n => Ok(items.clone()),
            _ => Err(Error::Type(format!("cannot unpack {} into {n} values", v.type_name()))),
        };
        let s = unpack(state, 4)?;
        let (flat, not_again, again, index) = (&s[0], &s[1], &s[2], &s[3]);
        if *again == int(0) {
            return Ok(Step::State(state.clone()));
        }
        if index.binary(BinOp::Lt, &self.repeat)?.truthy() {
            let r = unpack(roll, 3)?;
            let flat = flat.binary(BinOp::Add, &r[0])?;
            check_nan(&flat)?;
            let not_again = not_again.binary(BinOp::Add, &r[1])?;
            let again = again.binary(BinOp::Add, &r[2])?;
            let index = index.binary(BinOp::Add, &int(1))?;
            if self.restart
                && index
                    .binary(BinOp::Add, &again)?
                    .binary(BinOp::Gt, &self.repeat)?
                    .truthy()
            {
                return Ok(Step::Restart);
            }
            return Ok(Step::State(Value::tuple(vec![flat, not_again, again, index])));
        }
        Ok(Step::State(state.clone()))
    }

    /// `is_self_loop`: `BREAK` if no roll of the step die moves the state (or could restart), else `DEFAULT`.
    fn is_self_loop(&mut self, state: &Value) -> Result<i64> {
        if let Some(t) = self.self_loop.get(state) {
            return Ok(*t);
        }
        let mut result = BREAK;
        for (roll, _) in self.step_die.items() {
            match self.step(state, roll)? {
                // Might restart, therefore not a self-loop.
                Step::Restart => {
                    result = DEFAULT;
                    break;
                }
                Step::State(next) if next == *state => continue,
                Step::State(_) => {
                    result = DEFAULT;
                    break;
                }
            }
        }
        self.self_loop.insert(state.clone(), result);
        Ok(result)
    }

    /// `step_state`: one step from one state, a die of `(transition_type, next_state)`.
    fn step_state(&mut self, state: &Value) -> Result<Die<Value>> {
        let mut next_states = Vec::with_capacity(self.step_die.len());
        let mut next_quantities = Vec::with_capacity(self.step_die.len());
        for (roll, quantity) in self.step_die.items() {
            next_states.push(DieArg::Outcome(match self.step(state, roll)? {
                // Will prune at end.
                Step::Restart => Value::tuple(vec![int(RESTART), Value::None]),
                Step::State(next) => Value::tuple(vec![int(self.is_self_loop(&next)?), next]),
            }));
            next_quantities.push(quantity.clone());
        }
        Die::new(next_states, next_quantities)
    }

    /// `step_transition_die`: advances every `DEFAULT` state by one step.
    fn step_transition_die(&mut self, die: &Die<Value>) -> Result<Die<Value>> {
        let mut next_states = Vec::with_capacity(die.len());
        let mut quantities = Vec::with_capacity(die.len());
        for (outcome, quantity) in die.items() {
            let Value::Tuple(t) = outcome else {
                unreachable!("transition outcomes are (type, state)")
            };
            next_states.push(if t[0] == int(DEFAULT) {
                DieArg::Die(self.step_state(&t[1])?)
            } else {
                DieArg::Outcome(outcome.clone())
            });
            quantities.push(quantity.clone());
        }
        Die::new(next_states, quantities)
    }
}

/// `evaluate_agains_using_count` once the facade has checked `again_count`, built `not_again_die` (the outcomes
/// that are not `Again`), its `zero` outcome, the step die (`step_outcomes`, tupleized and made a die) and
/// `again_end_die` (`None` for `Restart`).
///
/// Each roll adds its flat part and counts: `not_again` the rolls that end a chain, `again` the rolls still
/// pending. After `again_count + 1` steps, the result is `flat + not_again @ not_again_die + again @
/// again_end_die`: pending rolls past the limit count as `again_end`. With `Restart`, any state whose rolls could
/// pass the limit restarts the whole roll instead. `matmul` is the facade's `n @ die` (Python types of `n`).
pub fn evaluate_agains_using_count<E: From<Error>>(
    step_die: &Die<Value>,
    not_again_die: &Die<Value>,
    zero: &Value,
    again_count: u64,
    again_end_die: Option<&Die<Value>>,
    matmul: impl Fn(&Value, &Die<Value>) -> std::result::Result<Die<Value>, E>,
) -> std::result::Result<Die<Value>, E> {
    let repeat = again_count + 1;
    check_nan(zero)?;
    for (roll, _) in step_die.items() {
        check_nan(roll)?;
    }
    let mut chain = CountChain {
        step_die,
        repeat: Value::from(dashu_int::IBig::from(repeat)),
        restart: again_end_die.is_none(),
        self_loop: FxHashMap::default(),
    };
    let initial = Value::tuple(vec![zero.clone(), int(0), int(1), int(0)]);
    // `transition_cache.self_loop_die(Die([initial_state]))`.
    let t = chain.is_self_loop(&initial)?;
    let mut transition_die = Die::new(
        vec![DieArg::Outcome(Value::tuple(vec![int(t), initial]))],
        vec![Quantity::ONE],
    )?;
    for _ in 0..repeat {
        transition_die = chain.step_transition_die(&transition_die)?;
        let any_default = transition_die
            .outcomes()
            .any(|o| matches!(o, Value::Tuple(t) if t[0] == int(DEFAULT)));
        if !any_default {
            break;
        }
    }
    // `transition_die.map(final_map, star=True)`: `Restart` becomes `Reroll`.
    let (states, quantities): (Vec<_>, Vec<_>) = transition_die
        .items()
        .iter()
        .map(|(o, q)| {
            let Value::Tuple(t) = o else {
                unreachable!("transition outcomes are (type, state)")
            };
            let arg = if t[0] == int(RESTART) {
                DieArg::Reroll
            } else {
                DieArg::Outcome(t[1].clone())
            };
            (arg, q.clone())
        })
        .unzip();
    let final_state = Die::new(states, quantities)?;
    // `final_state.map(finalize, star=True)`: `flat + terminal @ not_again_die (+ again @ again_end_die)`.
    let add = |a: &Die<Value>, b: &Die<Value>| a.binary_operator(b, |x, y| x.binary(BinOp::Add, y));
    let mut results = Vec::with_capacity(final_state.len());
    let mut quantities = Vec::with_capacity(final_state.len());
    for (state, quantity) in final_state.items() {
        let Value::Tuple(s) = state else {
            unreachable!("states are (flat, not_again, again, index)")
        };
        // `flat + die` is `Die([flat]).binary_operator(die, operator.add)` (`Die.__radd__`).
        let flat = Die::new(vec![DieArg::Outcome(s[0].clone())], vec![Quantity::ONE])?;
        let mut result = add(&flat, &matmul(&s[1], not_again_die)?)?;
        if let Some(end) = again_end_die {
            result = add(&result, &matmul(&s[2], end)?)?;
        }
        results.push(DieArg::Die(result));
        quantities.push(quantity.clone());
    }
    Ok(Die::new(results, quantities)?)
}
