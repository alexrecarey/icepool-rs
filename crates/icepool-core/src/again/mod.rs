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
//!
//! Python evaluates an expression recursively, so a deep one (`e = e + 1` in a loop) raises `RecursionError`. The
//! recursion here counts the Python frames it stands for (`value::recursion`): `_evaluate` (or `_again_count`) for
//! the expression, then the generator over its arguments and `_evaluate_arg` for each argument that is itself an
//! expression; calling its function, `call_frames` more.

use std::sync::Arc;

mod typed;
pub use typed::{Again, AgainEnd, AgainOutcome};

use crate::creation_args::DieArg;
use crate::error::{Error, Result};
use crate::map_tools::{map_repeat, TransitionCache};
use crate::Mapped;
use crate::value::recursion::{frames, level};
use crate::value::BinOp;
use crate::{Die, Quantity, Value};

/// The Python frames a function of an expression uses at most in icepool when it is given a die: an operator on a
/// die (`Die.__add__`, `binary_operator`, ...). Calibrated so that `Die([1, e])` for `e = Again + 1 + 1 ...` raises
/// from the same nesting as Python (330 at top level). Only depth mode's outer levels give the expressions a die;
/// its innermost level and count mode give them an outcome (usually an `int`, whose operators take no frames).
const DIE_CALL_FRAMES: usize = 6;

/// Python's frames for an argument that is itself an expression: the generator and `_evaluate_arg`, then the
/// expression's own `_evaluate`, with a level of native recursion.
fn descend<T, E: From<Error>>(
    f: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let _level = level()?;
    let _frames = frames(3)?;
    f()
}

/// An expression indicating that the die should be rolled again (`AgainExpression`).
///
/// For language bindings, whose `Again` this is: `F` is the binding's operation, `L` its object type. Typed dice
/// use [`Again`](crate::Again) instead.
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::value::{AgainArg, AgainExpression};
/// // `Again + 10`, with `char` operations over `i64` objects.
/// let again = Arc::new(AgainExpression::Again { is_additive: true });
/// let plus_ten: AgainExpression<char, i64> = AgainExpression::Apply {
///     function: '+',
///     args: vec![AgainArg::Again(again), AgainArg::Other(10)],
///     is_additive: true,
/// };
/// assert!(matches!(plus_ten, AgainExpression::Apply { function: '+', .. }));
/// ```
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum AgainExpression<F, L> {
    /// An expression without a function: represents the again roll directly. The `Again` symbol is
    /// `AgainExpression(is_additive=True)`; one made by hand as `AgainExpression()` is not additive, and count mode
    /// rejects it like any other non-additive expression (only the top-level flag is ever checked).
    Again {
        /// Whether the expression is compatible with `again_count`.
        is_additive: bool,
    },
    /// `function(*args)`, where any args that are themselves `Again` expressions resolve first. These are
    /// considered to be at the same level for purposes of `again_depth`. `is_additive`: whether the expression is
    /// compatible with `again_count`; the facade sets it as the Python dunder that built the expression would
    /// (`+` of additive expressions, `x @ Again` for a non-negative `x`; see `AgainExpression`).
    Apply {
        /// The operation (Python's `operator.add` & co.), applied by the facade's `apply`.
        function: F,
        /// Its arguments, in order.
        args: Vec<AgainArg<F, L>>,
        /// Whether the expression is compatible with `again_count`.
        is_additive: bool,
    },
}

/// One argument of an `AgainExpression::Apply`: another expression, or any other object, passed as-is.
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::value::{AgainArg, AgainExpression};
/// // The outcomes of `Die([1, Again])`.
/// let outcomes: Vec<AgainArg<char, i64>> =
///     vec![AgainArg::Other(1), AgainArg::Again(Arc::new(AgainExpression::Again { is_additive: true }))];
/// assert!(matches!(outcomes[0], AgainArg::Other(1)));
/// ```
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum AgainArg<F, L> {
    /// An `Again` expression.
    Again(Arc<AgainExpression<F, L>>),
    /// Any other object (an outcome, a die, `Reroll`).
    Other(L),
}

impl<F, L: Clone> AgainExpression<F, L> {
    /// `_evaluate`: recursively replaces the `Again` placeholders with the provided die (or other object), applying
    /// each function to its evaluated arguments, innermost first and arguments left to right, as Python does.
    ///
    /// The caller has taken the frame of the top-level `_evaluate`.
    pub fn evaluate<E: From<Error>>(
        &self,
        die: &L,
        apply: &impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
        call_frames: usize,
    ) -> std::result::Result<L, E> {
        match self {
            AgainExpression::Again { .. } => Ok(die.clone()),
            AgainExpression::Apply { function, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        AgainArg::Again(a) => descend(|| a.evaluate(die, apply, call_frames)),
                        AgainArg::Other(x) => Ok(x.clone()),
                    })
                    .collect::<std::result::Result<_, _>>()?;
                let _frames = frames(call_frames)?;
                apply(function, args)
            }
        }
    }

    /// `is_additive`: as the facade (or the dunder that built the expression) set it.
    pub fn is_additive(&self) -> bool {
        match self {
            AgainExpression::Again { is_additive } | AgainExpression::Apply { is_additive, .. } => {
                *is_additive
            }
        }
    }

    /// `_again_count`: counts the total number of Agains in the expression, by applying the expression's
    /// functions to `one` for each `Again` and `zero` for every other argument (so `Again + Again` is `1 + 1`, and
    /// `d6 @ Again` is `d6 @ 1`). Only works on additive expressions.
    /// The caller has taken the frame of the top-level `_again_count`.
    pub fn again_count<E: From<Error>>(
        &self,
        one: &L,
        zero: &L,
        apply: &impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
    ) -> std::result::Result<L, E> {
        match self {
            AgainExpression::Again { .. } => Ok(one.clone()),
            AgainExpression::Apply { function, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        AgainArg::Again(a) => descend(|| a.again_count(one, zero, apply)),
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
///
/// For language bindings (typed dice: [`Die::with_again_depth`](crate::Die::with_again_depth)):
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::value::{evaluate_agains_using_depth, AgainArg, AgainExpression, DieArg};
/// use icepool_core::{Die, Error, Quantity};
/// // `Die([1, 2, Again + 10], again_depth=1, again_end=0)`, the objects being `DieArg<i64>`s.
/// let again = Arc::new(AgainExpression::Again { is_additive: true });
/// let plus_ten = AgainExpression::Apply {
///     function: '+',
///     args: vec![AgainArg::Again(again), AgainArg::Other(DieArg::Outcome(10))],
///     is_additive: true,
/// };
/// let outcomes = vec![
///     AgainArg::Other(DieArg::Outcome(1)),
///     AgainArg::Other(DieArg::Outcome(2)),
///     AgainArg::Again(Arc::new(plus_ten)),
/// ];
/// let die = |objects| Die::new(objects, vec![Quantity::ONE; 3]).map(DieArg::Die);
/// let add = |_: &char, args: Vec<DieArg<i64>>| -> Result<DieArg<i64>, Error> {
///     Ok(match (&args[0], &args[1]) {
///         (DieArg::Outcome(a), DieArg::Outcome(b)) => DieArg::Outcome(a + b),
///         (DieArg::Die(a), DieArg::Outcome(b)) => DieArg::Die(a + *b),
///         _ => unreachable!("only `Again + 10` here"),
///     })
/// };
/// let end = DieArg::Outcome(0);
/// let result = evaluate_agains_using_depth(&outcomes, 1, Some(&end), &DieArg::Reroll, die, add).unwrap();
/// let DieArg::Die(result) = result else { unreachable!() };
/// assert_eq!(result, Die::from_weights([(1, 3u32), (2, 3), (11, 1), (12, 1), (20, 1)]));
/// ```
pub fn evaluate_agains_using_depth<F, L: Clone, E: From<Error>>(
    outcomes: &[AgainArg<F, L>],
    again_depth: u64,
    again_end: Option<&L>,
    reroll: &L,
    die: impl Fn(Vec<L>) -> std::result::Result<L, E>,
    apply: impl Fn(&F, Vec<L>) -> std::result::Result<L, E>,
) -> std::result::Result<L, E> {
    // Python's frames below the facade's call: each level after the innermost is a `Die(...)` call (`Die.__new__`
    // and `evaluate_agains_using_depth` again), then `replace_again` and `_evaluate`.
    let replace = |end: Option<&L>,
                   frames_above: usize,
                   call_frames: usize|
     -> std::result::Result<Vec<L>, E> {
        outcomes
            .iter()
            .map(|outcome| match (outcome, end) {
                (AgainArg::Again(a), Some(end)) => {
                    let _frames = frames(frames_above + 2)?;
                    a.evaluate(end, &apply, call_frames)
                }
                (AgainArg::Again(_), None) => Ok(reroll.clone()),
                // A tuple or simple arg that is not Again.
                (AgainArg::Other(x), _) => Ok(x.clone()),
            })
            .collect()
    };
    let mut tail = die(replace(again_end, 0, 0)?)?;
    for _ in 0..again_depth {
        tail = die(replace(Some(&tail), 2, DIE_CALL_FRAMES)?)?;
    }
    Ok(tail)
}

/// One element of count mode's step die before the facade tupleizes it (`make_step_outcome`, "add_flat,
/// add_not_again, add_again").
///
/// For language bindings: what [`step_outcomes`](crate::value::step_outcomes) hands its `make` callback.
///
/// ```
/// use icepool_core::value::StepOutcome;
/// let step: StepOutcome<i64> = StepOutcome::Again { flat: 3, count: 1 };
/// assert!(matches!(step, StepOutcome::Again { flat: 3, .. }));
/// ```
#[derive(Debug)]
pub enum StepOutcome<L> {
    /// Any outcome that is not an `Again` expression (a `Reroll` or a die too, as in Python):
    /// `tupleize(zero, 1, -1)`.
    NotAgain,
    /// An additive expression: `tupleize(flat, 0, count - 1)`, with `flat = outcome._evaluate(zero)` and
    /// `count = outcome._again_count()`.
    Again {
        /// The expression evaluated with `zero` in place of `Again`.
        flat: L,
        /// How many `Again`s it holds (`_again_count`).
        count: L,
    },
}

/// `[make_step_outcome(outcome) for outcome in outcomes]` of `evaluate_agains_using_count`: `zero` is the
/// not-again die's zero outcome (what `Again` evaluates to), `one` and `nought` the ints `_again_count` counts
/// with. `make` tupleizes each one, in order, as Python does.
///
/// For language bindings:
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::value::{step_outcomes, AgainArg, AgainExpression, BinOp, StepOutcome};
/// use icepool_core::{Error, Value};
/// // The step outcomes of `Die([1, 3 + Again])`.
/// let again = Arc::new(AgainExpression::Again { is_additive: true });
/// let three_plus = AgainExpression::Apply {
///     function: BinOp::Add,
///     args: vec![AgainArg::Other(Value::from(3)), AgainArg::Again(again)],
///     is_additive: true,
/// };
/// let outcomes = [AgainArg::Other(Value::from(1)), AgainArg::Again(Arc::new(three_plus))];
/// let apply = |op: &BinOp, args: Vec<Value>| args[0].binary(*op, &args[1]);
/// let make = |step| -> Result<Value, Error> {
///     Ok(match step {
///         StepOutcome::NotAgain => Value::tuple(vec![Value::from(0), Value::from(1), Value::from(-1)]),
///         StepOutcome::Again { flat, count } => Value::tuple(vec![flat, Value::from(0), count]),
///     })
/// };
/// let (zero, one, nought) = (Value::from(0), Value::from(1), Value::from(0));
/// let steps = step_outcomes(&outcomes, &zero, &one, &nought, apply, make).unwrap();
/// let three = Value::tuple(vec![Value::from(3), Value::from(0), Value::from(1)]);
/// assert_eq!(steps[1], three); // flat 3, one `Again`
/// ```
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
                    return Err(Error::InvalidArgument(
                        "again_count mode cannot be used with a non-additive AgainExpression."
                            .into(),
                    )
                    .into());
                }
                // `make_step_outcome`, then `_evaluate` / `_again_count`.
                let _frames = frames(2)?;
                let flat = a.evaluate(zero, &apply, 0)?;
                let count = a.again_count(one, nought, &apply)?;
                make(StepOutcome::Again { flat, count })
            }
            AgainArg::Other(_) => make(StepOutcome::NotAgain),
        })
        .collect()
}

/// What `step` returns: a state `(flat, not_again, again, index)` or `Restart`.
enum Step {
    State(Value),
    Restart,
}

fn int(i: i64) -> Value {
    Value::from(dashu_int::IBig::from(i))
}

/// `step(flat, not_again, again, index, roll)` of count mode's Markov chain; `repeat` is `again_count + 1`.
fn step(state: &Value, roll: &Value, repeat: &Value, restart: bool) -> Result<Step> {
    let unpack = |v: &Value, n: usize| match v {
        Value::Tuple(items) if items.len() == n => Ok(items.clone()),
        _ => Err(Error::WrongType(format!(
            "cannot unpack {} into {n} values",
            v.type_name()
        ))),
    };
    let s = unpack(state, 4)?;
    let (flat, not_again, again, index) = (&s[0], &s[1], &s[2], &s[3]);
    if *again == int(0) {
        return Ok(Step::State(state.clone()));
    }
    if index.binary(BinOp::Lt, repeat)?.truthy()? {
        let r = unpack(roll, 3)?;
        let flat = flat.binary(BinOp::Add, &r[0])?;
        let not_again = not_again.binary(BinOp::Add, &r[1])?;
        let again = again.binary(BinOp::Add, &r[2])?;
        let index = index.binary(BinOp::Add, &int(1))?;
        if restart
            && index
                .binary(BinOp::Add, &again)?
                .binary(BinOp::Gt, repeat)?
                .truthy()?
        {
            return Ok(Step::Restart);
        }
        return Ok(Step::State(Value::tuple(vec![
            flat, not_again, again, index,
        ])));
    }
    Ok(Step::State(state.clone()))
}

/// `evaluate_agains_using_count` once the facade has checked `again_count`, built `not_again_die` (the outcomes
/// that are not `Again`), its `zero` outcome, the step die (`step_outcomes`, tupleized and made a die) and
/// `again_end_die` (`None` for `Restart`).
///
/// Each roll adds its flat part and counts: `not_again` the rolls that end a chain, `again` the rolls still
/// pending. After `again_count + 1` steps, the result is `flat + not_again @ not_again_die + again @
/// again_end_die`: pending rolls past the limit count as `again_end`. With `Restart`, any state whose rolls could
/// pass the limit restarts the whole roll instead. `matmul` is the facade's `n @ die` (Python types of `n`).
///
/// For language bindings (typed dice: [`Die::with_again_count`](crate::Die::with_again_count)):
///
/// ```
/// use icepool_core::value::{evaluate_agains_using_count, matmul_dice};
/// use icepool_core::{Die, Error, Quantity, Value};
/// let int = |i: i64| Value::from(i);
/// let die = |outcomes: Vec<Value>| Die::from_dict(outcomes.into_iter().map(|o| (o, Quantity::ONE)).collect());
/// let step = |flat, not_again, again| Value::tuple(vec![int(flat), int(not_again), int(again)]);
/// // `Die([1, 2, 3 + Again], again_count=1)`: the step die of `step_outcomes`, tupleized.
/// let step_die = die(vec![step(0, 1, -1), step(0, 1, -1), step(3, 0, 0)]).unwrap();
/// let not_again_die = die(vec![int(1), int(2)]).unwrap();
/// let again_end_die = die(vec![int(0)]).unwrap();
/// let matmul = |n: &Value, d: &Die<Value>| -> Result<Die<Value>, Error> { matmul_dice(&die(vec![n.clone()])?, d) };
/// let result = evaluate_agains_using_count(&step_die, &not_again_die, &int(0), 1, Some(&again_end_die), matmul);
/// let expected = [(1, 3u8), (2, 3), (4, 1), (5, 1), (6, 1)].map(|(o, q)| (int(o), Quantity::from(q)));
/// assert_eq!(result.unwrap(), Die::from_dict(expected.to_vec()).unwrap());
/// ```
pub fn evaluate_agains_using_count<E: From<Error>>(
    step_die: &Die<Value>,
    not_again_die: &Die<Value>,
    zero: &Value,
    again_count: u64,
    again_end_die: Option<&Die<Value>>,
    matmul: impl Fn(&Value, &Die<Value>) -> std::result::Result<Die<Value>, E>,
) -> std::result::Result<Die<Value>, E> {
    let repeat = Value::from(dashu_int::IBig::from(again_count + 1));
    let restart = again_end_die.is_none();
    // `map(step, step_die, star=True, repeat=again_count + 1)` from the state `(zero, 0, 1, 0)`, done as
    // `map(repeat=n)` does it (`TransitionCache`), so every intermediate `Die` (and so every quantity) is
    // Python's. NaN states are refused there: Python's tuples holding the same NaN object (the zero of a die
    // holding inf is `inf * 0`, shared by every `(zero, 1, -1)` step outcome) compare equal by identity.
    let rolls = step_die.items();
    let mut cache = TransitionCache::new(
        |state: &Value, i: usize| -> Result<Mapped<Value>> {
            Ok(match step(state, &rolls[i].0, &repeat, restart)? {
                Step::Restart => Mapped::Restart,
                Step::State(next) => Mapped::Outcome(next),
            })
        },
        rolls.iter().map(|(_, q)| q.clone()).collect(),
    );
    let initial = Value::tuple(vec![zero.clone(), int(0), int(1), int(0)]);
    let initial = Die::new(vec![DieArg::Outcome(initial)], vec![Quantity::ONE])?;
    // `final_map` makes `Restart`ed states `Reroll`.
    let final_state = map_repeat(&mut cache, &initial, Some(again_count + 1))?;
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
