//! Port of icepool `evaluator/argsort.py`: the indexes of the originating multisets for each rank in their
//! additive union.

use super::{tuple_compared_with_int, Final, MultisetEvaluator};
use crate::error::{Error, Result};
use crate::keep_tuple::list_len;
use crate::order::Order;
use crate::Int;
use crate::Value;

/// What to do with an outcome that appears in more than one multiset.
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

/// `if order != self._order: raise UnsupportedOrder(...)`. `self._order` is whatever the caller passed (Python
/// compares the `Order` with it by `==`), so it is a value here.
fn check_order(order: Order, wanted: &Value, what: &str) -> Result<()> {
    if Value::from(order.value()).eq_op(wanted, true)?.truthy()? {
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
pub struct ArgsortEvaluator {
    /// How tied elements are ordered.
    pub tie: Tie,
    /// Which order the ranks are to be emitted in (`order=`, default `Order.Descending`).
    pub order: Value,
    /// How many ranks to emit.
    pub limit: Option<i64>,
    /// If `limit` is also set and there are not enough elements to reach it, the result is padded with this.
    pub limit_pad: Option<Value>,
}

impl ArgsortEvaluator {
    /// `next_state` with each count an `int`, or the error Python raises when its arithmetic reaches that count (a
    /// tuple).
    fn step(&self, mut state: Vec<i64>, counts: Vec<Result<Int>>) -> Result<Vec<i64>> {
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

impl MultisetEvaluator<Value> for ArgsortEvaluator {
    /// The indexes so far.
    type State = Vec<i64>;
    type Final = Value;

    fn initial_state(&self, order: Order, _: &[Value], _: &[Option<Int>]) -> Result<Vec<i64>> {
        check_order(order, &self.order, "argsort")?;
        Ok(Vec::new())
    }

    fn next_state(
        &self,
        state: &Vec<i64>,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Vec<i64>>> {
        let counts = counts.iter().cloned().map(Ok).collect();
        self.step(state.clone(), counts).map(Some)
    }

    /// Python's arithmetic on a tuple count: drop compares `count > max_count` (`TypeError`) unless the limit is
    /// reached, so it never looks at the counts; left / right take each count in turn, `min(count, limit -
    /// len(state))` (`TypeError`) or the count itself, then `append_count > 0` (`TypeError`), and may return
    /// early once the limit is reached.
    fn next_state_values(
        &self,
        state: &Vec<i64>,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Vec<i64>>> {
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
        self.step(state.clone(), counts).map(Some)
    }

    /// `argsort` reads no sizes.
    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Vec<i64>,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        let mut result: Vec<Value> = final_state.iter().map(|&i| Value::from(i)).collect();
        if let (Some(limit), Some(pad)) = (self.limit, &self.limit_pad) {
            let append_count = limit - final_state.len() as i64;
            if append_count > 0 {
                // `(pad,) * append_count`: `limit` is the user's, so as large as an int goes.
                let n = crate::keep_tuple::list_len(&Int::from(append_count))?;
                result.extend(std::iter::repeat_n(pad.clone(), n));
            }
        }
        Ok(Final::Outcome(Value::tuple(result)))
    }
}

/// `ArgsortGroupedEvaluator`: as argsort, but all ties are grouped together, giving a tuple for each rank rather
/// than a single `int`.
pub struct ArgsortGroupedEvaluator {
    /// Which order the ranks are to be emitted in.
    pub order: Value,
    /// How many ranks (unique input outcomes) to emit.
    pub limit: Option<i64>,
}

impl MultisetEvaluator<Value> for ArgsortGroupedEvaluator {
    type State = Vec<Vec<i64>>;
    type Final = Value;

    fn initial_state(&self, order: Order, _: &[Value], _: &[Option<Int>]) -> Result<Self::State> {
        check_order(order, &self.order, "argsort_grouped")?;
        Ok(Vec::new())
    }

    fn next_state(
        &self,
        state: &Self::State,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        let mut state = state.clone();
        if below(state.len(), self.limit) && counts.iter().any(Int::is_positive) {
            // `sum(((i, ) * count for i, count in enumerate(counts)), start=())`
            let mut to_append = Vec::new();
            for (i, count) in counts.iter().enumerate() {
                to_append.extend(std::iter::repeat_n(i as i64, list_len(count)?));
            }
            state.push(to_append);
        }
        Ok(Some(state))
    }

    /// A tuple count: `any(count > 0 for count in counts)` short-circuits on an earlier positive `int` count, and
    /// then `(i, ) * count` of the tuple raises; else `count > 0` does. Both only if the limit is not reached.
    fn next_state_values(
        &self,
        state: &Self::State,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Self::State>> {
        if !below(state.len(), self.limit) {
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

    /// `argsort_grouped` reads no sizes.
    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(Final::Outcome(Value::tuple(
            final_state
                .iter()
                .map(|group| Value::tuple(group.iter().map(|&i| Value::from(i)).collect()))
                .collect(),
        )))
    }
}
