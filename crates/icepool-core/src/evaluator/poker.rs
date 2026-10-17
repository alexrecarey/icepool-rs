//! Port of icepool `evaluator/poker.py`: evaluators for poker/Yahtzee-like mechanics.
//!
//! Each evaluator is a [`CountFold`] over any outcome type, with typed states and final outcomes: counts are
//! [`Int`]s, straights need [`IntegerOutcome`]s. The `Value` layer (`value/evaluators.rs`) turns the finals into
//! Python values and runs Python's own arithmetic on tuple counts (a `MultiDeal` input), which have no typed
//! meaning.

use dashu_int::IBig;

use super::fold::{count_fold_evaluators, CountFold};
use super::Final;
use crate::error::{Error, Result};
use crate::order::Order;
use std::marker::PhantomData;

use crate::outcome::{gt, min, sort_by, ErrInto, OutcomeOrder};
use crate::Int;

count_fold_evaluators!(
    HighestOutcomeAndCountEvaluator,
    AllCountsEvaluator<F>,
    LargestCountEvaluator,
    LargestCountAndOutcomeEvaluator,
    CountSubsetEvaluator<D>,
    LargestStraightEvaluator,
    LargestStraightAndOutcomeEvaluator,
    AllStraightsEvaluator,
    LargestCountWithWildEvaluator<S, T>,
    AllStraightsReduceCountsEvaluator<R, S>,
);

/// More consecutive outcomes than this and the port refuses (`range` of a huge span: Python would take minutes).
pub(crate) const MAX_CONSECUTIVE: usize = 1 << 20;

/// Integer outcomes: what the straight evaluators need.
pub trait IntegerOutcome: OutcomeOrder {
    /// `MultisetEvaluator.consecutive`: all `int`s from the min outcome to the max outcome among the inputs
    /// (`outcomes`, sorted), inclusive.
    fn consecutive(outcomes: &[Self]) -> Result<Vec<Self>>;

    /// `self - n`.
    fn try_sub_int(&self, n: &Int) -> Result<Self>;
}

/// The number of integers in `lo..=hi`, refused past [`MAX_CONSECUTIVE`].
pub(crate) fn consecutive_span(lo: IBig, hi: IBig) -> Result<usize> {
    let span = usize::try_from(hi - lo + IBig::ONE).unwrap_or(0);
    if span > MAX_CONSECUTIVE {
        return Err(Error::Unported(
            "consecutive outcomes over a huge range".into(),
        ));
    }
    Ok(span)
}

macro_rules! integer_outcome {
    ($($t:ty),*) => {$(
        impl IntegerOutcome for $t {
            fn consecutive(outcomes: &[Self]) -> Result<Vec<Self>> {
                let (Some(&lo), Some(&hi)) = (outcomes.first(), outcomes.last()) else {
                    return Ok(Vec::new());
                };
                consecutive_span(IBig::from(lo), IBig::from(hi))?;
                Ok((lo..=hi).collect())
            }

            fn try_sub_int(&self, n: &Int) -> Result<Self> {
                <$t>::try_from(n.to_ibig())
                    .ok()
                    .and_then(|n| self.checked_sub(n))
                    .ok_or_else(|| Error::Overflow("attempt to subtract with overflow".into()))
            }
        }
    )*};
}

integer_outcome!(i8, i16, i32, i64, i128, isize);

/// The truth value of what a wild selector returns: Python only asks for it where it branches on it, and for some
/// values (a `Vector`) asking raises.
pub trait Truth {
    /// The truth value (Python's `bool(x)`), or why there is none.
    fn truth(&self) -> Result<bool>;
}

impl Truth for bool {
    fn truth(&self) -> Result<bool> {
        Ok(*self)
    }
}

/// `AllCountsEvaluator`'s `filter`: which counts are kept.
pub trait CountFilter {
    /// `self._filter == 'all' or count >= self._filter`.
    fn keeps(&self, count: &Int) -> Result<bool>;
}

/// `None` is `filter='all'`; `Some(n)` keeps counts of at least `n`.
impl CountFilter for Option<Int> {
    fn keeps(&self, count: &Int) -> Result<bool> {
        Ok(self.as_ref().is_none_or(|filter| count >= filter))
    }
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

impl<O: OutcomeOrder> CountFold<O> for HighestOutcomeAndCountEvaluator {
    /// `None`, then `(outcome, count)`.
    type State = Option<(O, Int)>;
    type Final = (O, Int);

    fn fold(
        &self,
        state: &Self::State,
        _: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Self::State> {
        let count = counts[0].clone().max(Int::ZERO);
        let Some((best, best_count)) = state else {
            return Ok(Some((outcome.clone(), count)));
        };
        if count.is_positive() && gt(outcome, best)? {
            return Ok(Some((outcome.clone(), count)));
        }
        if count.is_zero() && best_count.is_zero() {
            return Ok(Some((min([outcome, best])?.clone(), Int::ZERO)));
        }
        Ok(state.clone())
    }

    /// Always sees zero counts.
    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        Ok(outcomes.to_vec())
    }

    fn finish(&self, state: &Self::State) -> Result<Final<(O, Int)>> {
        Ok(state.clone().map_or(Final::None, Final::Outcome))
    }
}

/// All counts in descending order (`AllCountsEvaluator`). In other words, this produces tuples of the sizes of all
/// matching sets.
pub struct AllCountsEvaluator<F = Option<Int>> {
    /// Any counts below this value will not be in the output. For example, `filter=2` will only produce pairs and
    /// better. If `'all'` (`None`), no filtering will be done.
    pub filter: F,
}

impl<O: Clone, F: CountFilter> CountFold<O> for AllCountsEvaluator<F> {
    /// The counts so far, descending (Python's `state or ()`).
    type State = Vec<Int>;
    type Final = Vec<Int>;

    fn fold(&self, state: &Vec<Int>, _: Order, _: &O, counts: &[Int]) -> Result<Vec<Int>> {
        let count = &counts[0];
        if self.filter.keeps(count)? {
            let mut state = state.clone();
            state.push(count.clone());
            Ok(sorted_descending(state))
        } else {
            Ok(state.clone())
        }
    }

    /// Always sees zero counts.
    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        Ok(outcomes.to_vec())
    }

    fn finish(&self, state: &Vec<Int>) -> Result<Final<Vec<Int>>> {
        Ok(Final::Outcome(state.clone()))
    }
}

/// The largest count of any outcome (`LargestCountEvaluator`).
pub struct LargestCountEvaluator;

impl<O> CountFold<O> for LargestCountEvaluator {
    type State = Option<Int>;
    type Final = Int;

    fn fold(&self, state: &Option<Int>, _: Order, _: &O, counts: &[Int]) -> Result<Option<Int>> {
        // `max(state or count, count)`: a state of 0 is falsy too, which matters for negative counts.
        let count = &counts[0];
        let first = match state {
            Some(s) if !s.is_zero() => s,
            _ => count,
        };
        Ok(Some(first.max(count).clone()))
    }

    fn finish(&self, state: &Option<Int>) -> Result<Final<Int>> {
        Ok(state.clone().map_or(Final::None, Final::Outcome))
    }
}

/// `max(a, b)` of `(count, outcome)` pairs: Python compares tuples elementwise, so by count, then (where the
/// counts are equal and the outcomes differ by `==`) by outcome. `b` only if it is greater, so the first of equal
/// pairs wins.
fn max_pair<O: OutcomeOrder>(a: (Int, O), b: (Int, O)) -> Result<(Int, O)> {
    let b_greater = if a.0 != b.0 {
        b.0 > a.0
    } else if a.1 == b.1 || a.1.try_eq(&b.1).err_into()? {
        false
    } else {
        gt(&b.1, &a.1)?
    };
    Ok(if b_greater { b } else { a })
}

/// The largest count of any outcome, along with that outcome (`LargestCountAndOutcomeEvaluator`).
pub struct LargestCountAndOutcomeEvaluator;

impl<O: OutcomeOrder> CountFold<O> for LargestCountAndOutcomeEvaluator {
    /// `None`, then `(count, outcome)`.
    type State = Option<(Int, O)>;
    type Final = (Int, O);

    fn fold(
        &self,
        state: &Self::State,
        _: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Self::State> {
        // `max(state or (count, outcome), (count, outcome))`
        let pair = (counts[0].clone(), outcome.clone());
        let first = state.clone().unwrap_or_else(|| pair.clone());
        max_pair(first, pair).map(Some)
    }

    fn finish(&self, state: &Self::State) -> Result<Final<(Int, O)>> {
        Ok(state.clone().map_or(Final::None, Final::Outcome))
    }
}

/// The number of times the right side is contained in the left side (`CountSubsetEvaluator`).
pub struct CountSubsetEvaluator<D = Int> {
    /// If the divisor is empty, the outcome will be this. If not set (`None`), `ZeroDivisionError` will be raised
    /// for an empty right side.
    pub empty_divisor: Option<D>,
}

impl<D> CountSubsetEvaluator<D> {
    /// `final_outcome` of a final state (`None` if the divisor was empty).
    pub(crate) fn final_outcome_of(&self, state: Option<D>) -> Result<Final<D>>
    where
        D: Clone,
    {
        match state.or_else(|| self.empty_divisor.clone()) {
            Some(outcome) => Ok(Final::Outcome(outcome)),
            None => Err(Error::DivisionByZero(
                "Empty divisor. Set empty_divisor_outcome if you want a particular value in this case.".into(),
            )),
        }
    }
}

impl<O, D: Clone + From<Int>> CountFold<O> for CountSubsetEvaluator<D> {
    type State = Option<Int>;
    type Final = D;

    fn fold(&self, state: &Option<Int>, _: Order, _: &O, counts: &[Int]) -> Result<Option<Int>> {
        let (left, right) = (&counts[0], &counts[1]);
        if right.is_zero() {
            return Ok(state.clone());
        }
        let current = left.floor_divmod(right)?.0;
        Ok(Some(match state {
            None => current,
            // `min(state, current)`
            Some(state) => state.clone().min(current),
        }))
    }

    fn finish(&self, state: &Option<Int>) -> Result<Final<D>> {
        self.final_outcome_of(state.clone().map(D::from))
    }
}

/// The size of the largest straight (`LargestStraightEvaluator`). Outcomes must be `int`s.
pub struct LargestStraightEvaluator;

impl<O: IntegerOutcome> CountFold<O> for LargestStraightEvaluator {
    /// `(best_run, run)`: Python's `state or (0, 0)`.
    type State = (Int, Int);
    type Final = Int;

    fn fold(&self, state: &(Int, Int), _: Order, _: &O, counts: &[Int]) -> Result<(Int, Int)> {
        let (best_run, mut run) = state.clone();
        if counts[0] >= 1 {
            run += 1;
        } else {
            run = Int::ZERO;
        }
        Ok((best_run.max(run.clone()), run))
    }

    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        O::consecutive(outcomes)
    }

    fn finish(&self, state: &(Int, Int)) -> Result<Final<Int>> {
        Ok(Final::Outcome(state.0.clone()))
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

impl<O: IntegerOutcome> CountFold<O> for LargestStraightAndOutcomeEvaluator {
    /// `(best_run, best_outcome, run)`, always set by `initial_state`.
    type State = Option<(Int, O, Int)>;
    type Final = (Int, O);

    fn initial(&self, order: Order, outcomes: &[O]) -> Result<Self::State> {
        let end = if order.value() > 0 {
            outcomes.last()
        } else {
            outcomes.first()
        };
        let end = end.ok_or_else(|| Error::IndexOutOfRange("tuple index out of range".into()))?;
        Ok(Some((Int::ZERO, end.clone(), Int::ZERO)))
    }

    fn fold(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Self::State> {
        let Some((best_run, best_outcome, run)) = state else {
            return Err(Error::WrongType(
                "cannot unpack non-iterable NoneType object".into(),
            ));
        };
        let run = if counts[0] >= 1 { run + 1 } else { Int::ZERO };
        if self.prioritize_highest == (order.value() > 0) {
            if run >= *best_run {
                Ok(Some((run.clone(), outcome.clone(), run)))
            } else {
                Ok(Some((best_run.clone(), best_outcome.clone(), run)))
            }
        } else if run > *best_run {
            // `outcome - (run - 1) * order.value`
            let offset = (&run - 1) * order.value();
            Ok(Some((run.clone(), outcome.try_sub_int(&offset)?, run)))
        } else {
            Ok(Some((best_run.clone(), best_outcome.clone(), run)))
        }
    }

    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        O::consecutive(outcomes)
    }

    fn finish(&self, state: &Self::State) -> Result<Final<(Int, O)>> {
        Ok(match state {
            Some((best_run, best_outcome, _)) => {
                Final::Outcome((best_run.clone(), best_outcome.clone()))
            }
            None => Final::None,
        })
    }
}

/// The sizes of all straights in descending order (`AllStraightsEvaluator`).
///
/// Each element can only contribute to one straight, though duplicate elements can produces straights that
/// overlap in outcomes. In this case, elements are preferentially assigned to the longer straight.
pub struct AllStraightsEvaluator;

impl<O: IntegerOutcome> CountFold<O> for AllStraightsEvaluator {
    /// `(current_runs, ended_runs)`: Python's `state or ((), ())`.
    type State = (Vec<Int>, Vec<Int>);
    type Final = Vec<Int>;

    fn fold(&self, state: &Self::State, _: Order, _: &O, counts: &[Int]) -> Result<Self::State> {
        let (current_runs, ended_runs) = state;
        let count = &counts[0];
        let len = current_runs.len() as i64;
        if *count < len {
            // `current_runs[:count]` / `current_runs[count:]`: a negative count counts from the end.
            let split = if count.is_negative() {
                (count + len).max(Int::ZERO).clamp_usize()
            } else {
                count.clamp_usize()
            };
            let next_current_runs = current_runs[..split].iter().map(|x| x + 1).collect();
            let mut ended = ended_runs.clone();
            ended.extend_from_slice(&current_runs[split..]);
            Ok((next_current_runs, sorted_descending(ended)))
        } else {
            let mut next_current_runs: Vec<Int> = current_runs.iter().map(|x| x + 1).collect();
            // `(1, ) * (count - len(current_runs))`
            let more = crate::keep_tuple::list_len(&(count - len))?;
            next_current_runs.extend(std::iter::repeat_n(Int::ONE, more));
            Ok((next_current_runs, ended_runs.clone()))
        }
    }

    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        O::consecutive(outcomes)
    }

    fn finish(&self, (current_runs, ended_runs): &Self::State) -> Result<Final<Vec<Int>>> {
        let mut all = current_runs.clone();
        all.extend_from_slice(ended_runs);
        Ok(Final::Outcome(sorted_descending(all)))
    }
}

/// The size of the largest matching set, with wilds (`LargestCountWithWildEvaluator`, what
/// `largest_count(wild=, wild_low=, wild_high=)` uses).
///
/// Each selector is `make_select_function(which)`: `None` selects nothing, a callable is called with the outcome,
/// any other collection becomes a `frozenset` tested with `in`. The caller builds them, returning the selector's
/// result as is ([`Truth`]): Python only takes its truth value where it uses it (`is_wild or ...`,
/// `if is_wild_early`, `if is_wild_late`), and a result without one (a `Vector` from `identity`) raises only there.
pub struct LargestCountWithWildEvaluator<S, T = bool> {
    /// These outcomes' counts are combined with the counts of any other outcomes.
    pub wild: S,
    /// These wilds can only be combined with outcomes that they are lower than.
    pub wild_low: S,
    /// These wilds can only be combined with outcomes that they are higher than.
    pub wild_high: S,
    /// What the selectors return.
    truth: PhantomData<fn() -> T>,
}

impl<S, T> LargestCountWithWildEvaluator<S, T> {
    /// The evaluator with these selectors (wild cards, and the wild ones that count low or high).
    pub fn new(wild: S, wild_low: S, wild_high: S) -> Self {
        LargestCountWithWildEvaluator {
            wild,
            wild_low,
            wild_high,
            truth: PhantomData,
        }
    }

    /// `(is_wild_early, is_wild_late)` of `outcome`: `is_wild or self._wild_low(outcome)` and
    /// `is_wild or self._wild_high(outcome)`, early and late in the order `next_state` sees the outcomes.
    pub(crate) fn wilds<O>(&self, order: Order, outcome: &O) -> Result<(T, T)>
    where
        S: Fn(&O) -> Result<T>,
        T: Truth + Clone,
    {
        let is_wild = (self.wild)(outcome)?;
        // `is_wild or select(outcome)`: `is_wild` itself if truthy, else the second selector's result.
        let wild = is_wild.truth()?;
        let either = |select: &S| -> Result<T> {
            if wild {
                Ok(is_wild.clone())
            } else {
                select(outcome)
            }
        };
        if order.is_ascending() {
            Ok((either(&self.wild_low)?, either(&self.wild_high)?))
        } else {
            Ok((either(&self.wild_high)?, either(&self.wild_low)?))
        }
    }
}

impl<O, T: Truth + Clone, S: Fn(&O) -> Result<T>> CountFold<O>
    for LargestCountWithWildEvaluator<S, T>
{
    /// `None` before the first outcome, then `(best, total_wild_early)`.
    type State = Option<(Int, Int)>;
    type Final = Int;

    fn fold(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Self::State> {
        let count = &counts[0];
        let (is_wild_early, is_wild_late) = self.wilds(order, outcome)?;
        let Some((best, total_wild_early)) = state else {
            return Ok(Some(if is_wild_early.truth()? {
                (count.clone(), count.clone())
            } else {
                (count.clone(), Int::ZERO)
            }));
        };
        let (mut best, mut total_wild_early) = (best.clone(), total_wild_early.clone());
        if is_wild_late.truth()? {
            best += count;
        }
        best = best.max(count + &total_wild_early);
        if is_wild_early.truth()? {
            total_wild_early += count;
        }
        Ok(Some((best, total_wild_early)))
    }

    fn finish(&self, state: &Self::State) -> Result<Final<Int>> {
        match state {
            // `final_state[0]`
            Some((best, _)) => Ok(Final::Outcome(best.clone())),
            None => Err(Error::WrongType(
                "'NoneType' object is not subscriptable".into(),
            )),
        }
    }
}

/// All straights with a reduce operation on the counts (`AllStraightsReduceCountsEvaluator`).
///
/// This can be used to evaluate e.g. cribbage-style straight counting. The result is a tuple of
/// `(run_length, run_score)`s; a run's score starts as its first count (`S::from`).
pub struct AllStraightsReduceCountsEvaluator<R, S = Int> {
    /// How to reduce the counts within each straight. The default is `operator.mul`, which counts the number of
    /// ways to pick elements for each straight, e.g. cribbage.
    pub reducer: R,
    /// The scores' type.
    score: PhantomData<fn() -> S>,
}

impl<R, S> AllStraightsReduceCountsEvaluator<R, S> {
    /// The evaluator with this reducer of the straights' counts.
    pub fn new(reducer: R) -> Self {
        AllStraightsReduceCountsEvaluator {
            reducer,
            score: PhantomData,
        }
    }
}

/// `tuple(sorted(ended_runs + (current_run, ), reverse=True))` for `(length, score)` runs: Python compares the
/// tuples by length, then (where the lengths are equal and the scores differ by `==`) by score. A reverse sort
/// reverses, sorts stably and reverses back, so equal runs keep their order.
fn end_run<S: OutcomeOrder>(
    ended_runs: &[(Int, S)],
    current_run: (Int, S),
) -> Result<Vec<(Int, S)>> {
    let mut runs = ended_runs.to_vec();
    runs.push(current_run);
    runs.reverse();
    let mut runs = sort_by::<S, _>(runs, |a, b| {
        if a.0 != b.0 {
            Ok(a.0 < b.0)
        } else if a.1 == b.1 || a.1.try_eq(&b.1)? {
            Ok(false)
        } else {
            a.1.try_lt(&b.1)
        }
    })?;
    runs.reverse();
    Ok(runs)
}

impl<O, S, R> CountFold<O> for AllStraightsReduceCountsEvaluator<R, S>
where
    O: IntegerOutcome,
    S: OutcomeOrder + From<Int>,
    R: Fn(&S, &Int) -> Result<S>,
{
    /// `(current_run, ended_runs)`, the current run `(length, score)` if there is one: Python's
    /// `state or (None, None, ())`.
    type State = (Option<(Int, S)>, Vec<(Int, S)>);
    type Final = Vec<(Int, S)>;

    fn fold(&self, state: &Self::State, _: Order, _: &O, counts: &[Int]) -> Result<Self::State> {
        let (current_run, ended_runs) = state;
        let count = &counts[0];
        if count.is_positive() {
            let current_run = match current_run {
                None => (Int::ONE, S::from(count.clone())),
                Some((length, score)) => (length + 1, (self.reducer)(score, count)?),
            };
            Ok((Some(current_run), ended_runs.clone()))
        } else {
            let ended_runs = match current_run {
                Some(run) => end_run(ended_runs, run.clone())?,
                None => ended_runs.clone(),
            };
            Ok((None, ended_runs))
        }
    }

    fn extra(&self, outcomes: &[O]) -> Result<Vec<O>> {
        O::consecutive(outcomes)
    }

    fn finish(&self, (current_run, ended_runs): &Self::State) -> Result<Final<Vec<(Int, S)>>> {
        Ok(Final::Outcome(match current_run {
            Some(run) => end_run(ended_runs, run.clone())?,
            None => ended_runs.clone(),
        }))
    }
}
