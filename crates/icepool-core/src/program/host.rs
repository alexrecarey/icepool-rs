//! Callables and evaluators owned by the host language (the JS facade's functions and `MultisetEvaluator`
//! objects), for the program layer to call back. The core only sees these traits: the host converts arguments and
//! results at its own boundary, as `Value::Host` does for outcomes (value/host.rs).

use std::fmt;
use std::sync::Arc;

use super::catalogue_evaluators;
use super::evaluate::implicit_convert;
use super::multiset_function::{in_function, raw_result};
use super::ops::py_iter;
use super::{PErr, PValue};
use crate::error::Result;
use crate::evaluator::{Final, MultisetEvaluator};
use crate::order::Order;
use crate::pool::Count;
use crate::Value;

/// A host callable passed where Python takes one (`map`, `reroll(which)`, `group_by(key)`, ...): it becomes a
/// [`super::catalogue::Func::Host`], called exactly where the catalogue's Python closures would be.
pub trait HostFunction: Send + Sync + fmt::Debug {
    /// `count_positional_parameters`: (required, total), total `None` for any number (`*args`). `map` infers
    /// `star` from it.
    fn positional(&self) -> (usize, Option<usize>);
    /// `f(*args, **kwargs)`. Its results may be anything a Python callable returns there: values, dice,
    /// `Reroll`, `Break(...)`, tuples holding dice.
    fn call(
        &self,
        args: Vec<PValue>,
        kwargs: Vec<(String, PValue)>,
    ) -> std::result::Result<PValue, PErr>;
}

/// A host `MultisetEvaluator`: the four methods of multiset_evaluator.py, over Python-style values. Counts and
/// sizes arrive as `int`s, tuples of `int`s (a `MultiDeal` input) or `None` (a size with negative counts), as the
/// Python methods receive them; the host's defaults (no `initial_state`: `None`, no `final_outcome`: the final
/// state) are its own business.
pub trait HostEvaluator: Send + Sync + fmt::Debug {
    fn initial_state(&self, order: Order, outcomes: &[Value], sizes: &[Value]) -> Result<Value>;
    /// `Ok(None)` is `icepool.Reroll`.
    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Value],
    ) -> Result<Option<Value>>;
    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>>;
    /// A value or a die; `Final::Reroll` for `Reroll`, `Final::None` for `None`.
    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Value],
    ) -> Result<Final<PValue>>;
}

/// A count or size as the Python object the evaluator methods receive.
fn count_value(c: &Count) -> Value {
    match c {
        Count::Int(c) => Value::from(*c),
        Count::Tuple(cs) => Value::tuple(cs.iter().map(|&c| Value::from(c)).collect()),
    }
}

fn size_values(sizes: &[Option<Count>]) -> Vec<Value> {
    sizes
        .iter()
        .map(|s| s.as_ref().map_or(Value::None, count_value))
        .collect()
}

fn int_sizes(sizes: &[Option<i64>]) -> Vec<Value> {
    sizes
        .iter()
        .map(|s| s.map_or(Value::None, Value::from))
        .collect()
}

/// The driver's view of a [`HostEvaluator`]. States are `Value`s, so the host's states are hashed and merged as
/// Python would (the JS facade turns its states into values, or into keys it serialises, before they get here).
/// It shares the evaluator: inside a `@multiset_function` body the evaluation is recorded for later.
struct Host(Arc<dyn HostEvaluator>);

impl MultisetEvaluator<Value> for Host {
    type State = Value;
    type Final = PValue;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<Value> {
        self.0.initial_state(order, outcomes, &int_sizes(sizes))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[i64],
    ) -> Result<Option<Value>> {
        let counts: Vec<Value> = counts.iter().map(|&c| Value::from(c)).collect();
        self.0.next_state(state, order, outcome, &counts)
    }

    fn extra_outcomes(&self, outcomes: &[Value]) -> Result<Vec<Value>> {
        self.0.extra_outcomes(outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<i64>],
    ) -> Result<Final<PValue>> {
        self.0
            .final_outcome(final_state, order, outcomes, &int_sizes(sizes))
    }

    // Tuple counts (a `MultiDeal` input) reach the host as tuples, as they reach a Python evaluator.
    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Value> {
        self.0.initial_state(order, outcomes, &size_values(sizes))
    }

    fn next_state_counts(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Count],
    ) -> Result<Option<Value>> {
        let counts: Vec<Value> = counts.iter().map(count_value).collect();
        self.0.next_state(state, order, outcome, &counts)
    }

    fn final_outcome_counts(
        &self,
        final_state: &Value,
        order: Order,
        outcomes: &[Value],
        sizes: &[Option<Count>],
    ) -> Result<Final<PValue>> {
        self.0
            .final_outcome(final_state, order, outcomes, &size_values(sizes))
    }
}

/// `evaluator.evaluate(*args)` for a host evaluator: each argument implicitly converted to an expression (a list
/// or tuple of outcomes becomes a pool), then evaluated as `EVALUATORS[...]` are. Inside a `@multiset_function`
/// body (an argument holds a parameter) it is a `MultisetFunctionRawResult` recording the evaluator instead,
/// evaluated when the function is.
pub fn evaluate(
    evaluator: &Arc<dyn HostEvaluator>,
    args: PValue,
) -> std::result::Result<PValue, PErr> {
    let exprs = py_iter(args)?
        .into_iter()
        .map(implicit_convert)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let host = Host(evaluator.clone());
    if in_function(&exprs) {
        return Ok(raw_result(host, &exprs));
    }
    catalogue_evaluators::run(&host, &exprs)
}

/// `icepool.multiset_function(f)` for a host function: `f` is called with one parameter expression per
/// positional parameter (`params` required ones, more if `var`, the `*args` parameter), then the keyword-only
/// arguments `kwonly` by name, and returns evaluations of them (or a tuple of evaluations, or constants), as a
/// Python `@multiset_function` body does. The host states the signature, which Python reads from the function.
pub fn multiset_function(
    f: Arc<dyn HostFunction>,
    params: usize,
    var: bool,
    kwonly: Vec<String>,
) -> PValue {
    super::multiset_function::host_function(f, params, var, kwonly)
}

/// Whether a value is a multiset tuple expression (a `MultiDeal`, a tuple parameter, a slice of one): the host
/// gives these their own class (indexing yields one hand).
pub fn is_tuple_expression(v: &PValue) -> bool {
    match v {
        PValue::MultiDeal(_) => true,
        PValue::Expr(e) => matches!(e.tuple_len(), Ok(Some(_))),
        _ => false,
    }
}

/// A host callable as a program value (Python's function object).
pub fn function(f: Arc<dyn HostFunction>) -> PValue {
    PValue::Func(Arc::new(super::catalogue::Func::Host(f)))
}
