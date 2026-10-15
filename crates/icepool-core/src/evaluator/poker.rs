//! Port of icepool `evaluator/poker.py`: evaluators for poker/Yahtzee-like mechanics.

use dashu_int::IBig;

use super::basic::{int, py_sorted_reverse, state_as_outcome};
use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::order::Order;
use crate::value::BinOp;
use crate::{Int, Value};

/// More consecutive outcomes than this and the port refuses (`range` of a huge span: Python would take minutes).
const MAX_CONSECUTIVE: usize = 1 << 20;

/// `MultisetEvaluator.consecutive`: all `int`s from the min outcome to the max outcome among the inputs,
/// inclusive. `TypeError` if any input has any non-`int` outcome (a `bool` is an `int`).
pub fn consecutive(outcomes: &[Value]) -> Result<Vec<Value>> {
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
    let (lo, hi) = (
        as_int(first).expect("checked"),
        as_int(last).expect("checked"),
    );
    let span = usize::try_from(&hi - &lo + IBig::ONE).unwrap_or(0);
    if span > MAX_CONSECUTIVE {
        return Err(Error::Unsupported(
            "consecutive outcomes over a huge range".into(),
        ));
    }
    Ok((0..span).map(|i| Value::Int(&lo + IBig::from(i))).collect())
}

/// The elements of a tuple state.
fn items(state: &Value) -> &[Value] {
    match state {
        Value::Tuple(t) => t,
        _ => &[],
    }
}

/// An int inside a state (all of them are made from counts here).
fn as_count(x: &Value) -> Int {
    match x {
        Value::Int(i) => i.into(),
        _ => unreachable!("count states hold ints"),
    }
}

fn int_tuple(xs: impl IntoIterator<Item = Int>) -> Value {
    Value::tuple(xs.into_iter().map(int).collect())
}

/// `tuple(sorted(xs, reverse=True))` for ints.
fn sorted_descending(mut xs: Vec<Int>) -> Vec<Int> {
    xs.sort_unstable_by(|a, b| b.cmp(a));
    xs
}

/// The highest outcome that has positive count, along with that count (`HighestOutcomeAndCountEvaluator`).
///
/// If no outcomes have positive count, the result is the min outcome with a count of 0.
pub struct HighestOutcomeAndCountEvaluator;

impl MultisetEvaluator<Value> for HighestOutcomeAndCountEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let count = counts[0].clone().max(Int::ZERO);
        let pair = |o: Value, c: Int| Ok(Some(Value::tuple(vec![o, int(c)])));
        let Value::Tuple(s) = state else {
            return pair(outcome.clone(), count);
        };
        if count.is_positive() && outcome.binary(BinOp::Gt, &s[0])?.truthy()? {
            return pair(outcome.clone(), count);
        }
        if count.is_zero() && s[1] == int(0) {
            return pair(Value::py_min(outcome.clone(), s[0].clone())?, Int::ZERO);
        }
        Ok(Some(state.clone()))
    }

    /// Always sees zero counts.
    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        Ok(outcomes.to_vec())
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// All counts in descending order (`AllCountsEvaluator`). In other words, this produces tuples of the sizes of all
/// matching sets.
pub struct AllCountsEvaluator {
    /// Any counts below this value will not be in the output. For example, `filter=2` will only produce pairs and
    /// better. If `'all'`, no filtering will be done.
    pub filter: Value,
}

impl MultisetEvaluator<Value> for AllCountsEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let count = counts[0].clone();
        // `state = state or ()`
        let state = items(state);
        if self.filter == Value::str("all")
            || int(&count).binary(BinOp::Ge, &self.filter)?.truthy()?
        {
            let mut xs: Vec<Int> = state.iter().map(as_count).collect();
            xs.push(count);
            Ok(Some(int_tuple(sorted_descending(xs))))
        } else {
            Ok(Some(Value::tuple(state.to_vec())))
        }
    }

    /// Always sees zero counts.
    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        Ok(outcomes.to_vec())
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(Final::Outcome(match final_state {
            Value::None => Value::tuple(Vec::new()),
            state => state.clone(),
        }))
    }
}

/// The largest count of any outcome (`LargestCountEvaluator`).
pub struct LargestCountEvaluator;

impl MultisetEvaluator<Value> for LargestCountEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        // `max(state or count, count)`: a state of 0 is falsy too, which matters for negative counts.
        let count = int(&counts[0]);
        let first = if state.truthy()? {
            state.clone()
        } else {
            count.clone()
        };
        Ok(Some(Value::py_max(first, count)?))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// The largest count of any outcome, along with that outcome (`LargestCountAndOutcomeEvaluator`).
pub struct LargestCountAndOutcomeEvaluator;

impl MultisetEvaluator<Value> for LargestCountAndOutcomeEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        // `max(state or (count, outcome), (count, outcome))`
        let pair = Value::tuple(vec![int(&counts[0]), outcome.clone()]);
        let first = if state.truthy()? {
            state.clone()
        } else {
            pair.clone()
        };
        Ok(Some(Value::py_max(first, pair)?))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(state_as_outcome(final_state))
    }
}

/// The number of times the right side is contained in the left side (`CountSubsetEvaluator`).
pub struct CountSubsetEvaluator {
    /// If the divisor is empty, the outcome will be this. If not set (`None`), `ZeroDivisionError` will be raised
    /// for an empty right side.
    pub empty_divisor: Value,
}

impl MultisetEvaluator<Value> for CountSubsetEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let (left, right) = (&counts[0], &counts[1]);
        if right.is_zero() {
            return Ok(Some(state.clone()));
        }
        let current = int(left).binary(BinOp::FloorDiv, &int(right))?;
        Ok(Some(match state {
            Value::None => current,
            state => Value::py_min(state.clone(), current)?,
        }))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        match final_state {
            Value::None => match &self.empty_divisor {
                Value::None => Err(Error::ZeroDivision(
                    "Empty divisor. Set empty_divisor_outcome if you want a particular value in this case."
                        .into(),
                )),
                d => Ok(Final::Outcome(d.clone())),
            },
            state => Ok(Final::Outcome(state.clone())),
        }
    }
}

/// The size of the largest straight (`LargestStraightEvaluator`). Outcomes must be `int`s.
pub struct LargestStraightEvaluator;

impl MultisetEvaluator<Value> for LargestStraightEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        // `best_run, run = state or (0, 0)`
        let (best_run, mut run) = match items(state) {
            [b, r] => (as_count(b), as_count(r)),
            _ => (Int::ZERO, Int::ZERO),
        };
        if counts[0] >= 1 {
            run += 1;
        } else {
            run = Int::ZERO;
        }
        Ok(Some(int_tuple([best_run.max(run.clone()), run])))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        consecutive(outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(Final::Outcome(match items(final_state) {
            [best, _] => best.clone(),
            _ => int(0),
        }))
    }
}

/// The size of the largest straight among the elements and the highest (optionally, lowest) outcome in that
/// straight (`LargestStraightAndOutcomeEvaluator`).
///
/// Straight size is prioritized first, then the outcome. Outcomes must be `int`s.
pub struct LargestStraightAndOutcomeEvaluator {
    /// `priority='high'`: controls which outcome within the straight is returned, and which straight is picked if
    /// there is a tie for largest straight.
    pub prioritize_highest: bool,
}

impl MultisetEvaluator<Value> for LargestStraightAndOutcomeEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, order: Order, outcomes: &[Value], _: &[Option<Int>]) -> Result<Value> {
        let end = if order.value() > 0 {
            outcomes.last()
        } else {
            outcomes.first()
        };
        let end = end.ok_or_else(|| Error::Index("tuple index out of range".into()))?;
        Ok(Value::tuple(vec![int(0), end.clone(), int(0)]))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let [best_run, best_outcome, run] = items(state) else {
            unreachable!("a 3-tuple state")
        };
        let (best_run, mut run) = (as_count(best_run), as_count(run));
        if counts[0] >= 1 {
            run += 1;
        } else {
            run = Int::ZERO;
        }
        let state = |a: &Int, o: Value, r: &Int| Ok(Some(Value::tuple(vec![int(a), o, int(r)])));
        if self.prioritize_highest == (order.value() > 0) {
            if run >= best_run {
                state(&run, outcome.clone(), &run)
            } else {
                state(&best_run, best_outcome.clone(), &run)
            }
        } else if run > best_run {
            // `outcome - (run - 1) * order.value`
            let offset = int(&run - 1).binary(BinOp::Mul, &int(order.value()))?;
            state(&run, outcome.binary(BinOp::Sub, &offset)?, &run)
        } else {
            state(&best_run, best_outcome.clone(), &run)
        }
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        consecutive(outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let [best_run, best_outcome, _] = items(final_state) else {
            unreachable!("a 3-tuple state")
        };
        Ok(Final::Outcome(Value::tuple(vec![
            best_run.clone(),
            best_outcome.clone(),
        ])))
    }
}

/// The sizes of all straights in descending order (`AllStraightsEvaluator`).
///
/// Each element can only contribute to one straight, though duplicate elements can produces straights that
/// overlap in outcomes. In this case, elements are preferentially assigned to the longer straight.
pub struct AllStraightsEvaluator;

/// `(current_runs, ended_runs) = state or ((), ())`.
fn runs(state: &Value) -> (Vec<Int>, Vec<Int>) {
    match items(state) {
        [current, ended] => (
            items(current).iter().map(as_count).collect(),
            items(ended).iter().map(as_count).collect(),
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

impl MultisetEvaluator<Value> for AllStraightsEvaluator {
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let (current_runs, ended_runs) = runs(state);
        let count = &counts[0];
        let len = current_runs.len() as i64;
        let (next_current_runs, next_ended_runs) = if *count < len {
            // `current_runs[:count]` / `current_runs[count:]`: a negative count counts from the end.
            let split = (count + len).max(Int::ZERO).clamp_usize().min(len as usize);
            let split = if count.is_negative() {
                split
            } else {
                count.clamp_usize()
            };
            let next_current: Vec<Int> = current_runs[..split].iter().map(|x| x + 1).collect();
            let mut ended = ended_runs;
            ended.extend_from_slice(&current_runs[split..]);
            (next_current, sorted_descending(ended))
        } else {
            let mut next_current: Vec<Int> = current_runs.iter().map(|x| x + 1).collect();
            // `(1, ) * (count - len(current_runs))`
            let more = crate::keep_tuple::list_len(&(count - len))?;
            next_current.extend(std::iter::repeat_n(Int::ONE, more));
            (next_current, ended_runs)
        };
        Ok(Some(Value::tuple(vec![
            int_tuple(next_current_runs),
            int_tuple(next_ended_runs),
        ])))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        consecutive(outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let (mut all, ended) = runs(final_state);
        all.extend(ended);
        Ok(Final::Outcome(int_tuple(sorted_descending(all))))
    }
}

/// The size of the largest matching set, with wilds (`LargestCountWithWildEvaluator`, what
/// `largest_count(wild=, wild_low=, wild_high=)` uses).
///
/// Each selector is `make_select_function(which)`: `None` selects nothing, a callable is called with the outcome,
/// any other collection becomes a `frozenset` tested with `in`. The caller builds them, returning the selector's
/// result as is: Python only takes its truth value where it uses it (`is_wild or ...`, `if is_wild_early`,
/// `if is_wild_late`), and a result without one (a `Vector` from `identity`) raises only there.
pub struct LargestCountWithWildEvaluator<S> {
    /// These outcomes' counts are combined with the counts of any other outcomes.
    pub wild: S,
    /// These wilds can only be combined with outcomes that they are lower than.
    pub wild_low: S,
    /// These wilds can only be combined with outcomes that they are higher than.
    pub wild_high: S,
}

impl<S: Fn(&Value) -> Result<Value>> MultisetEvaluator<Value> for LargestCountWithWildEvaluator<S> {
    /// `None` before the first outcome, then `(best, total_wild_early)`.
    type State = Option<(Int, Int)>;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Self::State> {
        Ok(None)
    }

    fn next_state(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        let count = &counts[0];
        let is_wild = (self.wild)(outcome)?;
        // `is_wild or select(outcome)`: `is_wild` itself if truthy, else the second selector's result.
        let wild = is_wild.truthy()?;
        let either = |select: &S| -> Result<Value> {
            if wild {
                Ok(is_wild.clone())
            } else {
                select(outcome)
            }
        };
        let (is_wild_early, is_wild_late) = if order.is_ascending() {
            (either(&self.wild_low)?, either(&self.wild_high)?)
        } else {
            (either(&self.wild_high)?, either(&self.wild_low)?)
        };
        let Some((best, total_wild_early)) = state else {
            return Ok(Some(Some(if is_wild_early.truthy()? {
                (count.clone(), count.clone())
            } else {
                (count.clone(), Int::ZERO)
            })));
        };
        let (mut best, mut total_wild_early) = (best.clone(), total_wild_early.clone());
        if is_wild_late.truthy()? {
            best += count;
        }
        best = best.max(count + &total_wild_early);
        if is_wild_early.truthy()? {
            total_wild_early += count;
        }
        Ok(Some(Some((best, total_wild_early))))
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        match final_state {
            // `final_state[0]`
            Some((best, _)) => Ok(Final::Outcome(int(best))),
            None => Err(Error::Type("'NoneType' object is not subscriptable".into())),
        }
    }
}

/// All straights with a reduce operation on the counts (`AllStraightsReduceCountsEvaluator`).
///
/// This can be used to evaluate e.g. cribbage-style straight counting. The result is a tuple of
/// `(run_length, run_score)`s.
pub struct AllStraightsReduceCountsEvaluator<R> {
    /// How to reduce the counts within each straight. The default is `operator.mul`, which counts the number of
    /// ways to pick elements for each straight, e.g. cribbage.
    pub reducer: R,
}

/// `current_run_length, current_run_score, ended_runs = state or (None, None, ())`.
fn reduce_state(state: &Value) -> (Value, Value, Vec<Value>) {
    match items(state) {
        [length, score, ended] => (length.clone(), score.clone(), items(ended).to_vec()),
        _ => (Value::None, Value::None, Vec::new()),
    }
}

/// `tuple(sorted(ended_runs + ((current_run_length, current_run_score), ), reverse=True))`.
fn end_run(ended_runs: Vec<Value>, length: Value, score: Value) -> Result<Vec<Value>> {
    let mut runs = ended_runs;
    runs.push(Value::tuple(vec![length, score]));
    py_sorted_reverse(runs)
}

impl<R: Fn(&Value, &Value) -> Result<Value>> MultisetEvaluator<Value>
    for AllStraightsReduceCountsEvaluator<R>
{
    type State = Value;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Value> {
        Ok(Value::None)
    }

    fn next_state(
        &self,
        state: &Value,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let (mut length, mut score, mut ended_runs) = reduce_state(state);
        let count = &counts[0];
        if count.is_positive() {
            if length == Value::None {
                length = int(1);
                score = int(count);
            } else {
                length = length.binary(BinOp::Add, &int(1))?;
                score = (self.reducer)(&score, &int(count))?;
            }
        } else {
            if length != Value::None {
                ended_runs = end_run(ended_runs, length, score)?;
            }
            length = Value::None;
            score = Value::None;
        }
        Ok(Some(Value::tuple(vec![
            length,
            score,
            Value::tuple(ended_runs),
        ])))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        consecutive(outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let (length, score, mut ended_runs) = reduce_state(final_state);
        if length != Value::None {
            ended_runs = end_run(ended_runs, length, score)?;
        }
        Ok(Final::Outcome(Value::tuple(ended_runs)))
    }
}
