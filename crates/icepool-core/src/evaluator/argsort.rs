//! Port of icepool `evaluator/argsort.py`: the indexes of the originating multisets for each rank in their
//! additive union.

use super::{Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::keep_tuple::list_len;
use crate::order::Order;
use crate::Int;

/// What to do with an outcome that appears in more than one multiset.
///
/// ```
/// use icepool_core::evaluator::argsort::{ArgsortEvaluator, Tie};
/// use icepool_core::evaluator::{evaluate, MultisetSource};
/// use icepool_core::{Die, Int, Order, Pool};
/// let multiset = |outcomes: &[i64]| {
///     let dice = outcomes.iter().map(|&o| (Die::single(o), 1)).collect();
///     MultisetSource::Pool(Pool::new(dice).unwrap().source())
/// };
/// // Python's `MultisetExpression.argsort([3, 1], [3, 2], tie=tie)`, descending: the 3s tie.
/// let argsort = |tie| {
///     let evaluator: ArgsortEvaluator = ArgsortEvaluator { tie, order: Order::Descending, limit: None, limit_pad: None };
///     let result = evaluate(&evaluator, &[multiset(&[3, 1]), multiset(&[3, 2])]).unwrap();
///     result[0].0.clone()
/// };
/// let ints = |xs: &[i64]| xs.iter().map(|&x| Int::from(x)).collect::<Vec<_>>();
/// assert_eq!(argsort(Tie::Drop), ints(&[1, 0]));
/// assert_eq!(argsort(Tie::Left), ints(&[0, 1, 1, 0]));
/// assert_eq!(argsort(Tie::Right), ints(&[1, 0, 1, 0]));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// `ArgsortEvaluatorDrop`: one element of that outcome is dropped from each multiset. This is repeated until
    /// only one multiset has elements of that outcome.
    Drop,
    /// `ArgsortEvaluatorLeft`: the elements coming from the leftmost multiset (lower indexes) are ranked earlier.
    Left,
    /// `ArgsortEvaluatorRight`: the elements coming from the rightmost multiset (higher indexes) are ranked
    /// earlier.
    Right,
}

/// The order an argsort evaluator is to be evaluated in (`order=`): an [`Order`]. Python takes whatever the caller
/// passed and compares the evaluation's `Order` with it by `!=`, so the `Value` layer passes a value.
///
/// ```
/// use icepool_core::evaluator::argsort::RequestedOrder;
/// use icepool_core::Order;
/// assert!(Order::Descending.differs_from(Order::Ascending).unwrap());
/// assert!(!Order::Descending.differs_from(Order::Descending).unwrap());
/// ```
pub trait RequestedOrder {
    /// `order != self._order`.
    ///
    /// ```
    /// use icepool_core::evaluator::argsort::RequestedOrder;
    /// use icepool_core::Order;
    /// // An argsort asked for descending order refuses an ascending evaluation.
    /// let requested = Order::Descending;
    /// assert_eq!(requested.differs_from(Order::Ascending).unwrap(), true);
    /// assert_eq!(requested.differs_from(Order::Descending).unwrap(), false);
    /// ```
    fn differs_from(&self, order: Order) -> Result<bool>;
}

impl RequestedOrder for Order {
    fn differs_from(&self, order: Order) -> Result<bool> {
        Ok(*self != order)
    }
}

/// `if order != self._order: raise UnsupportedOrder(...)`.
fn check_order(order: Order, wanted: &impl RequestedOrder, what: &str) -> Result<()> {
    if wanted.differs_from(order)? {
        // TODO (upstream): implement reverse order?
        return Err(Error::UnsupportedOrder(format!(
            "{what} must be evaluated in the given order."
        )));
    }
    Ok(())
}

/// `len(state) < limit` and friends: `None` is no limit.
fn below(len: usize, limit: Option<i64>) -> bool {
    limit.is_none_or(|limit| (len as i64) < limit)
}

/// `ArgsortEvaluatorDrop` / `ArgsortEvaluatorLeft` / `ArgsortEvaluatorRight`.
///
/// The result is a tuple of indexes (`P`, an [`Int`] unless the `Value` layer says otherwise), padded with
/// `limit_pad` if given.
///
/// ```
/// use icepool_core::evaluator::argsort::{ArgsortEvaluator, Tie};
/// use icepool_core::evaluator::{evaluate, MultisetSource};
/// use icepool_core::{d, Die, Int, Order};
/// // Python's `MultisetExpression.argsort(d(6).pool(1), d(6).pool(1), tie='drop', limit=1, limit_pad=-1)`:
/// // which of two d6 rolled higher, `-1` on a tie.
/// let evaluator: ArgsortEvaluator =
///     ArgsortEvaluator { tie: Tie::Drop, order: Order::Descending, limit: Some(1), limit_pad: Some(Int::from(-1)) };
/// let die = MultisetSource::Pool(d(6).pool(1).source());
/// let result = Die::from_weights(evaluate(&evaluator, &[die.clone(), die]).unwrap());
/// let i = |x: i64| vec![Int::from(x)];
/// assert_eq!(result, Die::from_weights([(i(-1), 6u32), (i(0), 15), (i(1), 15)]));
/// ```
#[derive(Debug)]
pub struct ArgsortEvaluator<W = Order, P = Int> {
    /// How tied elements are ordered.
    pub tie: Tie,
    /// Which order the ranks are to be emitted in (`order=`, default `Order.Descending`).
    pub order: W,
    /// How many ranks to emit.
    pub limit: Option<i64>,
    /// If `limit` is also set and there are not enough elements to reach it, the result is padded with this.
    pub limit_pad: Option<P>,
}

impl<W, P> ArgsortEvaluator<W, P> {
    /// `next_state`, with each count an `int` or the error Python's arithmetic raises when it reaches that count
    /// (the `Value` layer's tuple counts: drop compares every count unless the limit is reached; left / right take
    /// them in turn and may return at the limit before reaching it).
    pub(crate) fn step(&self, mut state: Vec<i64>, counts: Vec<Result<Int>>) -> Result<Vec<i64>> {
        // `min(count, limit - len(state))`, or the count without a limit.
        let room = |state: &Vec<i64>, count: Int| match self.limit {
            None => count,
            Some(limit) => count.min(Int::from(limit) - Int::from(state.len())),
        };
        // `(index, ) * append_count`: Python builds it in memory.
        let append = |state: &mut Vec<i64>, index: i64, append_count: &Int| -> Result<()> {
            state.extend(std::iter::repeat_n(index, list_len(append_count)?));
            Ok(())
        };
        match self.tie {
            Tie::Drop => {
                if below(state.len(), self.limit) {
                    let counts = counts.into_iter().collect::<Result<Vec<Int>>>()?;
                    let mut max_count = &Int::ZERO;
                    let mut tie_count = &Int::ZERO;
                    let mut max_index = 0;
                    for (index, count) in counts.iter().enumerate() {
                        if count > max_count {
                            tie_count = max_count;
                            max_count = count;
                            max_index = index as i64;
                        } else if count > tie_count {
                            tie_count = count;
                        }
                    }
                    let append_count = room(&state, max_count - tie_count);
                    if append_count.is_positive() {
                        append(&mut state, max_index, &append_count)?;
                    }
                }
            }
            Tie::Left | Tie::Right => {
                let mut indexed: Vec<(usize, Result<Int>)> =
                    counts.into_iter().enumerate().collect();
                if self.tie == Tie::Right {
                    indexed.reverse();
                }
                for (index, count) in indexed {
                    let append_count = room(&state, count?);
                    if append_count.is_positive() {
                        append(&mut state, index as i64, &append_count)?;
                        if Some(state.len() as i64) == self.limit {
                            break;
                        }
                    }
                }
            }
        }
        Ok(state)
    }
}

/// Argsort compares counts only, never outcomes: any outcome type will do.
impl<O, W: RequestedOrder, P: Clone + From<i64>> MultisetEvaluator<O> for ArgsortEvaluator<W, P> {
    /// The indexes so far.
    type State = Vec<i64>;
    /// The indexes, padded.
    type Final = Vec<P>;

    fn initial_state(&self, order: Order, _: &[O], _: &[Option<Int>]) -> Result<Vec<i64>> {
        check_order(order, &self.order, "argsort")?;
        Ok(Vec::new())
    }

    fn next_state(
        &self,
        state: &Vec<i64>,
        _: Order,
        _: &O,
        counts: &[Int],
    ) -> Result<Option<Vec<i64>>> {
        let counts = counts.iter().cloned().map(Ok).collect();
        self.step(state.clone(), counts).map(Some)
    }

    fn final_outcome(
        &self,
        final_state: &Vec<i64>,
        _: Order,
        _: &[O],
        _: &[Option<Int>],
    ) -> Result<Final<Vec<P>>> {
        let mut result: Vec<P> = final_state.iter().map(|&i| P::from(i)).collect();
        if let (Some(limit), Some(pad)) = (self.limit, &self.limit_pad) {
            let append_count = limit - final_state.len() as i64;
            if append_count > 0 {
                // `(pad,) * append_count`: `limit` is the user's, so as large as an int goes.
                let n = list_len(&Int::from(append_count))?;
                result.extend(std::iter::repeat_n(pad.clone(), n));
            }
        }
        Ok(Final::Outcome(result))
    }
}

/// `ArgsortGroupedEvaluator`: as argsort, but all ties are grouped together, giving a tuple for each rank rather
/// than a single `int`.
///
/// ```
/// use icepool_core::evaluator::argsort::ArgsortGroupedEvaluator;
/// use icepool_core::evaluator::{evaluate, MultisetSource};
/// use icepool_core::{Die, Int, Order, Pool};
/// let multiset = |outcomes: &[i64]| {
///     let dice = outcomes.iter().map(|&o| (Die::single(o), 1)).collect();
///     MultisetSource::Pool(Pool::new(dice).unwrap().source())
/// };
/// // Python's `MultisetExpression.argsort_grouped([10, 9, 5], [9, 9])`: `((0,), (0, 1, 1), (0,))`.
/// let evaluator: ArgsortGroupedEvaluator = ArgsortGroupedEvaluator { order: Order::Descending, limit: None };
/// let result = evaluate(&evaluator, &[multiset(&[10, 9, 5]), multiset(&[9, 9])]).unwrap();
/// let ints = |xs: &[i64]| xs.iter().map(|&x| Int::from(x)).collect::<Vec<_>>();
/// assert_eq!(result[0].0, vec![ints(&[0]), ints(&[0, 1, 1]), ints(&[0])]);
/// ```
#[derive(Debug)]
pub struct ArgsortGroupedEvaluator<W = Order> {
    /// Which order the ranks are to be emitted in.
    pub order: W,
    /// How many ranks (unique input outcomes) to emit.
    pub limit: Option<i64>,
}

impl<W> ArgsortGroupedEvaluator<W> {
    /// Whether `next_state` still appends (`self._limit is None or len(state) < self._limit`).
    pub(crate) fn below_limit(&self, state: &[Vec<i64>]) -> bool {
        below(state.len(), self.limit)
    }
}

impl<O, W: RequestedOrder> MultisetEvaluator<O> for ArgsortGroupedEvaluator<W> {
    type State = Vec<Vec<i64>>;
    /// The indexes of each rank.
    type Final = Vec<Vec<Int>>;

    fn initial_state(&self, order: Order, _: &[O], _: &[Option<Int>]) -> Result<Self::State> {
        check_order(order, &self.order, "argsort_grouped")?;
        Ok(Vec::new())
    }

    fn next_state(
        &self,
        state: &Self::State,
        _: Order,
        _: &O,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        let mut state = state.clone();
        if self.below_limit(&state) && counts.iter().any(Int::is_positive) {
            // `sum(((i, ) * count for i, count in enumerate(counts)), start=())`
            let mut to_append = Vec::new();
            for (i, count) in counts.iter().enumerate() {
                to_append.extend(std::iter::repeat_n(i as i64, list_len(count)?));
            }
            state.push(to_append);
        }
        Ok(Some(state))
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[O],
        _: &[Option<Int>],
    ) -> Result<Final<Self::Final>> {
        Ok(Final::Outcome(
            final_state
                .iter()
                .map(|group| group.iter().map(|&i| Int::from(i)).collect())
                .collect(),
        ))
    }
}
