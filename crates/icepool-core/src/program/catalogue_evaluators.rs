//! The named evaluators of harness/catalogue/evaluators.py (Rust side), for the `evaluate` node.
//!
//! Each Python entry is a `MultisetEvaluator` subclass; here it is a variant holding the same parameters, with the
//! same method bodies written with Python's operators on values (so outcome types behave as in Python). Their final
//! outcomes may be dice, so `Final` is a `PValue` and the result die is built by the program layer's `new_die`.

use super::ops::{d, new_die};
use super::{raise, PErr, PValue};
use crate::error::{Error, Result};
use crate::evaluator::basic::int;
use crate::evaluator::poker::consecutive;
use crate::evaluator::{evaluate_with_depth_check, Final, MultisetEvaluator};
use crate::order::Order;
use crate::pool::MultisetSource;
use crate::value::BinOp;
use crate::{Pool, Value};

#[derive(Debug)]
pub enum CatalogueEvaluator {
    /// `CountEq(k)`: how many elements equal `k`.
    CountEq(Value),
    /// `RerollAtLeast(k)`: Reroll if any outcome comes up `k` or more times, else the size.
    RerollAtLeast(Value),
    /// `FinalDie(n)`: the size `s`, finalized to Reroll (0), None (negative), `d(s)` (`s <= n`) or `s`.
    FinalDie(Value),
    /// `Seen(req)`: the outcomes with a positive count in the order seen; `req` restricts the order.
    Seen(Value),
    /// `CountDiff()`: `(sum of outcome * (first count - other counts), *sizes)`.
    CountDiff,
    /// `ZeroRuns()`: how many consecutive int outcomes have a zero count.
    ZeroRuns,
}

fn unsupported_order<T>() -> Result<T> {
    Err(Error::UnsupportedOrder(String::new()))
}

/// A `next_state(self, state, order, outcome, count)` with exactly one count: more inputs are a `TypeError`.
fn one_count(counts: &[i64]) -> Result<i64> {
    match counts {
        [count] => Ok(*count),
        _ => Err(Error::Type(format!(
            "next_state() takes 5 positional arguments but {} were given",
            counts.len() + 4
        ))),
    }
}

fn add(a: &Value, b: &Value) -> Result<Value> {
    a.binary(BinOp::Add, b)
}

impl CatalogueEvaluator {
    /// `EVALUATORS[name](*params)`.
    pub fn new(name: &str, params: Vec<PValue>) -> std::result::Result<Self, PErr> {
        use CatalogueEvaluator::*;
        let want = match name {
            "count_diff" | "zero_runs" => 0,
            "count_eq" | "reroll_at_least" | "final_die" | "seen" => 1,
            _ => return raise(Error::Key(format!("no catalogue evaluator {name:?}"))),
        };
        if params.len() != want {
            return raise(Error::Type(format!(
                "{name}() takes {want} parameters, {} given",
                params.len()
            )));
        }
        let mut params = params.into_iter().map(|p| match p {
            PValue::Value(v) => Ok(v),
            _ => Err(PErr::Unimplemented(
                "a non-value evaluator parameter".into(),
            )),
        });
        let mut param = || params.next().expect("arity checked");
        Ok(match name {
            "count_eq" => CountEq(param()?),
            "reroll_at_least" => RerollAtLeast(param()?),
            "final_die" => FinalDie(param()?),
            "seen" => Seen(param()?),
            "count_diff" => CountDiff,
            "zero_runs" => ZeroRuns,
            _ => unreachable!("checked above"),
        })
    }
}

impl MultisetEvaluator<Value> for CatalogueEvaluator {
    type State = Value;
    type Final = PValue;

    fn initial_state(&self, order: Order, _: &[Value], _: &[Option<i64>]) -> Result<Value> {
        use CatalogueEvaluator::*;
        Ok(match self {
            CountEq(_) | FinalDie(_) | CountDiff | ZeroRuns => int(0),
            RerollAtLeast(_) => Value::None,
            Seen(req) => {
                if *req == int(2) || (*req == int(1) && order.value() < 0) {
                    return unsupported_order();
                }
                Value::tuple(Vec::new())
            }
        })
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        use CatalogueEvaluator::*;
        Ok(Some(match self {
            CountEq(k) => {
                let count = one_count(counts)?;
                if outcome == k {
                    add(state, &int(count))?
                } else {
                    state.clone()
                }
            }
            RerollAtLeast(k) => {
                let count = one_count(counts)?;
                if int(count).binary(BinOp::Ge, k)?.truthy() {
                    return Ok(None);
                }
                // `(state or 0) + count`
                let state = if state.truthy() {
                    state.clone()
                } else {
                    int(0)
                };
                add(&state, &int(count))?
            }
            FinalDie(_) => add(state, &int(one_count(counts)?))?,
            Seen(req) => {
                if *req == int(-1) && order.value() > 0 {
                    return unsupported_order();
                }
                if counts.iter().any(|&c| c > 0) {
                    let Value::Tuple(seen) = state else {
                        unreachable!("a tuple state")
                    };
                    let mut seen = seen.to_vec();
                    seen.push(outcome.clone());
                    Value::tuple(seen)
                } else {
                    state.clone()
                }
            }
            CountDiff => {
                // `state + outcome * (counts[0] - sum(counts[1:]))`
                let first = counts
                    .first()
                    .ok_or_else(|| Error::Index("tuple index out of range".into()))?;
                let rest: i64 = counts[1..].iter().sum();
                add(state, &outcome.binary(BinOp::Mul, &int(first - rest))?)?
            }
            ZeroRuns => add(state, &int(i64::from(one_count(counts)? == 0)))?,
        }))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        match self {
            CatalogueEvaluator::ZeroRuns => consecutive(outcomes),
            _ => Ok(Vec::new()),
        }
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<Final<PValue>> {
        use CatalogueEvaluator::*;
        let outcome = |v: &Value| match v {
            Value::None => Final::None,
            v => Final::Outcome(PValue::Value(v.clone())),
        };
        Ok(match self {
            FinalDie(n) => {
                if *final_state == int(0) {
                    Final::Reroll
                } else if final_state.binary(BinOp::Lt, &int(0))?.truthy() {
                    Final::None
                } else if final_state.binary(BinOp::Le, n)?.truthy() {
                    match d(PValue::Value(final_state.clone())) {
                        Ok(die) => Final::Outcome(die),
                        Err(PErr::Raise(e)) => return Err(e),
                        Err(PErr::Unimplemented(what)) => return Err(Error::Unsupported(what)),
                    }
                } else {
                    outcome(final_state)
                }
            }
            CountDiff => {
                // `(final_state,) + sizes`
                let mut items = vec![final_state.clone()];
                items.extend(sizes.iter().map(|s| s.map_or(Value::None, int)));
                Final::Outcome(PValue::Value(Value::tuple(items)))
            }
            _ => outcome(final_state),
        })
    }
}

/// `EVALUATORS[name](*params).evaluate(*pools)`: `Die(final_outcomes, final_weights)` through `new_die`, which
/// mixes in final outcomes that are dice.
pub fn run(
    evaluator: &CatalogueEvaluator,
    pools: &[Pool<Value>],
) -> std::result::Result<PValue, PErr> {
    let sources: Vec<MultisetSource<Value>> = pools
        .iter()
        .map(|p| MultisetSource::Pool(p.source()))
        .collect();
    let finals = evaluate_with_depth_check(evaluator, &sources, super::evaluate::check_recursion)?;
    let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
    let weights: Vec<Value> = weights.into_iter().map(|w| Value::Int(w.into())).collect();
    new_die(outcomes, &weights)
}
