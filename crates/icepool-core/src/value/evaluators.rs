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

use super::sort::sorted_reverse;
use super::{BinOp, Value};
use crate::error::{Error, Result};
use crate::evaluator::poker::{
    consecutive_span, AllCountsEvaluator, AllStraightsEvaluator, AllStraightsReduceCountsEvaluator,
    CountFilter, CountSubsetEvaluator, HighestOutcomeAndCountEvaluator, IntegerOutcome,
    LargestCountAndOutcomeEvaluator, LargestCountEvaluator, LargestCountWithWildEvaluator,
    LargestStraightAndOutcomeEvaluator, LargestStraightEvaluator, Truth,
};
use crate::evaluator::{tuple_compared_with_int, tuple_unsupported, Final, MultisetEvaluator};
use crate::order::Order;
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
        Value::tuple(vec![self.0.into_value(), self.1.into_value(), self.2.into_value()])
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
            return Err(Error::Type(
                "consecutive cannot be used with outcomes of type other than 'int'.".into(),
            ));
        }
        let (lo, hi) = (as_int(first).expect("checked"), as_int(last).expect("checked"));
        let span = consecutive_span(lo.clone(), hi)?;
        Ok((0..span).map(|i| Value::Int(&lo + IBig::from(i))).collect())
    }

    fn try_sub_int(&self, n: &Int) -> Result<Value> {
        self.binary(BinOp::Sub, &n.into())
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

/// An evaluator's `next_state` over tuple counts, as Python's arithmetic does it.
pub trait TupleCounts: MultisetEvaluator<Value> {
    /// `next_state` when some count is a tuple, every count as the Python value it is (`int` or `tuple` of
    /// `int`s). The default says `Unsupported`.
    fn next_state_tuples(
        &self,
        _state: &PyState<Self::State>,
        _order: Order,
        _outcome: &Value,
        _counts: &[Value],
    ) -> Result<Option<Value>> {
        Err(tuple_unsupported())
    }

    /// `final_outcome` of a state `next_state_tuples` produced.
    fn final_outcome_tuples(&self, _state: &Value) -> Result<Final<Value>> {
        Err(tuple_unsupported())
    }
}

/// A built-in evaluator as Python runs it: final outcomes as Python values, tuple counts through
/// [`TupleCounts`].
pub struct PyEvaluator<E>(pub E);

impl<E> MultisetEvaluator<Value> for PyEvaluator<E>
where
    E: TupleCounts,
    E::Final: IntoValue,
{
    type State = PyState<E::State>;
    type Final = Value;

    fn initial_state(&self, order: Order, outcomes: &[Value], sizes: &[Option<Int>]) -> Result<Self::State> {
        Ok(PyState::Typed(self.0.initial_state(order, outcomes, sizes)?))
    }

    fn next_state(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        match state {
            PyState::Typed(s) => Ok(self.0.next_state(s, order, outcome, counts)?.map(PyState::Typed)),
            // Once the state is Python's, it stays Python's.
            PyState::Python(_) => {
                let counts: Vec<Value> = counts.iter().map(Value::from).collect();
                self.next_state_values(state, order, outcome, &counts)
            }
        }
    }

    fn next_state_values(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<Self::State>> {
        Ok(self.0.next_state_tuples(state, order, outcome, counts)?.map(PyState::Python))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        self.0.extra_outcomes(outcomes)
    }

    fn reads_sizes(&self) -> bool {
        self.0.reads_sizes()
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
    fn next_state_tuples(&self, _: &PyState<Self::State>, _: Order, _: &Value, _: &[Value]) -> Result<Option<Value>> {
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
    ) -> Result<Option<Value>> {
        let count = &counts[0];
        // `state = state or ()`
        let state = state.value();
        let state = items(&state);
        if self.filter == Value::str("all") || count.binary(BinOp::Ge, &self.filter)?.truthy()? {
            let mut xs = state.to_vec();
            xs.push(count.clone());
            // `tuple(sorted(state, reverse=True))`
            Ok(Some(Value::tuple(sorted_reverse(xs)?)))
        } else {
            Ok(Some(Value::tuple(state.to_vec())))
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
    ) -> Result<Option<Value>> {
        // `max(state or count, count)`
        let (state, count) = (state.value(), counts[0].clone());
        let first = if state.truthy()? { state } else { count.clone() };
        Ok(Some(Value::py_max(first, count)?))
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
    ) -> Result<Option<Value>> {
        // `max(state or (count, outcome), (count, outcome))`
        let state = state.value();
        let pair = Value::tuple(vec![counts[0].clone(), outcome.clone()]);
        let first = if state.truthy()? { state } else { pair.clone() };
        Ok(Some(Value::py_max(first, pair)?))
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
    ) -> Result<Option<Value>> {
        let (left, right) = (&counts[0], &counts[1]);
        let state = state.value();
        if *right == Value::from(0) {
            return Ok(Some(state));
        }
        let current = left.binary(BinOp::FloorDiv, right)?;
        Ok(Some(match state {
            Value::None => current,
            state => Value::py_min(state, current)?,
        }))
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
    fn next_state_tuples(&self, _: &PyState<Self::State>, _: Order, _: &Value, _: &[Value]) -> Result<Option<Value>> {
        Err(tuple_compared_with_int(">="))
    }
}

/// `count >= 1` of a tuple count: `TypeError`.
impl TupleCounts for LargestStraightAndOutcomeEvaluator {
    fn next_state_tuples(&self, _: &PyState<Self::State>, _: Order, _: &Value, _: &[Value]) -> Result<Option<Value>> {
        Err(tuple_compared_with_int(">="))
    }
}

/// `count < len(current_runs)` of a tuple count: `TypeError`.
impl TupleCounts for AllStraightsEvaluator {
    fn next_state_tuples(&self, _: &PyState<Self::State>, _: Order, _: &Value, _: &[Value]) -> Result<Option<Value>> {
        Err(tuple_compared_with_int("<"))
    }
}

/// `count > 0` of a tuple count: `TypeError`.
impl<R: Fn(&Value, &Int) -> Result<Value>> TupleCounts for AllStraightsReduceCountsEvaluator<R, Value> {
    fn next_state_tuples(&self, _: &PyState<Self::State>, _: Order, _: &Value, _: &[Value]) -> Result<Option<Value>> {
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
    ) -> Result<Option<Value>> {
        let count = &counts[0];
        let (is_wild_early, is_wild_late) = self.wilds(order, outcome)?;
        let state = state.value();
        let [best, total_wild_early] = items(&state) else {
            // `state is None`
            let total_wild_early = if is_wild_early.truthy()? { count.clone() } else { Value::from(0) };
            return Ok(Some(Value::tuple(vec![count.clone(), total_wild_early])));
        };
        let (mut best, mut total_wild_early) = (best.clone(), total_wild_early.clone());
        if is_wild_late.truthy()? {
            best = best.binary(BinOp::Add, count)?;
        }
        best = Value::py_max(best, count.binary(BinOp::Add, &total_wild_early)?)?;
        if is_wild_early.truthy()? {
            total_wild_early = total_wild_early.binary(BinOp::Add, count)?;
        }
        Ok(Some(Value::tuple(vec![best, total_wild_early])))
    }

    fn final_outcome_tuples(&self, state: &Value) -> Result<Final<Value>> {
        match items(state) {
            // `final_state[0]`
            [best, _] => Ok(Final::Outcome(best.clone())),
            _ => Err(Error::Type("'NoneType' object is not subscriptable".into())),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::{evaluate, MultisetEvaluator};
    use crate::pool::MultisetSource;
    use crate::function::try_d;
    use crate::{Pool, Quantity};

    fn pool<O: crate::outcome::OutcomeOrder + From<i64>>(sides: i64, n: usize) -> MultisetSource<O> {
        MultisetSource::Pool(Pool::new(vec![(try_d(sides).unwrap(), n)]).unwrap().source())
    }

    /// Evaluates `evaluator` on `n`d`sides` typed (`i64` outcomes) and as Python does (`Value` outcomes): the
    /// typed finals, as Python spells them, are exactly Python's finals.
    fn typed_matches_python<E>(evaluator: E, sides: i64, n: usize) -> Vec<(<E as MultisetEvaluator<i64>>::Final, Quantity)>
    where
        E: MultisetEvaluator<i64> + TupleCounts,
        <E as MultisetEvaluator<i64>>::Final: IntoValue + Clone,
        <E as MultisetEvaluator<Value>>::Final: IntoValue,
    {
        let typed = evaluate(&evaluator, &[pool::<i64>(sides, n)]).unwrap();
        let python = evaluate(&PyEvaluator(evaluator), &[pool::<Value>(sides, n)]).unwrap();
        let spelled: Vec<_> = typed.iter().map(|(f, q)| (f.clone().into_value(), q.clone())).collect();
        assert_eq!(spelled, python);
        typed
    }

    #[test]
    fn typed_poker_evaluators_agree_with_python() {
        let mut largest: Vec<(Int, Quantity)> = typed_matches_python(LargestCountEvaluator, 6, 3);
        largest.sort_by(|a, b| a.0.cmp(&b.0));
        let expected = [(1, 120u32), (2, 90), (3, 6)].map(|(c, q)| (Int::from(c as i64), Quantity::from(q)));
        assert_eq!(largest, expected);
        typed_matches_python(HighestOutcomeAndCountEvaluator, 4, 3);
        typed_matches_python(LargestCountAndOutcomeEvaluator, 4, 3);
        typed_matches_python(LargestStraightEvaluator, 6, 4);
        typed_matches_python(LargestStraightAndOutcomeEvaluator { prioritize_highest: false }, 6, 3);
        typed_matches_python(AllStraightsEvaluator, 5, 3);
    }
}
