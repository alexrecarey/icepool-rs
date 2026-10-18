//! The built-in evaluators as Python runs them on [`Value`]s.
//!
//! The evaluators themselves are generic (`crate::evaluator`), with typed states and final outcomes. Python builds
//! its result die from the final outcomes *as Python values*, so [`PyEvaluator`] converts each final outcome
//! ([`IntoValue`]) before the die is built. And Python's evaluators have no special case for a tuple count (a
//! `MultiDeal` input): their arithmetic just runs on it (`outcome * count` repeats a tuple, `count >= 1` is
//! `TypeError`, `count == 0` is `False`...). That has no typed meaning, so it lives here: an evaluator's
//! [`TupleCounts`] runs `next_state` over `Value` counts and a `Value` state, which [`PyEvaluator`] switches to
//! at the first tuple count.

use dashu_int::IBig;

use super::sort::{sorted, sorted_reverse};
use super::{BinOp, Value};
use crate::error::{Error, Result};
use crate::evaluator::argsort::{ArgsortEvaluator, ArgsortGroupedEvaluator, RequestedOrder};
use crate::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, MappedSumEvaluator, ProductOfCountsEvaluator, SizeEvaluator,
    SumEvaluator, Summand,
};
use crate::evaluator::comparison::{step, Comparison, ComparisonEvaluator};
use crate::evaluator::keep::KeepEvaluator;
use crate::evaluator::lexi::{two, LexiComparisonEvaluator};
use crate::evaluator::poker::{
    consecutive_span, AllCountsEvaluator, AllStraightsEvaluator, AllStraightsReduceCountsEvaluator,
    CountFilter, CountSubsetEvaluator, HighestOutcomeAndCountEvaluator, IntegerOutcome,
    LargestCountAndOutcomeEvaluator, LargestCountEvaluator, LargestCountWithWildEvaluator,
    LargestStraightAndOutcomeEvaluator, LargestStraightEvaluator, Truth,
};
use crate::evaluator::{int_counts, tuple_unsupported, Final, MultisetEvaluator};
use crate::expression::count_value;
use crate::order::Order;
use crate::pool::Count;
use crate::Int;

/// A typed value as Python spells it.
pub trait IntoValue {
    /// The value as Python spells it.
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl IntoValue for Int {
    fn into_value(self) -> Value {
        self.into()
    }
}

impl IntoValue for i64 {
    fn into_value(self) -> Value {
        self.into()
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

/// `None`, or the value.
impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        self.map_or(Value::None, IntoValue::into_value)
    }
}

/// A tuple.
impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::tuple(self.into_iter().map(IntoValue::into_value).collect())
    }
}

impl<A: IntoValue, B: IntoValue> IntoValue for (A, B) {
    fn into_value(self) -> Value {
        Value::tuple(vec![self.0.into_value(), self.1.into_value()])
    }
}

impl<A: IntoValue, B: IntoValue, C: IntoValue> IntoValue for (A, B, C) {
    fn into_value(self) -> Value {
        Value::tuple(vec![
            self.0.into_value(),
            self.1.into_value(),
            self.2.into_value(),
        ])
    }
}

/// Python's truth value (`bool(x)`), which raises for some values (a `Vector`).
impl Truth for Value {
    fn truth(&self) -> Result<bool> {
        self.truthy()
    }
}

/// `self._filter == 'all' or count >= self._filter`, for any filter Python accepts.
impl CountFilter for Value {
    fn keeps(&self, count: &Int) -> Result<bool> {
        Ok(*self == Value::str("all") || Value::from(count).binary(BinOp::Ge, self)?.truthy()?)
    }
}

/// `int` outcomes (`bool` is an `int`); anything else is refused by `consecutive`.
impl IntegerOutcome for Value {
    /// `TypeError` if any input has any non-`int` outcome.
    fn consecutive(outcomes: &[Value]) -> Result<Vec<Value>> {
        let (Some(first), Some(last)) = (outcomes.first(), outcomes.last()) else {
            return Ok(Vec::new());
        };
        let as_int = |x: &Value| match x {
            Value::Int(i) => Some(i.clone()),
            Value::Bool(b) => Some(IBig::from(*b as u8)),
            _ => None,
        };
        if outcomes.iter().any(|x| as_int(x).is_none()) {
            return Err(Error::WrongType(
                "consecutive cannot be used with outcomes of type other than 'int'.".into(),
            ));
        }
        let (lo, hi) = (
            as_int(first).expect("checked"),
            as_int(last).expect("checked"),
        );
        let span = consecutive_span(lo.clone(), hi)?;
        Ok((0..span).map(|i| Value::Int(&lo + IBig::from(i))).collect())
    }

    fn try_sub_int(&self, n: &Int) -> Result<Value> {
        self.binary(BinOp::Sub, &n.into())
    }
}

/// Python's `None` is the one summand that makes a sum start over (`state is None`).
impl Summand for Value {
    fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }
}

/// A [`PyEvaluator`]'s state: the evaluator's own typed state until the first tuple count, then Python's.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum PyState<S> {
    /// The evaluator's own typed state.
    Typed(S),
    /// Python's state, once a tuple count arrived.
    Python(Value),
}

impl<S: Clone + IntoValue> PyState<S> {
    /// The state as Python spells it.
    pub fn value(&self) -> Value {
        match self {
            PyState::Typed(s) => s.clone().into_value(),
            PyState::Python(v) => v.clone(),
        }
    }
}

/// A Python state produced from tuple counts, as `next_state_tuples` returns it.
fn python<S>(state: Value) -> Result<Option<PyState<S>>> {
    Ok(Some(PyState::Python(state)))
}

/// The `TypeError` Python raises comparing a tuple count with an `int` (`count >= 1`; `max(count, 0)` compares
/// `0 > count`). A one-input evaluator whose `next_state` starts by such a comparison raises it for any tuple count
/// (its one count is the tuple whenever [`TupleCounts::next_state_tuples`] is called).
fn tuple_compared_with_int(op: &'static str) -> Error {
    Error::Incomparable {
        op,
        left: "tuple",
        right: "int",
    }
}

/// The sizes as `int`s, a tuple size as `None`: for evaluators that do not read it.
fn unread_tuple_sizes(sizes: &[Option<Count>]) -> Vec<Option<Int>> {
    sizes
        .iter()
        .map(|s| match s {
            Some(Count::Int(s)) => Some(s.clone()),
            Some(Count::Tuple(_)) | None => None,
        })
        .collect()
}

/// A built-in evaluator over tuple counts and sizes (a `MultiDeal` input, or tuple counts passed on by operators),
/// as Python's arithmetic runs on them.
pub trait TupleCounts: MultisetEvaluator<Value> {
    /// `initial_state` when some size may be a tuple. Python passes a tuple size on like any other; only keep and
    /// lexi read their sizes (and override this), so by default a tuple size reaches `initial_state` as `None`,
    /// unread. (No built-in `final_outcome` reads its sizes.)
    fn initial_state_tuples(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        self.initial_state(order, outcomes, &unread_tuple_sizes(sizes))
    }

    /// `next_state` when some count is a tuple (or the state is already Python's), every count as the Python
    /// value it is (`int` or `tuple` of `int`s). The default says `Unsupported`.
    fn next_state_tuples(
        &self,
        _state: &PyState<Self::State>,
        _order: Order,
        _outcome: &Value,
        _counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_unsupported())
    }

    /// `final_outcome` of a Python state `next_state_tuples` produced.
    fn final_outcome_tuples(&self, _state: &Value) -> Result<Final<Value>> {
        Err(tuple_unsupported())
    }
}

/// A built-in evaluator as Python runs it: final outcomes as Python values, tuple counts and sizes through
/// [`TupleCounts`].
pub struct PyEvaluator<E>(pub E);

impl<E> MultisetEvaluator<Value> for PyEvaluator<E>
where
    E: TupleCounts,
    E::Final: IntoValue,
{
    type State = PyState<E::State>;
    type Final = Value;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Int>],
    ) -> Result<Self::State> {
        Ok(PyState::Typed(
            self.0.initial_state(order, outcomes, sizes)?,
        ))
    }

    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        Ok(PyState::Typed(
            self.0.initial_state_tuples(order, outcomes, sizes)?,
        ))
    }

    fn next_state(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        match state {
            PyState::Typed(s) => Ok(self
                .0
                .next_state(s, order, outcome, counts)?
                .map(PyState::Typed)),
            // Once the state is Python's, it stays Python's.
            PyState::Python(_) => {
                let counts: Vec<Value> = counts.iter().map(Value::from).collect();
                self.0.next_state_tuples(state, order, outcome, &counts)
            }
        }
    }

    fn next_state_counts(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> Result<Option<Self::State>> {
        match int_counts(counts) {
            Some(counts) => self.next_state(state, order, outcome, &counts),
            None => {
                let counts: Vec<Value> = counts.iter().map(count_value).collect();
                self.0.next_state_tuples(state, order, outcome, &counts)
            }
        }
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        self.0.extra_outcomes(outcomes)
    }

    fn final_outcome(
        &self,
        state: &Self::State,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Int>],
    ) -> Result<Final<Value>> {
        match state {
            PyState::Typed(s) => Ok(match self.0.final_outcome(s, order, outcomes, sizes)? {
                Final::Outcome(f) => Final::Outcome(f.into_value()),
                Final::Reroll => Final::Reroll,
                Final::None => Final::None,
            }),
            PyState::Python(v) => self.0.final_outcome_tuples(v),
        }
    }

    fn final_outcome_counts(
        &self,
        state: &Self::State,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Final<Value>> {
        self.final_outcome(state, order, outcomes, &unread_tuple_sizes(sizes))
    }
}

/// The elements of a tuple state.
fn items(state: &Value) -> &[Value] {
    match state {
        Value::Tuple(t) => t,
        _ => &[],
    }
}

/// The default `final_outcome`: the final state itself (`None` is refused by the driver).
fn state_as_outcome(state: &Value) -> Final<Value> {
    match state {
        Value::None => Final::None,
        state => Final::Outcome(state.clone()),
    }
}

/// `max(count, 0)` of a tuple count: `TypeError`.
impl TupleCounts for HighestOutcomeAndCountEvaluator {
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int(">"))
    }
}

/// A tuple count is kept only with `filter='all'` (`count >= filter` is `TypeError` for an int filter), and sorting
/// tuples with ints is `TypeError`.
impl TupleCounts for AllCountsEvaluator<Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Vec<Int>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        let count = &counts[0];
        // `state = state or ()`
        let state = state.value();
        let state = items(&state);
        if self.filter == Value::str("all") || count.binary(BinOp::Ge, &self.filter)?.truthy()? {
            let mut xs = state.to_vec();
            xs.push(count.clone());
            // `tuple(sorted(state, reverse=True))`
            python(Value::tuple(sorted_reverse(xs)?))
        } else {
            python(Value::tuple(state.to_vec()))
        }
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(Final::Outcome(state.clone()))
    }
}

/// A tuple count is the state while it is the largest (tuples compare lexicographically, `()` is falsy), and
/// `TypeError` against an `int` one.
impl TupleCounts for LargestCountEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<Int>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        // `max(state or count, count)`
        let (state, count) = (state.value(), counts[0].clone());
        let first = if state.truthy()? {
            state
        } else {
            count.clone()
        };
        python(Value::py_max(first, count)?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(state_as_outcome(state))
    }
}

/// The pairs compare by count first (lexicographically for tuple counts, `TypeError` between a tuple and an
/// `int`), then by outcome.
impl TupleCounts for LargestCountAndOutcomeEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<(Int, Value)>>,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        // `max(state or (count, outcome), (count, outcome))`
        let state = state.value();
        let pair = Value::tuple(vec![counts[0].clone(), outcome.clone()]);
        let first = if state.truthy()? { state } else { pair.clone() };
        python(Value::py_max(first, pair)?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(state_as_outcome(state))
    }
}

/// A tuple `right` is not `== 0`, and `//` with a tuple is `TypeError`.
impl TupleCounts for CountSubsetEvaluator<Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<Int>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        let (left, right) = (&counts[0], &counts[1]);
        let state = state.value();
        if *right == Value::from(0) {
            return python(state);
        }
        let current = left.binary(BinOp::FloorDiv, right)?;
        python(match state {
            Value::None => current,
            state => Value::py_min(state, current)?,
        })
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        self.final_outcome_of(match state {
            Value::None => None,
            state => Some(state.clone()),
        })
    }
}

/// `count >= 1` of a tuple count: `TypeError`.
impl TupleCounts for LargestStraightEvaluator {
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int(">="))
    }
}

/// `count >= 1` of a tuple count: `TypeError`.
impl TupleCounts for LargestStraightAndOutcomeEvaluator {
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int(">="))
    }
}

/// `count < len(current_runs)` of a tuple count: `TypeError`.
impl TupleCounts for AllStraightsEvaluator {
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int("<"))
    }
}

/// `count > 0` of a tuple count: `TypeError`.
impl<R: Fn(&Value, &Int) -> Result<Value>> TupleCounts
    for AllStraightsReduceCountsEvaluator<R, Value>
{
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int(">"))
    }
}

/// Python's `+` and `max` on the counts, so tuple counts concatenate (`count + 0` is `TypeError`).
impl<S: Fn(&Value) -> Result<Value>> TupleCounts for LargestCountWithWildEvaluator<S, Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<(Int, Int)>>,
        order: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        let count = &counts[0];
        let (is_wild_early, is_wild_late) = self.wilds(order, outcome)?;
        let state = state.value();
        let [best, total_wild_early] = items(&state) else {
            // `state is None`
            let total_wild_early = if is_wild_early.truthy()? {
                count.clone()
            } else {
                Value::from(0)
            };
            return python(Value::tuple(vec![count.clone(), total_wild_early]));
        };
        let (mut best, mut total_wild_early) = (best.clone(), total_wild_early.clone());
        if is_wild_late.truthy()? {
            best = best.binary(BinOp::Add, count)?;
        }
        best = Value::py_max(best, count.binary(BinOp::Add, &total_wild_early)?)?;
        if is_wild_early.truthy()? {
            total_wild_early = total_wild_early.binary(BinOp::Add, count)?;
        }
        python(Value::tuple(vec![best, total_wild_early]))
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        match items(state) {
            // `final_state[0]`
            [best, _] => Ok(Final::Outcome(best.clone())),
            _ => Err(Error::WrongType(
                "'NoneType' object is not subscriptable".into(),
            )),
        }
    }
}

/// `SumEvaluator.next_state` after the (optional) map, with Python's operators: `count == 1` is `False` for a tuple
/// count, an `int` outcome times it repeats the tuple, and `state + ...` concatenates.
fn add_to_sum(state: Value, outcome: Value, count: &Value) -> Result<Value> {
    Ok(match state {
        Value::None if *count == Value::from(1) => outcome,
        Value::None => outcome.binary(BinOp::Mul, count)?,
        state => state.binary(BinOp::Add, &outcome.binary(BinOp::Mul, count)?)?,
    })
}

impl TupleCounts for SumEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<Value>>,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        python(add_to_sum(state.value(), outcome.clone(), &counts[0])?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(state_as_outcome(state))
    }
}

/// As [`SumEvaluator`], on the mapped outcome.
impl<F: Fn(&Value) -> Result<Value>> TupleCounts for MappedSumEvaluator<F, Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<Value>>,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        python(add_to_sum(state.value(), (self.map)(outcome)?, &counts[0])?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(state_as_outcome(state))
    }
}

/// `state + count`: `0 + tuple` is `TypeError`.
impl TupleCounts for SizeEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Int>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        python(state.value().binary(BinOp::Add, &counts[0])?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        // `final_state or 0`
        Ok(Final::Outcome(if state.truthy()? {
            state.clone()
        } else {
            Value::from(0)
        }))
    }
}

/// `state * count`: `1 * tuple` is the tuple, then a tuple times a tuple is `TypeError` (and times an `int`
/// repeats it).
impl TupleCounts for ProductOfCountsEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Int>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        python(state.value().binary(BinOp::Mul, &counts[0])?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(state_as_outcome(state))
    }
}

/// A tuple count: `count == 0` is `False`.
impl TupleCounts for EmptyEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<bool>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        // `state and (count == 0)`
        let state = state.value();
        python(if state.truthy()? {
            Value::Bool(counts[0] == Value::from(0))
        } else {
            state
        })
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(Final::Outcome(Value::Bool(self.final_of(state.truthy()?))))
    }
}

/// A tuple count: `(outcome, ) * count` is `TypeError` (a tuple times a tuple).
impl TupleCounts for ExpandEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Vec<Value>>,
        _: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        // `(state or ()) + (outcome, ) * count`; the typed state is the tuple, `()` where Python has `None`.
        let more = Value::tuple(vec![outcome.clone()]).binary(BinOp::Mul, &counts[0])?;
        python(state.value().binary(BinOp::Add, &more)?)
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        let items = items(state).to_vec();
        let sorted = if self.ascending {
            sorted(items)?
        } else {
            sorted_reverse(items)?
        };
        Ok(Final::Outcome(Value::tuple(sorted)))
    }
}

/// `max(count, 0)` of a tuple count: `TypeError`.
impl TupleCounts for KeepEvaluator {
    /// Python passes a tuple size on to `initial_state`, which reads it only to count from the other end
    /// (`size + index + 1` / `size - index`: `TypeError` for a tuple).
    fn initial_state_tuples(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        let size = match &sizes[0] {
            Some(Count::Int(size)) => Some(size.clone()),
            Some(Count::Tuple(_)) => {
                if matches!(&self.index, Some(index) if (order.value() > 0) != !index.is_negative())
                {
                    let op = if order.value() > 0 { "+" } else { "-" };
                    return Err(Error::WrongType(format!(
                        "unsupported operand type(s) for {op}: 'tuple' and 'int'"
                    )));
                }
                None
            }
            None => None,
        };
        self.initial_state(order, outcomes, &[size])
    }

    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        _: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        Err(tuple_compared_with_int(">"))
    }
}

/// `order != self._order` with whatever the caller passed as `order=`.
impl RequestedOrder for Value {
    fn differs_from(&self, order: Order) -> Result<bool> {
        Value::from(order.value()).eq_op(self, true)?.truthy()
    }
}

/// The state as it was before any tuple count: argsort and lexi have no Python state (a tuple count either
/// raises or leaves the typed state, see their `next_state_tuples`).
fn typed<S>(state: &PyState<S>) -> &S {
    match state {
        PyState::Typed(s) => s,
        PyState::Python(_) => unreachable!("argsort and lexi states stay typed"),
    }
}

/// Python's arithmetic on a tuple count: drop compares `count > max_count` (`TypeError`) unless the limit is
/// reached, so it never looks at the counts; left / right take each count in turn, `min(count, limit -
/// len(state))` (`TypeError`) or the count itself, then `append_count > 0` (`TypeError`), and may return early
/// once the limit is reached.
impl TupleCounts for ArgsortEvaluator<Value, Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Vec<i64>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Vec<i64>>>> {
        let limit = self.limit;
        let counts = counts
            .iter()
            .map(|count| match count {
                Value::Int(c) => Ok(Int::from(c)),
                // `min(count, limit - len(state))` compares `limit - len(state) < count`.
                _ if limit.is_some() => Err(Error::Incomparable {
                    op: "<",
                    left: "int",
                    right: "tuple",
                }),
                _ => Err(tuple_compared_with_int(">")),
            })
            .collect();
        let state = self.step(typed(state).clone(), counts)?;
        Ok(Some(PyState::Typed(state)))
    }
}

/// A tuple count: `any(count > 0 for count in counts)` short-circuits on an earlier positive `int` count, and then
/// `(i, ) * count` of the tuple raises; else `count > 0` does. Both only if the limit is not reached.
impl TupleCounts for ArgsortGroupedEvaluator<Value> {
    fn next_state_tuples(
        &self,
        state: &PyState<Vec<Vec<i64>>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Vec<Vec<i64>>>>> {
        if !self.below_limit(typed(state)) {
            return Ok(Some(state.clone()));
        }
        let positive_before_tuple = counts
            .iter()
            .take_while(|count| matches!(count, Value::Int(_)))
            .any(|count| matches!(count, Value::Int(c) if Int::from(c).is_positive()));
        Err(if positive_before_tuple {
            Error::WrongType("can't multiply sequence by non-int of type 'tuple'".into())
        } else {
            tuple_compared_with_int(">")
        })
    }
}

impl TupleCounts for LexiComparisonEvaluator {
    /// Python passes tuple sizes on to `initial_state`, which reads them only in reverse order: after the `None`
    /// check, `left_size < right_size` (`TypeError` between a tuple and an `int`) and then `right_size -
    /// left_size` (`TypeError` between two tuples).
    fn initial_state_tuples(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        let (left_size, right_size) = two(sizes, "initial_state")?;
        let int = |size: Option<Count>| match size {
            Some(Count::Int(size)) => Ok(Some(size)),
            None => Ok(None),
            Some(Count::Tuple(_)) => Err(()),
        };
        match (int(left_size), int(right_size)) {
            (Ok(left_size), Ok(right_size)) => {
                self.initial_state(order, outcomes, &[left_size, right_size])
            }
            // Forward order reads no sizes.
            _ if order == self.sort_order => self.initial_state(order, outcomes, &[None, None]),
            (Ok(None), _) | (_, Ok(None)) => self.initial_state(order, outcomes, &[None, None]),
            (Err(()), Err(())) => Err(Error::WrongType(
                "unsupported operand type(s) for -: 'tuple' and 'tuple'".into(),
            )),
            (left, _) => Err(Error::Incomparable {
                op: "<",
                left: if left.is_ok() { "int" } else { "tuple" },
                right: if left.is_ok() { "tuple" } else { "int" },
            }),
        }
    }

    /// `left = max(left, 0)` of a tuple compares `0 > left`: `TypeError` (the int count, if any, is fine).
    fn next_state_tuples(
        &self,
        _: &PyState<Self::State>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        two(counts, "next_state")?;
        Err(Error::Incomparable {
            op: ">",
            left: "int",
            right: "tuple",
        })
    }
}

/// [`ComparisonEvaluator::any_all`] for counts that may be tuples, with Python's operators (`<` between tuples is
/// lexicographic, giving a bool for tuples of ints; between a tuple and an int `TypeError`; `==` just `False`).
/// Python evaluates the pair left to right, and `left > 0 and right > 0` stops at a false left.
fn any_all_values(comparison: Comparison, left: &Value, right: &Value) -> Result<(bool, bool)> {
    use BinOp::{Ge, Gt, Le, Lt};
    use Comparison::*;
    let cmp = |op, a: &Value, b: &Value| a.binary(op, b)?.truthy();
    let zero = Value::from(0);
    Ok(match comparison {
        IsProperSubset => (cmp(Lt, left, right)?, cmp(Le, left, right)?),
        IsSubset => (true, cmp(Le, left, right)?),
        IsProperSuperset => (cmp(Gt, left, right)?, cmp(Ge, left, right)?),
        IsSuperset => (true, cmp(Ge, left, right)?),
        IsEqualSet => (true, left == right),
        IsNotEqualSet => (left != right, true),
        IsDisjointSet => (true, !(cmp(Gt, left, &zero)? && cmp(Gt, right, &zero)?)),
    })
}

/// A comparison's state as Python spells it (`None` or `(has_any, has_all)`), typed again.
fn comparison_state(state: &Value) -> Option<(bool, bool)> {
    match items(state) {
        [Value::Bool(has_any), Value::Bool(has_all)] => Some((*has_any, *has_all)),
        _ => None,
    }
}

/// The comparisons of tuple counts are Python's (see `any_all_values`); the state stays a pair of bools.
impl TupleCounts for ComparisonEvaluator {
    fn next_state_tuples(
        &self,
        state: &PyState<Option<(bool, bool)>>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<PyState<Self::State>>> {
        let this = any_all_values(self.0, &counts[0], &counts[1])?;
        python(step(comparison_state(&state.value()), this).into_value())
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        Ok(Final::Outcome(Value::Bool(
            self.final_of(comparison_state(state)),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::argsort::Tie;
    use crate::evaluator::{evaluate, MultisetEvaluator};
    use crate::function::try_d;
    use crate::pool::MultisetSource;
    use crate::{Pool, Quantity};

    /// `keep.len()` d`sides`, counted as `keep` says (one count per sorted roll; negative counts too).
    fn kept<O: crate::outcome::OutcomeOrder + From<i64>>(
        sides: i64,
        keep: &[i64],
    ) -> MultisetSource<O> {
        let keep: Vec<Int> = keep.iter().map(|&k| Int::from(k)).collect();
        MultisetSource::Pool(
            Pool::from_parts(&[(try_d(sides).unwrap(), keep.len())], keep.into())
                .unwrap()
                .source(),
        )
    }

    /// Evaluates `typed` on `i64` outcomes and `PyEvaluator(python)` on `Value` outcomes, each on the pools
    /// `pools` builds: the typed finals, as Python spells them, are exactly Python's finals.
    fn typed_matches_python_with<T, P>(
        typed: &T,
        python: P,
        pools: impl Fn() -> (Vec<MultisetSource<i64>>, Vec<MultisetSource<Value>>),
    ) -> Vec<(T::Final, Quantity)>
    where
        T: MultisetEvaluator<i64>,
        T::Final: IntoValue + Clone,
        P: TupleCounts,
        P::Final: IntoValue,
    {
        let (typed_pools, python_pools) = pools();
        let typed = evaluate(typed, &typed_pools).unwrap();
        let python = evaluate(&PyEvaluator(python), &python_pools).unwrap();
        let spelled: Vec<_> = typed
            .iter()
            .map(|(f, q)| (f.clone().into_value(), q.clone()))
            .collect();
        assert_eq!(spelled, python);
        typed
    }

    /// [`typed_matches_python_with`] for an evaluator of any outcome type on `keep.len()`d`sides` kept as `keep`.
    fn typed_matches_kept<E>(
        evaluator: E,
        sides: i64,
        keep: &[i64],
    ) -> Vec<(<E as MultisetEvaluator<i64>>::Final, Quantity)>
    where
        E: MultisetEvaluator<i64> + TupleCounts,
        <E as MultisetEvaluator<i64>>::Final: IntoValue + Clone,
        <E as MultisetEvaluator<Value>>::Final: IntoValue,
    {
        // The evaluator has no state of its own between evaluations, so the typed run can borrow it first.
        let typed = evaluate(&evaluator, &[kept::<i64>(sides, keep)]).unwrap();
        let python = evaluate(&PyEvaluator(evaluator), &[kept::<Value>(sides, keep)]).unwrap();
        let spelled: Vec<_> = typed
            .iter()
            .map(|(f, q)| (f.clone().into_value(), q.clone()))
            .collect();
        assert_eq!(spelled, python);
        typed
    }

    /// [`typed_matches_kept`] on `n`d`sides`.
    fn typed_matches_python<E>(
        evaluator: E,
        sides: i64,
        n: usize,
    ) -> Vec<(<E as MultisetEvaluator<i64>>::Final, Quantity)>
    where
        E: MultisetEvaluator<i64> + TupleCounts,
        <E as MultisetEvaluator<i64>>::Final: IntoValue + Clone,
        <E as MultisetEvaluator<Value>>::Final: IntoValue,
    {
        typed_matches_kept(evaluator, sides, &vec![1; n])
    }

    #[test]
    fn typed_poker_evaluators_agree_with_python() {
        let mut largest: Vec<(Int, Quantity)> = typed_matches_python(LargestCountEvaluator, 6, 3);
        largest.sort_by(|a, b| a.0.cmp(&b.0));
        let expected =
            [(1, 120u32), (2, 90), (3, 6)].map(|(c, q)| (Int::from(c as i64), Quantity::from(q)));
        assert_eq!(largest, expected);
        typed_matches_python(HighestOutcomeAndCountEvaluator, 4, 3);
        typed_matches_python(LargestCountAndOutcomeEvaluator, 4, 3);
        typed_matches_python(LargestStraightEvaluator, 6, 4);
        typed_matches_python(
            LargestStraightAndOutcomeEvaluator {
                prioritize_highest: false,
            },
            6,
            3,
        );
        typed_matches_python(AllStraightsEvaluator, 5, 3);
    }

    #[test]
    fn typed_basic_evaluators_agree_with_python() {
        // Negative and zero counts as well as ordinary ones.
        for keep in [&[1, 1, 1][..], &[-1, 0, 2], &[], &[0, 0]] {
            typed_matches_kept(SumEvaluator, 4, keep);
            typed_matches_kept(SizeEvaluator, 4, keep);
            typed_matches_kept(ProductOfCountsEvaluator, 4, keep);
            typed_matches_kept(EmptyEvaluator { any: false }, 4, keep);
            typed_matches_kept(EmptyEvaluator { any: true }, 4, keep);
            typed_matches_kept(ExpandEvaluator { ascending: true }, 4, keep);
            typed_matches_kept(ExpandEvaluator { ascending: false }, 4, keep);
        }
        // `d(6).pool(3).sum()`: 3..=18.
        let sums: Vec<(i64, Quantity)> = typed_matches_python(SumEvaluator, 6, 3);
        assert_eq!(sums.len(), 16);
        typed_matches_python_with(
            &MappedSumEvaluator::new(|o: &i64| Ok(o * 3 - 7)),
            MappedSumEvaluator::new(|o: &Value| {
                o.binary(BinOp::Mul, &Value::from(3))?
                    .binary(BinOp::Sub, &Value::from(7))
            }),
            || (vec![kept(5, &[1, -2, 1])], vec![kept(5, &[1, -2, 1])]),
        );
    }

    #[test]
    fn a_sum_mapped_to_none_starts_over() {
        // `Die([1, 2]).pool(1).sum(map={1: None})`: `None * 0` is `TypeError` once 1 was not rolled.
        let python = PyEvaluator(MappedSumEvaluator::new(|o: &Value| {
            Ok(if *o == Value::from(1) {
                Value::None
            } else {
                o.clone()
            })
        }));
        let error = evaluate(&python, &[kept::<Value>(2, &[1])]).err();
        assert!(matches!(error, Some(Error::WrongType(_))), "{error:?}");
        // `Die([1]).pool(1).sum(map={1: None})`: the final state is `None`, which no die has as an outcome.
        assert!(evaluate(&python, &[kept::<Value>(1, &[1])]).is_err());
    }

    #[test]
    fn typed_keep_evaluator_agrees_with_python() {
        for index in [None, Some(0), Some(1), Some(-1), Some(-3)] {
            let index = index.map(Int::from);
            let keep = |index: &Option<Int>| KeepEvaluator {
                index: index.clone(),
            };
            typed_matches_kept(keep(&index), 4, &[1, 1, 1]);
            // A count of -1 may cancel an element of the same outcome: only the first two are always there.
            if index
                .as_ref()
                .is_none_or(|i| *i == Int::ZERO || *i == Int::from(-1))
            {
                typed_matches_kept(keep(&index), 4, &[1, -1, 2]);
            }
        }
        // Too few elements: `IndexError`, typed and as Python.
        let keep = || KeepEvaluator {
            index: Some(Int::from(5)),
        };
        let short = |e: Error| matches!(e, Error::IndexOutOfRange(_));
        assert!(evaluate(&keep(), &[kept::<i64>(4, &[1, 1])]).is_err_and(short));
        assert!(evaluate(&PyEvaluator(keep()), &[kept::<Value>(4, &[1, 1])]).is_err_and(short));
    }

    #[test]
    fn typed_comparison_evaluators_agree_with_python() {
        use Comparison::*;
        for comparison in [
            IsProperSubset,
            IsSubset,
            IsProperSuperset,
            IsSuperset,
            IsEqualSet,
            IsNotEqualSet,
            IsDisjointSet,
        ] {
            for (left, right) in [(&[1, 1][..], &[1][..]), (&[1, -1], &[0, 2]), (&[], &[])] {
                typed_matches_python_with(
                    &ComparisonEvaluator(comparison),
                    ComparisonEvaluator(comparison),
                    || {
                        (
                            vec![kept(3, left), kept(3, right)],
                            vec![kept(3, left), kept(3, right)],
                        )
                    },
                );
            }
        }
    }

    /// Two or three pools, kept as the slices say, for the multi-input evaluators.
    fn pools(
        sides: i64,
        keeps: &[&[i64]],
    ) -> (Vec<MultisetSource<i64>>, Vec<MultisetSource<Value>>) {
        (
            keeps.iter().map(|k| kept(sides, k)).collect(),
            keeps.iter().map(|k| kept(sides, k)).collect(),
        )
    }

    #[test]
    fn typed_argsort_evaluators_agree_with_python() {
        let inputs: [&[&[i64]]; 4] = [
            &[&[1, 1], &[1]],
            &[&[1, 1], &[1, 1], &[1]],
            &[&[1, -1, 2], &[0, 1]],
            &[&[], &[1]],
        ];
        for keeps in inputs {
            for tie in [Tie::Drop, Tie::Left, Tie::Right] {
                for (limit, pad) in [
                    (None, None),
                    (Some(2), None),
                    (Some(4), Some(-1)),
                    (Some(0), Some(7)),
                ] {
                    typed_matches_python_with(
                        &ArgsortEvaluator {
                            tie,
                            order: Order::Descending,
                            limit,
                            limit_pad: pad.map(Int::from),
                        },
                        ArgsortEvaluator {
                            tie,
                            order: Value::from(Order::Descending.value()),
                            limit,
                            limit_pad: pad.map(Value::from),
                        },
                        || pools(3, keeps),
                    );
                }
            }
            for limit in [None, Some(1), Some(0)] {
                typed_matches_python_with(
                    &ArgsortGroupedEvaluator {
                        order: Order::Descending,
                        limit,
                    },
                    ArgsortGroupedEvaluator {
                        order: Value::from(Order::Descending.value()),
                        limit,
                    },
                    || pools(3, keeps),
                );
            }
        }
        // `order=Order.Ascending` is evaluated in ascending order, typed and as Python.
        let ascending: Vec<(Vec<Int>, Quantity)> = typed_matches_python_with(
            &ArgsortEvaluator {
                tie: Tie::Left,
                order: Order::Ascending,
                limit: None,
                limit_pad: None,
            },
            ArgsortEvaluator {
                tie: Tie::Left,
                order: Value::from(Order::Ascending.value()),
                limit: None,
                limit_pad: None::<Value>,
            },
            || pools(2, &[&[1], &[1]]),
        );
        // d2 vs d2 (`icepool.d2.pool(1).argsort(icepool.d2.pool(1), order=Order.Ascending, tie='left')`): the
        // right one alone is lowest only for (2, 1); a tie ranks the left one first.
        let mut ascending: Vec<_> = ascending
            .into_iter()
            .map(|(f, q)| (f, q.to_string()))
            .collect();
        ascending.sort();
        let expected = [(vec![0, 1], "3"), (vec![1, 0], "1")].map(|(f, q)| {
            (
                f.into_iter().map(Int::from).collect::<Vec<_>>(),
                q.to_string(),
            )
        });
        assert_eq!(ascending, expected);
    }

    #[test]
    fn typed_lexi_evaluator_agrees_with_python() {
        for sort_order in [Order::Ascending, Order::Descending] {
            for keeps in [
                &[&[1, 1][..], &[1, 1]][..],
                &[&[1, 1, 1], &[1, 1]],
                &[&[1], &[1, 1, 1]],
                &[&[1, -1, 1], &[0, 2]],
            ] {
                let lexi = || LexiComparisonEvaluator {
                    sort_order,
                    lexi_tuple: (1, 2, 3, 4, 5),
                };
                typed_matches_python_with(&lexi(), lexi(), || pools(3, keeps));
            }
        }
    }

    #[test]
    fn a_typed_evaluator_refuses_tuple_counts() {
        use crate::deck::Deck;
        use crate::multi_deal::MultiDeal;
        // Two hands of one card from {1, 2, 3}: every count is a tuple.
        let deal = |hands| {
            let items = (1..=3).map(|o| (o, Quantity::ONE)).collect();
            let deck: Deck<i64> = Deck::from_dict(items).unwrap();
            MultiDeal::new(deck, hands).unwrap()
        };
        let hands = [
            (IBig::from(1), IBig::from(1)),
            (IBig::from(1), IBig::from(1)),
        ];
        let source = MultisetSource::MultiDeal(deal(&hands).source());
        let error = evaluate(&SumEvaluator, std::slice::from_ref(&source)).err();
        assert!(matches!(error, Some(Error::Unported(_))), "{error:?}");
        // Python's `sum` of a tuple count is Python's arithmetic: `outcome * count` repeats the tuple.
        let deal = |hands| {
            let items = (1..=3).map(|o| (Value::from(o), Quantity::ONE)).collect();
            MultiDeal::new(Deck::from_dict(items).unwrap(), hands).unwrap()
        };
        let source = MultisetSource::MultiDeal(deal(&hands).source());
        assert!(evaluate(&PyEvaluator(SumEvaluator), &[source]).is_ok());
    }
}
