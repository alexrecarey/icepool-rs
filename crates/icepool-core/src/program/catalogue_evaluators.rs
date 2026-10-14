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
use crate::pool::{Count, MultisetSource};
use crate::value::builtins::py_sum;
use crate::value::BinOp;
use crate::Value;

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
    /// `ScaledSize()` with `evaluate(..., start=, scale=)`: `(start + sum of all counts) * scale`. `unexpected` is
    /// the first other keyword, a `TypeError` once `initial_state` is called.
    ScaledSize {
        start: Value,
        scale: Value,
        unexpected: Option<String>,
    },
    /// `HandSums()`: for a tuple input (a `MultiDeal`), the sum of each hand.
    HandSums,
    /// `Record()`: every `(outcome, *counts)` in the order seen, then `(record, sizes)`; for int and tuple counts.
    Record,
}

fn unsupported_order<T>() -> Result<T> {
    Err(Error::UnsupportedOrder(String::new()))
}

/// A `next_state(self, state, order, outcome, count)` with exactly one count: more inputs are a `TypeError`.
fn one_count(counts: &[Value]) -> Result<&Value> {
    match counts {
        [count] => Ok(count),
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
            "count_diff" | "zero_runs" | "scaled_size" | "hand_sums" | "record" => 0,
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
            "hand_sums" => HandSums,
            "record" => Record,
            "scaled_size" => ScaledSize {
                start: int(0),
                scale: int(1),
                unexpected: None,
            },
            _ => unreachable!("checked above"),
        })
    }

    /// The keyword arguments of `evaluate(*args, **kwargs)`, which Python passes to `initial_state` and
    /// `final_outcome`. Only `scaled_size` declares any; the others are not given keywords by the generators.
    pub fn with_kwargs(mut self, kwargs: Vec<(String, PValue)>) -> std::result::Result<Self, PErr> {
        if kwargs.is_empty() {
            return Ok(self);
        }
        let CatalogueEvaluator::ScaledSize {
            start,
            scale,
            unexpected,
        } = &mut self
        else {
            return Err(PErr::Unimplemented(
                "keyword arguments to this catalogue evaluator".into(),
            ));
        };
        for (name, v) in kwargs {
            let PValue::Value(v) = v else {
                return Err(PErr::Unimplemented(
                    "a non-value evaluator keyword argument".into(),
                ));
            };
            match name.as_str() {
                "start" => *start = v,
                "scale" => *scale = v,
                _ => {
                    unexpected.get_or_insert(name);
                }
            }
        }
        Ok(self)
    }
}

/// A count or size as the Python object `next_state` / `initial_state` receive: an `int`, a tuple of `int`s (a
/// `MultiDeal` input), or `None` (a size with negative counts).
fn count_value(c: &Count) -> Value {
    match c {
        Count::Int(c) => int(*c),
        Count::Tuple(cs) => Value::tuple(cs.iter().map(|&c| int(c)).collect()),
    }
}

fn size_values(sizes: &[Option<Count>]) -> Vec<Value> {
    sizes
        .iter()
        .map(|s| s.as_ref().map_or(Value::None, count_value))
        .collect()
}

fn int_counts(counts: &[i64]) -> Vec<Count> {
    counts.iter().map(|&c| Count::Int(c)).collect()
}

fn int_sizes(sizes: &[Option<i64>]) -> Vec<Option<Count>> {
    sizes.iter().map(|s| s.map(Count::Int)).collect()
}

/// `iter(x)` over a tuple; anything else here is an int or `None`, which is not iterable (`TypeError`).
fn tuple_items(x: &Value) -> Result<&[Value]> {
    match x {
        Value::Tuple(items) => Ok(items),
        x => Err(Error::Type(format!(
            "'{}' object is not iterable",
            x.type_name()
        ))),
    }
}

/// The bodies are written over the counts and sizes as Python values, so a tuple count (a `MultiDeal` input)
/// meets Python's operators exactly as in the Python class (mostly a `TypeError`; `hand_sums` and `record` are
/// the ones written for tuples). The `int`-count methods go through the same bodies.
impl MultisetEvaluator<Value> for CatalogueEvaluator {
    type State = Value;
    type Final = PValue;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<Value> {
        self.initial_state_counts(order, outcomes, &int_sizes(sizes))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        self.next_state_counts(state, order, outcome, &int_counts(counts))
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<Final<PValue>> {
        self.final_outcome_counts(final_state, order, outcomes, &int_sizes(sizes))
    }

    fn initial_state_counts(
        &self,
        order: Order,
        _: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Value> {
        use CatalogueEvaluator::*;
        Ok(match self {
            CountEq(_) | FinalDie(_) | CountDiff | ZeroRuns => int(0),
            ScaledSize {
                start, unexpected, ..
            } => {
                if let Some(name) = unexpected {
                    return Err(Error::Type(format!(
                        "ScaledSize.initial_state() got an unexpected keyword argument '{name}'"
                    )));
                }
                start.clone()
            }
            RerollAtLeast(_) | Record => Value::None,
            Seen(req) => {
                if *req == int(2) || (*req == int(1) && order.value() < 0) {
                    return unsupported_order();
                }
                Value::tuple(Vec::new())
            }
            HandSums => {
                // `tuple(0 for _ in sizes[0])`
                let first = size_values(sizes)
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::Index("tuple index out of range".into()))?;
                Value::tuple(vec![int(0); tuple_items(&first)?.len()])
            }
        })
    }

    fn next_state_counts(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> Result<Option<Value>> {
        use CatalogueEvaluator::*;
        let counts: Vec<Value> = counts.iter().map(count_value).collect();
        Ok(Some(match self {
            CountEq(k) => {
                let count = one_count(&counts)?;
                if outcome == k {
                    add(state, count)?
                } else {
                    state.clone()
                }
            }
            RerollAtLeast(k) => {
                let count = one_count(&counts)?;
                if count.binary(BinOp::Ge, k)?.truthy() {
                    return Ok(None);
                }
                // `(state or 0) + count`
                let state = if state.truthy() {
                    state.clone()
                } else {
                    int(0)
                };
                add(&state, count)?
            }
            FinalDie(_) => add(state, one_count(&counts)?)?,
            Seen(req) => {
                if *req == int(-1) && order.value() > 0 {
                    return unsupported_order();
                }
                // `any(c > 0 for c in counts)`
                let mut any = false;
                for c in &counts {
                    if c.binary(BinOp::Gt, &int(0))?.truthy() {
                        any = true;
                        break;
                    }
                }
                if any {
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
                let rest = py_sum(counts[1..].iter().cloned().map(Ok), int(0))?;
                add(
                    state,
                    &outcome.binary(BinOp::Mul, &first.binary(BinOp::Sub, &rest)?)?,
                )?
            }
            // `state + (1 if count == 0 else 0)`
            ZeroRuns => add(state, &int(i64::from(*one_count(&counts)? == int(0))))?,
            // `state + sum(counts)`
            ScaledSize { .. } => add(state, &py_sum(counts.iter().cloned().map(Ok), int(0))?)?,
            HandSums => {
                // `tuple(x + outcome * c for x, c in zip(state, counts))`
                let counts = tuple_items(one_count(&counts)?)?;
                let sums = tuple_items(state)?
                    .iter()
                    .zip(counts)
                    .map(|(x, c)| add(x, &outcome.binary(BinOp::Mul, c)?))
                    .collect::<Result<Vec<_>>>()?;
                Value::tuple(sums)
            }
            Record => {
                // `(state or ()) + ((outcome,) + counts,)`
                let mut seen = if state.truthy() {
                    tuple_items(state)?.to_vec()
                } else {
                    Vec::new()
                };
                let mut entry = vec![outcome.clone()];
                entry.extend(counts);
                seen.push(Value::tuple(entry));
                Value::tuple(seen)
            }
        }))
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        match self {
            CatalogueEvaluator::ZeroRuns => consecutive(outcomes),
            _ => Ok(Vec::new()),
        }
    }

    fn final_outcome_counts(
        &self,
        final_state: &Value,
        _: Order,
        _: &[Value],
        sizes: &[Option<Count>],
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
                items.extend(size_values(sizes));
                Final::Outcome(PValue::Value(Value::tuple(items)))
            }
            // `final_state * scale`
            ScaledSize { scale, .. } => outcome(&final_state.binary(BinOp::Mul, scale)?),
            // `(final_state, sizes)`
            Record => Final::Outcome(PValue::Value(Value::tuple(vec![
                final_state.clone(),
                Value::tuple(size_values(sizes)),
            ]))),
            _ => outcome(final_state),
        })
    }
}

/// `EVALUATORS[name](*params).evaluate(*inputs)`: `Die(final_outcomes, final_weights)` through `new_die`, which
/// mixes in final outcomes that are dice.
pub fn run(
    evaluator: &CatalogueEvaluator,
    sources: &[MultisetSource<Value>],
) -> std::result::Result<PValue, PErr> {
    let finals = evaluate_with_depth_check(evaluator, sources, super::evaluate::check_recursion)?;
    let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
    let weights: Vec<Value> = weights.into_iter().map(|w| Value::Int(w.into())).collect();
    new_die(outcomes, &weights)
}
