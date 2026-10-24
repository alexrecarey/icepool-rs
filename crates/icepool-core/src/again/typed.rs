//! `Again` for typed outcomes: an expression over `O: OutcomeArith` built with Rust's operators
//! (`Again::new() + 1`), the dice made from such expressions in depth mode (`Die(..., again_depth=, again_end=)`,
//! `die.map(..., again_depth=)`) and count mode (`Die(..., again_count=)`, a Markov chain over typed states), and
//! `Die.explode`, which is depth-mode `Again` (population/die.py).
//!
//! The expression is the engine's [`AgainExpression`] over [`AgainOp`] (the arithmetic `OutcomeArith` has) and
//! `DieArg<O>` (an operand is an outcome; once `Again` is replaced, a die). Python's `Again` takes any operator and
//! any object; that dynamic form is the bindings', over their own operations and objects.
//!
//! The engine counts the Python frames an expression would take (`value::recursion`), so an expression nested
//! about a thousand deep stops with `Error::RecursionLimit`, as Python's does, rather than overflowing the stack.

use std::ops::{Add, Mul, Neg, Sub};
use std::sync::Arc;

use super::{evaluate_agains_using_depth, AgainArg, AgainExpression};
use crate::creation_args::DieArg;
use crate::die::Die;
use crate::error::{Error, Result};
use crate::map_tools::{map_repeat, TransitionCache};
use crate::outcome::{Outcome, OutcomeArith};
use crate::Mapped;
use crate::Quantity;

/// The operations a typed [`Again`] expression applies: Python's `operator.add`, `sub`, `mul` and `neg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgainOp {
    /// `a + b`.
    Add,
    /// `a - b`.
    Sub,
    /// `a * b`.
    Mul,
    /// `-a`.
    Neg,
}

/// `icepool.Again`: stands for the die rolled again, usually with some arithmetic applied (`Again::new() + 6`).
///
/// Give expressions to [`Die::with_again_depth`] or [`Die::map_again`]: each `Again` is replaced by the die
/// being defined, up to a depth, and past that by the `end` ([`AgainEnd`]). Every `Again` in an expression is a
/// separate roll, and all are at the same depth. An integer literal on the left of `Again` needs its type
/// (`2i64 * Again::new()`): Rust would otherwise take it for an `i32`.
///
/// ```
/// use icepool_core::{d, Again, AgainEnd, Die};
/// // `d(6).map(lambda x: 6 + Again if x == 6 else x, again_depth=1)`: an exploding d6, once.
/// let once = d(6).map_again(|&x| if x == 6 { (6i64 + Again::new()).into() } else { x.into() }, 1, AgainEnd::Zero);
/// assert_eq!(once.unwrap(), d(6).explode(|&x| x == 6, 1, AgainEnd::Zero).unwrap());
/// ```
#[derive(Clone, Debug)]
pub struct Again<O>(Arc<AgainExpression<AgainOp, DieArg<O>>>);

impl<O: Clone> Again<O> {
    /// The `Again` symbol: the die rolled again, as is.
    ///
    /// ```
    /// use icepool_core::{Again, AgainEnd, Die, Quantity};
    /// // `Die([1, 2, Again], again_depth=2)`: past the depth, `Again` is zero.
    /// let outcomes = vec![(1.into(), Quantity::ONE), (2.into(), Quantity::ONE), (Again::new().into(), Quantity::ONE)];
    /// let die = Die::with_again_depth(outcomes, 2, AgainEnd::Zero).unwrap();
    /// assert_eq!(die, Die::from_weights([(0, 1u32), (1, 13), (2, 13)]));
    /// ```
    pub fn new() -> Self {
        Again(Arc::new(AgainExpression::Again { is_additive: true }))
    }

    /// `AgainExpression(op, *args)` with `is_additive` (compatible with `again_count`) set as Python's dunders set
    /// it: `+` is additive if every expression among its operands is; nothing else is.
    fn apply(op: AgainOp, args: Vec<AgainArg<AgainOp, DieArg<O>>>) -> Self {
        let is_additive = op == AgainOp::Add
            && args.iter().all(|arg| match arg {
                AgainArg::Again(a) => a.is_additive(),
                AgainArg::Other(_) => true,
            });
        Again(Arc::new(AgainExpression::Apply {
            function: op,
            args,
            is_additive,
        }))
    }

    fn arg(self) -> AgainArg<AgainOp, DieArg<O>> {
        AgainArg::Again(self.0)
    }
}

impl<O: Clone> Default for Again<O> {
    fn default() -> Self {
        Again::new()
    }
}

fn outcome_arg<O>(outcome: O) -> AgainArg<AgainOp, DieArg<O>> {
    AgainArg::Other(DieArg::Outcome(outcome))
}

/// `Again op Again`, `Again op outcome` and, for the primitive integers (by macro: the orphan rule), `outcome op
/// Again`, each argument in the order written.
macro_rules! again_binary_ops {
    ($($trait:ident $method:ident $op:ident),*) => {$(
        impl<O: Clone> $trait for Again<O> {
            type Output = Again<O>;
            fn $method(self, other: Again<O>) -> Again<O> {
                Again::apply(AgainOp::$op, vec![self.arg(), other.arg()])
            }
        }
        impl<O: OutcomeArith> $trait<O> for Again<O> {
            type Output = Again<O>;
            fn $method(self, other: O) -> Again<O> {
                Again::apply(AgainOp::$op, vec![self.arg(), outcome_arg(other)])
            }
        }
    )*};
}
again_binary_ops!(Add add Add, Sub sub Sub, Mul mul Mul);

macro_rules! again_reflected_ops {
    ($($t:ty),*) => {$(
        impl Add<Again<$t>> for $t {
            type Output = Again<$t>;
            fn add(self, again: Again<$t>) -> Again<$t> {
                Again::apply(AgainOp::Add, vec![outcome_arg(self), again.arg()])
            }
        }
        impl Sub<Again<$t>> for $t {
            type Output = Again<$t>;
            fn sub(self, again: Again<$t>) -> Again<$t> {
                Again::apply(AgainOp::Sub, vec![outcome_arg(self), again.arg()])
            }
        }
        impl Mul<Again<$t>> for $t {
            type Output = Again<$t>;
            fn mul(self, again: Again<$t>) -> Again<$t> {
                Again::apply(AgainOp::Mul, vec![outcome_arg(self), again.arg()])
            }
        }
    )*};
}
again_reflected_ops!(i8, i16, i32, i64, i128, isize);

impl<O: Clone> Neg for Again<O> {
    type Output = Again<O>;
    fn neg(self) -> Again<O> {
        Again::apply(AgainOp::Neg, vec![self.arg()])
    }
}

/// One of the outcomes given to [`Die::with_again_depth`], or returned by the function of [`Die::map_again`]:
/// what Python's `Die(outcomes)` accepts there.
///
/// ```
/// use icepool_core::{d, Again, AgainEnd, AgainOutcome, Die, Quantity};
/// // `Die([1, Reroll, d(2), Again + 1], again_depth=1)`
/// let outcomes = vec![
///     (AgainOutcome::Outcome(1), Quantity::ONE),
///     (AgainOutcome::Reroll, Quantity::ONE),
///     (AgainOutcome::Die(d(2)), Quantity::ONE),
///     (AgainOutcome::Again(Again::new() + 1), Quantity::ONE),
/// ];
/// let die = Die::with_again_depth(outcomes, 1, AgainEnd::Zero).unwrap();
/// assert_eq!(die, Die::from_weights([(1, 9u32), (2, 8), (3, 1)]));
/// ```
#[derive(Clone, Debug)]
pub enum AgainOutcome<O> {
    /// A plain outcome.
    Outcome(O),
    /// A die, mixed in as `Die()` mixes dice.
    Die(Die<O>),
    /// `icepool.Reroll`: drops out.
    Reroll,
    /// An `Again` expression.
    Again(Again<O>),
}

impl<O> From<O> for AgainOutcome<O> {
    fn from(outcome: O) -> Self {
        AgainOutcome::Outcome(outcome)
    }
}

impl<O> From<Again<O>> for AgainOutcome<O> {
    fn from(again: Again<O>) -> Self {
        AgainOutcome::Again(again)
    }
}

/// What an `Again` becomes once the depth is reached: Python's `again_end`.
///
/// ```
/// use icepool_core::{Again, AgainEnd, Die, Quantity};
/// let outcomes = || vec![(1.into(), Quantity::ONE), (2.into(), Quantity::ONE), (Again::new().into(), Quantity::ONE)];
/// // `Die([1, 2, Again], again_depth=2, again_end=7)`
/// let die = Die::with_again_depth(outcomes(), 2, AgainEnd::Outcome(7)).unwrap();
/// assert_eq!(die, Die::from_weights([(1, 13u32), (2, 13), (7, 1)]));
/// // `Die([1, 2, Again + 5], again_depth=1, again_end=Reroll)`
/// let outcomes = vec![
///     (1.into(), Quantity::ONE),
///     (2.into(), Quantity::ONE),
///     ((Again::new() + 5).into(), Quantity::ONE),
/// ];
/// let die = Die::with_again_depth(outcomes, 1, AgainEnd::Reroll).unwrap();
/// assert_eq!(die, Die::from_weights([(1, 2u32), (2, 2), (6, 1), (7, 1)]));
/// ```
#[derive(Clone, Debug)]
pub enum AgainEnd<O> {
    /// The default (`None`): the zero outcome of the outcomes that are not `Again` (`x * 0` of the lowest).
    Zero,
    /// This outcome.
    Outcome(O),
    /// This die.
    Die(Die<O>),
    /// `icepool.Reroll`: an expression still holding an `Again` at the last level drops out, rerolling until an
    /// outcome without `Again` comes up. (Count mode: the rolls past the count are rolls of the outcomes that are
    /// not `Again`.)
    Reroll,
    /// `icepool.Restart` (count mode only): a roll that would pass the count restarts the whole roll.
    Restart,
}

/// The function of an expression, applied to its evaluated arguments: outcomes as `OutcomeArith` has it, and
/// dice (once `Again` is a die) as the die operators, `outcome op die` being `Die([outcome]) op die`.
fn call<O: OutcomeArith>(op: &AgainOp, args: Vec<DieArg<O>>) -> Result<DieArg<O>> {
    let as_die = |arg: &DieArg<O>| match arg {
        DieArg::Die(d) => Ok(d.clone()),
        DieArg::Outcome(o) => Die::new(vec![DieArg::Outcome(o.clone())], vec![Quantity::ONE]),
        // Depth mode replaces a whole expression with `Reroll`, never one of its arguments.
        DieArg::Reroll => unreachable!("an Again expression is never evaluated with Reroll"),
    };
    let binary = |f: fn(&O, &O) -> Result<O>| match (&args[0], &args[1]) {
        (DieArg::Outcome(a), DieArg::Outcome(b)) => Ok(DieArg::Outcome(f(a, b)?)),
        (a, b) => Ok(DieArg::Die(as_die(a)?.binary_operator(&as_die(b)?, f)?)),
    };
    match op {
        AgainOp::Add => binary(O::try_add),
        AgainOp::Sub => binary(O::try_sub),
        AgainOp::Mul => binary(O::try_mul),
        AgainOp::Neg => match &args[0] {
            DieArg::Outcome(a) => Ok(DieArg::Outcome(a.try_neg()?)),
            a => Ok(DieArg::Die(as_die(a)?.unary_operator(O::try_neg)?)),
        },
    }
}

impl<O: OutcomeArith> Die<O> {
    /// `Die(outcomes, times, again_depth=depth, again_end=end)` (`evaluate_agains_using_depth`): each `Again`
    /// is replaced with this die, rolled again, at most `depth` times; at the last level, with `end`.
    ///
    /// Without any `Again` among the outcomes, this is the plain `Die(outcomes, times)` (`depth` and `end`
    /// unused). [`AgainEnd::Zero`] with every outcome an `Again` fails as Python's `Die()` with no arguments
    /// does.
    ///
    /// ```
    /// use icepool_core::{Again, AgainEnd, Die, Quantity};
    /// // `Die([1, 2, Again + 5], [1, 2, 3], again_depth=1)`
    /// let q = |n: u8| Quantity::from(n);
    /// let outcomes = vec![(1.into(), q(1)), (2.into(), q(2)), ((Again::new() + 5).into(), q(3))];
    /// let die = Die::with_again_depth(outcomes, 1, AgainEnd::Zero).unwrap();
    /// assert_eq!(die, Die::from_weights([(1, 2u32), (2, 4), (6, 1), (7, 2), (10, 3)]));
    /// ```
    pub fn with_again_depth(
        outcomes: Vec<(AgainOutcome<O>, Quantity)>,
        depth: u64,
        end: AgainEnd<O>,
    ) -> Result<Self> {
        let (outcomes, times): (Vec<_>, Vec<_>) = outcomes
            .into_iter()
            .map(|(outcome, quantity)| (outcome_or_again(outcome), quantity))
            .unzip();
        if !outcomes.iter().any(|x| matches!(x, AgainArg::Again(_))) {
            return Die::new(plain(outcomes), times);
        }
        let end = match end {
            AgainEnd::Zero => Some(DieArg::Outcome(zero_outcome(&not_again_die(
                &outcomes, &times,
            )?)?)),
            AgainEnd::Outcome(o) => Some(DieArg::Outcome(o)),
            AgainEnd::Die(d) => Some(DieArg::Die(d)),
            AgainEnd::Reroll => None,
            // Every outcome that is an expression raises this, so the first one does.
            AgainEnd::Restart => {
                return Err(Error::InvalidArgument(
                    "Restart is not compatible with again_depth.".into(),
                ))
            }
        };
        let result = evaluate_agains_using_depth(
            &outcomes,
            depth,
            end.as_ref(),
            &DieArg::Reroll,
            |objects| Die::new(objects, times.clone()).map(DieArg::Die),
            call,
        )?;
        match result {
            DieArg::Die(die) => Ok(die),
            _ => unreachable!("evaluate_agains_using_depth returns what `Die::new` built"),
        }
    }

    /// `die.map(f, again_depth=depth, again_end=end)`: `f` of every outcome (zero quantities included), which
    /// may be an `Again` expression, made into a die by [`Die::with_again_depth`].
    ///
    /// ```
    /// use icepool_core::{d, Again, AgainEnd, Die};
    /// // `d(3).map(lambda x: 10 - Again if x == 3 else x, again_depth=1)`
    /// let die = d(3).map_again(|&x| if x == 3 { (10i64 - Again::new()).into() } else { x.into() }, 1, AgainEnd::Zero);
    /// assert_eq!(die.unwrap(), Die::from_weights([(0, 1u32), (1, 3), (2, 3), (8, 1), (9, 1)]));
    /// ```
    pub fn map_again(
        &self,
        mut f: impl FnMut(&O) -> AgainOutcome<O>,
        depth: u64,
        end: AgainEnd<O>,
    ) -> Result<Self> {
        let outcomes = self
            .items()
            .iter()
            .map(|(outcome, quantity)| (f(outcome), quantity.clone()))
            .collect();
        Die::with_again_depth(outcomes, depth, end)
    }

    /// Causes the outcomes for which `which` is true to be rolled again and added to the total, at most `depth`
    /// more times (Python's `die.explode(which, depth=depth, end=end)`; Python's default depth is 9 and its
    /// default `which` the max outcome). Past the depth, a further explosion counts as `end`; [`AgainEnd::Reroll`]
    /// makes one extra final roll, rerolling until a non-exploding outcome is reached.
    ///
    /// As icepool does it: `die.map(lambda o: o + Again if o in which else o, again_depth=depth, again_end=end)`.
    ///
    /// ```
    /// use icepool_core::{d, AgainEnd, Die};
    /// // `d(4).explode(depth=1)`
    /// let exploded = d(4).explode(|&x| x == 4, 1, AgainEnd::Zero).unwrap();
    /// assert_eq!(exploded, Die::from_weights([(1, 4u32), (2, 4), (3, 4), (5, 1), (6, 1), (7, 1), (8, 1)]));
    /// ```
    pub fn explode(
        &self,
        mut which: impl FnMut(&O) -> bool,
        depth: u64,
        end: AgainEnd<O>,
    ) -> Result<Self> {
        if depth == 0 {
            return Ok(self.clone());
        }
        self.map_again(
            |outcome| {
                if which(outcome) {
                    AgainOutcome::Again(Again::apply(
                        AgainOp::Add,
                        vec![outcome_arg(outcome.clone()), Again::new().arg()],
                    ))
                } else {
                    AgainOutcome::Outcome(outcome.clone())
                }
            },
            depth,
            end,
        )
    }
}

/// A state of count mode's process: `(flat, not_again, again, index)`, the total so far, the rolls that ended a
/// chain, the rolls still pending, and the number of steps taken.
type CountState<O> = (O, i64, i64, u64);

impl<O: Outcome + OutcomeArith> Die<O> {
    /// `Die(outcomes, times, again_count=count, again_end=end)` (`evaluate_agains_using_count`): each `Again` is
    /// rolled again, up to `count` rolls in total beyond the first; the rolls past that count as `end`
    /// ([`AgainEnd::Zero`]: a zero, [`AgainEnd::Reroll`]: a roll of the outcomes that are not `Again`,
    /// [`AgainEnd::Restart`]: the whole roll starts over). Only additive expressions (sums of `Again`s and
    /// outcomes) are allowed: an [`Error::InvalidArgument`] otherwise, as Python's `ValueError`.
    ///
    /// ```
    /// use icepool_core::{Again, AgainEnd, Die, Quantity};
    /// // `Die([1, 2, 3 + Again], again_count=1)`: at most one extra roll.
    /// let outcomes = vec![(1.into(), Quantity::ONE), (2.into(), Quantity::ONE), ((3i64 + Again::new()).into(), Quantity::ONE)];
    /// let die = Die::with_again_count(outcomes, 1, AgainEnd::Zero).unwrap();
    /// assert_eq!(die, Die::from_weights([(1, 3u32), (2, 3), (4, 1), (5, 1), (6, 1)]));
    /// ```
    pub fn with_again_count(
        outcomes: Vec<(AgainOutcome<O>, Quantity)>,
        count: u64,
        end: AgainEnd<O>,
    ) -> Result<Self> {
        let (outcomes, times): (Vec<_>, Vec<_>) = outcomes
            .into_iter()
            .map(|(outcome, quantity)| (outcome_or_again(outcome), quantity))
            .unzip();
        if !outcomes.iter().any(|x| matches!(x, AgainArg::Again(_))) {
            return Die::new(plain(outcomes), times);
        }
        let not_again_die = not_again_die(&outcomes, &times)?;
        let zero = zero_outcome(&not_again_die)?;
        let again_end_die = match end {
            AgainEnd::Zero => Some(Die::single(zero.clone())),
            AgainEnd::Outcome(o) => Some(Die::single(o)),
            AgainEnd::Die(d) => Some(d),
            AgainEnd::Reroll => Some(not_again_die.clone()),
            AgainEnd::Restart => None,
        };
        // `make_step_outcome`, "add_flat, add_not_again, add_again": an expression adds its value with `Again`
        // as zero, and leaves its count of `Again`s pending (the roll itself is done); `Reroll` and an empty die
        // drop out, so that roll is rerolled (as in `compute_not_again_die`; fixed upstream, see DIVERGENCES.md);
        // anything else ends a chain.
        let mut steps = Vec::with_capacity(outcomes.len());
        for outcome in &outcomes {
            steps.push(match outcome {
                AgainArg::Again(expr) => {
                    if !expr.is_additive() {
                        return Err(Error::InvalidArgument(
                            "again_count mode cannot be used with a non-additive AgainExpression."
                                .into(),
                        ));
                    }
                    let DieArg::Outcome(flat) =
                        expr.evaluate(&DieArg::Outcome(zero.clone()), &call, 0)?
                    else {
                        unreachable!("an additive typed expression of outcomes is an outcome")
                    };
                    DieArg::Outcome((flat, 0, again_count(expr) - 1))
                }
                AgainArg::Other(DieArg::Reroll) => DieArg::Reroll,
                AgainArg::Other(DieArg::Die(d)) if d.is_empty() => DieArg::Reroll,
                AgainArg::Other(_) => DieArg::Outcome((zero.clone(), 1, -1)),
            });
        }
        let step_die = Die::new(steps, times)?;
        let steps = step_die.items();
        let restart = again_end_die.is_none();
        let limit = count + 1;
        // `step(flat, not_again, again, index, roll)`.
        let step = |state: &CountState<O>, i: usize| -> Result<Mapped<CountState<O>>> {
            let (flat, not_again, again, index) = state;
            if *again == 0 || *index >= limit {
                return Ok(Mapped::Outcome(state.clone()));
            }
            let (add_flat, add_not_again, add_again) = &steps[i].0;
            let next = (
                flat.try_add(add_flat)?,
                not_again + add_not_again,
                again + add_again,
                index + 1,
            );
            if restart && next.3 as i128 + next.2 as i128 > limit as i128 {
                return Ok(Mapped::Restart);
            }
            Ok(Mapped::Outcome(next))
        };
        let quantities = steps.iter().map(|(_, q)| q.clone()).collect();
        let mut cache = TransitionCache::new(step, quantities);
        let initial = Die::single((zero, 0, 1, 0));
        let final_state = map_repeat(&mut cache, &initial, Some(limit))?;
        // `finalize`: `flat + not_again @ not_again_die (+ again @ again_end_die)`.
        let mut results = Vec::with_capacity(final_state.len());
        let mut quantities = Vec::with_capacity(final_state.len());
        for ((flat, not_again, again, _), quantity) in final_state.items() {
            let add = |a: &Die<O>, b: &Die<O>| a.binary_operator(b, O::try_add);
            let mut result = add(
                &Die::single(flat.clone()),
                &not_again_die.matmul(&[(*not_again, Quantity::ONE)])?,
            )?;
            if let Some(end) = &again_end_die {
                result = add(&result, &end.matmul(&[(*again, Quantity::ONE)])?)?;
            }
            results.push(DieArg::Die(result));
            quantities.push(quantity.clone());
        }
        Die::new(results, quantities)
    }
}

/// `_again_count`: the number of `Again`s in an additive expression (its sums applied to 1 for each `Again` and 0
/// for anything else).
fn again_count<O>(expr: &AgainExpression<AgainOp, DieArg<O>>) -> i64 {
    match expr {
        AgainExpression::Again { .. } => 1,
        AgainExpression::Apply { args, .. } => args
            .iter()
            .map(|arg| match arg {
                AgainArg::Again(a) => again_count(a),
                AgainArg::Other(_) => 0,
            })
            .sum(),
    }
}

/// An outcome given to `Die()` with `Again`, as the engine's argument.
fn outcome_or_again<O>(outcome: AgainOutcome<O>) -> AgainArg<AgainOp, DieArg<O>> {
    match outcome {
        AgainOutcome::Again(again) => AgainArg::Again(again.0),
        AgainOutcome::Outcome(o) => outcome_arg(o),
        AgainOutcome::Die(d) => AgainArg::Other(DieArg::Die(d)),
        AgainOutcome::Reroll => AgainArg::Other(DieArg::Reroll),
    }
}

/// The outcomes, none of them `Again`, as `Die()` takes them.
fn plain<O>(outcomes: Vec<AgainArg<AgainOp, DieArg<O>>>) -> Vec<DieArg<O>> {
    outcomes
        .into_iter()
        .filter_map(|x| match x {
            AgainArg::Other(x) => Some(x),
            AgainArg::Again(_) => None,
        })
        .collect()
}

/// `compute_not_again_die(outcomes, times)`: the die of the outcomes that are not `Again` expressions (nor
/// `Reroll`, nor an empty die: `outcome not in icepool.REROLL_TYPES` compares with `==`).
fn not_again_die<O: OutcomeArith>(
    outcomes: &[AgainArg<AgainOp, DieArg<O>>],
    times: &[Quantity],
) -> Result<Die<O>> {
    let (outcomes, times): (Vec<_>, Vec<_>) = outcomes
        .iter()
        .zip(times)
        .filter_map(|(outcome, quantity)| match outcome {
            AgainArg::Other(DieArg::Die(d)) if d.is_empty() => None,
            AgainArg::Other(DieArg::Reroll) | AgainArg::Again(_) => None,
            AgainArg::Other(x) => Some((x.clone(), quantity.clone())),
        })
        .unzip();
    if outcomes.is_empty() {
        // `icepool.Die(*zip(*filtered))` with nothing left calls `Die()`.
        return Err(Error::WrongType(
            "Die.__new__() missing 1 required positional argument: 'outcomes'".into(),
        ));
    }
    Die::new(outcomes, times)
}

/// `die.zero_outcome()`: `die.zero().outcomes()[0]`.
fn zero_outcome<O: OutcomeArith>(die: &Die<O>) -> Result<O> {
    Ok(die.zero()?.min_outcome()?.clone())
}

#[cfg(test)]
mod tests {
    //! Typed `Again` on `Die<i64>`. The expected dice are Python icepool's answers; the Python is in the comment
    //! above each.

    use super::*;
    use crate::d;

    /// A die from `(outcome, quantity)` pairs.
    fn die(items: &[(i64, u32)]) -> Die<i64> {
        Die::from_weights(items.iter().copied())
    }

    /// `die.map(lambda x: expr(x) if x == outcome else x, again_depth=depth, again_end=end)`.
    fn map_one(
        die: &Die<i64>,
        outcome: i64,
        expr: impl Fn() -> Again<i64>,
        depth: u64,
        end: AgainEnd<i64>,
    ) -> Result<Die<i64>> {
        die.map_again(
            |&x| {
                if x == outcome {
                    expr().into()
                } else {
                    x.into()
                }
            },
            depth,
            end,
        )
    }

    #[test]
    fn explode_with_each_kind_of_end() {
        // `d(6).explode(depth=2)`
        let twice = die(&[
            (1, 36),
            (2, 36),
            (3, 36),
            (4, 36),
            (5, 36),
            (7, 6),
            (8, 6),
            (9, 6),
            (10, 6),
            (11, 6),
            (13, 1),
            (14, 1),
            (15, 1),
            (16, 1),
            (17, 1),
            (18, 1),
        ]);
        assert_eq!(d(6).explode(|&x| x == 6, 2, AgainEnd::Zero).unwrap(), twice);
        // `d(6).explode([5, 6], depth=1, end=icepool.Reroll)`
        let rerolled = die(&[
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (6, 1),
            (7, 2),
            (8, 2),
            (9, 2),
            (10, 1),
        ]);
        assert_eq!(
            d(6).explode(|&x| x >= 5, 1, AgainEnd::Reroll).unwrap(),
            rerolled
        );
        // `d(4).explode(depth=1, end=d(2))`
        let to_d2 = die(&[
            (1, 8),
            (2, 8),
            (3, 8),
            (5, 2),
            (6, 2),
            (7, 2),
            (9, 1),
            (10, 1),
        ]);
        assert_eq!(
            d(4).explode(|&x| x == 4, 1, AgainEnd::Die(d(2))).unwrap(),
            to_d2
        );
        // `d(4).explode(lambda x: x == 1, depth=0)`
        assert_eq!(d(4).explode(|&x| x == 1, 0, AgainEnd::Zero).unwrap(), d(4));
    }

    #[test]
    fn operators_build_expressions_in_the_order_written() {
        // `d(6).map(lambda x: 2 * Again if x == 1 else x, again_depth=1)`
        let doubled = die(&[
            (0, 1),
            (2, 6),
            (3, 6),
            (4, 7),
            (5, 6),
            (6, 7),
            (8, 1),
            (10, 1),
            (12, 1),
        ]);
        assert_eq!(
            map_one(&d(6), 1, || 2i64 * Again::new(), 1, AgainEnd::Zero).unwrap(),
            doubled
        );
        // `d(3).map(lambda x: -Again if x == 1 else x, again_depth=2)`
        let negated = die(&[(-3, 3), (-2, 3), (0, 1), (2, 10), (3, 10)]);
        assert_eq!(
            map_one(&d(3), 1, || -Again::new(), 2, AgainEnd::Zero).unwrap(),
            negated
        );
        // `d(3).map(lambda x: Again - 1 if x == 3 else x, again_depth=1, again_end=10)`
        let less = die(&[(0, 1), (1, 4), (2, 3), (8, 1)]);
        let end = AgainEnd::Outcome(10);
        assert_eq!(
            map_one(&d(3), 3, || Again::new() - 1, 1, end).unwrap(),
            less
        );
        // `d(3).map(lambda x: (Again + 1) * (Again - 1) if x == 3 else x, again_depth=1)`: two separate rolls.
        let product = die(&[(-6, 1), (-4, 1), (0, 5), (1, 9), (2, 10), (3, 1)]);
        let expr = || (Again::new() + 1) * (Again::new() - 1);
        assert_eq!(map_one(&d(3), 3, expr, 1, AgainEnd::Zero).unwrap(), product);
    }

    #[test]
    fn again_beside_rerolls_and_without_plain_outcomes() {
        // `d(3).map(lambda x: 10 - Again if x == 3 else icepool.Reroll if x == 1 else x, again_depth=1)`
        let result = d(3).map_again(
            |&x| match x {
                3 => (10i64 - Again::new()).into(),
                1 => AgainOutcome::Reroll,
                x => x.into(),
            },
            1,
            AgainEnd::Zero,
        );
        assert_eq!(result.unwrap(), die(&[(0, 1), (2, 2), (8, 1)]));
        // `d(2).map(lambda x: Again + 1, again_depth=1)`: no outcome without `Again` to take the zero of.
        let all_again = |end| d(2).map_again(|_| (Again::new() + 1).into(), 1, end);
        assert!(matches!(
            all_again(AgainEnd::Zero),
            Err(Error::WrongType(_))
        ));
        // `d(2).map(lambda x: Again + 1, again_depth=1, again_end=0)`
        assert_eq!(all_again(AgainEnd::Outcome(0)).unwrap(), die(&[(2, 4)]));
    }

    /// `Die(outcomes, times, again_count=count, again_end=end)` of plain outcomes and an expression.
    fn count_mode(
        outcomes: Vec<AgainOutcome<i64>>,
        times: &[u32],
        count: u64,
        end: AgainEnd<i64>,
    ) -> Result<Die<i64>> {
        let outcomes = outcomes
            .into_iter()
            .zip(times.iter().map(|&t| Quantity::from(t)))
            .collect();
        Die::with_again_count(outcomes, count, end)
    }

    #[test]
    fn count_mode_with_each_kind_of_end() {
        let twice = || (3i64 + Again::new() + Again::new()).into();
        // `Die([1, 2, 3 + Again + Again], again_count=2, again_end=icepool.Restart)`
        let restarted = count_mode(
            vec![1.into(), 2.into(), twice()],
            &[1, 1, 1],
            2,
            AgainEnd::Restart,
        );
        assert_eq!(
            restarted.unwrap(),
            die(&[(1, 9), (2, 9), (5, 1), (6, 2), (7, 1)])
        );
        // ... `again_end=icepool.Reroll`
        let rerolled = count_mode(
            vec![1.into(), 2.into(), twice()],
            &[1, 1, 1],
            2,
            AgainEnd::Reroll,
        );
        let want = die(&[
            (1, 144),
            (2, 144),
            (5, 16),
            (6, 32),
            (7, 16),
            (9, 8),
            (10, 24),
            (11, 24),
            (12, 8),
            (13, 1),
            (14, 4),
            (15, 6),
            (16, 4),
            (17, 1),
        ]);
        assert_eq!(rerolled.unwrap(), want);
        // `Die([1, 2, 3 + Again + Again], again_count=3, again_end=10)`
        let ended = count_mode(
            vec![1.into(), 2.into(), twice()],
            &[1, 1, 1],
            3,
            AgainEnd::Outcome(10),
        );
        let want = die(&[
            (1, 27),
            (2, 27),
            (5, 3),
            (6, 6),
            (7, 3),
            (18, 2),
            (19, 4),
            (20, 2),
            (40, 3),
            (41, 3),
            (62, 1),
        ]);
        assert_eq!(ended.unwrap(), want);
        // `Die([1, Reroll, 3 + Again], [1, 1, 2], again_count=1, again_end=d(2))`: `Reroll` drops out of every
        // roll (fixed upstream; it used to end a chain as a step).
        let outcomes = vec![1.into(), AgainOutcome::Reroll, (3i64 + Again::new()).into()];
        let mixed = count_mode(outcomes, &[1, 1, 2], 1, AgainEnd::Die(d(2)));
        assert_eq!(mixed.unwrap(), die(&[(1, 3), (4, 2), (7, 2), (8, 2)]));
    }

    #[test]
    fn count_mode_agrees_with_depth_mode_for_one_again() {
        // `Die([1, 2, 3, 4, 5, 6 + Again], again_count=2)` is `d(6).explode(depth=2)`.
        let outcomes = (1..=5)
            .map(AgainOutcome::from)
            .chain([(6i64 + Again::new()).into()])
            .collect();
        let counted = count_mode(outcomes, &[1; 6], 2, AgainEnd::Zero).unwrap();
        assert_eq!(
            counted,
            d(6).explode(|&x| x == 6, 2, AgainEnd::Zero).unwrap()
        );
    }

    #[test]
    fn modes_refuse_what_python_refuses() {
        // `Die([1, 2 * Again], again_count=1)`, `Die([1, 2 * Again], again_depth=1, again_end=icepool.Restart)`
        let outcomes = || vec![1.into(), (2i64 * Again::new()).into()];
        let counted = count_mode(outcomes(), &[1, 1], 1, AgainEnd::Zero);
        assert!(matches!(counted, Err(Error::InvalidArgument(m)) if m.contains("non-additive")));
        let quantities =
            |o: Vec<AgainOutcome<i64>>| o.into_iter().map(|o| (o, Quantity::ONE)).collect();
        let depth = Die::with_again_depth(quantities(outcomes()), 1, AgainEnd::Restart);
        assert!(matches!(depth, Err(Error::InvalidArgument(m)) if m.contains("Restart")));
    }

    #[test]
    fn only_sums_of_agains_are_additive() {
        let additive = |a: Again<i64>| a.0.is_additive();
        assert!(additive(Again::new()));
        assert!(additive(1i64 + Again::new() + Again::new()));
        assert!(!additive(Again::new() - 1));
        assert!(!additive(2i64 * Again::new()));
        assert!(!additive(-Again::new() + 1));
    }
}
