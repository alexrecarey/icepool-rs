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
use crate::Int;
use crate::Value;

/// A host callable passed where Python takes one (`map`, `reroll(which)`, `group_by(key)`, ...): it becomes a
/// `catalogue::Func::Host`, called exactly where the catalogue's Python closures would be.
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
        Count::Int(c) => Value::from(c),
        Count::Tuple(cs) => Value::tuple(cs.iter().map(Value::from).collect()),
    }
}

fn size_values(sizes: &[Option<Count>]) -> Vec<Value> {
    sizes
        .iter()
        .map(|s| s.as_ref().map_or(Value::None, count_value))
        .collect()
}

fn int_sizes(sizes: &[Option<Int>]) -> Vec<Value> {
    sizes
        .iter()
        .map(|s| s.as_ref().map_or(Value::None, Value::from))
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
        sizes: &[Option<Int>],
    ) -> Result<Value> {
        self.0.initial_state(order, outcomes, &int_sizes(sizes))
    }

    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &Value,
        counts: &[Int],
    ) -> Result<Option<Value>> {
        let counts: Vec<Value> = counts.iter().map(Value::from).collect();
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
        sizes: &[Option<Int>],
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

/// `icepool.from_rv(rv, outcomes, denominator, **kwargs)` for a host random variable, duck-typed as Python's
/// (`scipy.stats`-like, see harness/catalogue/rvs.py): `cdf` is its bound `rv.cdf`, called once with a sequence
/// (and the kwargs) and returning an iterable of probabilities; `has_pdf` is `hasattr(rv, 'pdf')`, which makes
/// it continuous (the CDF is taken at the midpoints between the outcomes).
pub fn from_rv(
    cdf: Arc<dyn HostFunction>,
    has_pdf: bool,
    outcomes: PValue,
    denominator: PValue,
    kwargs: Vec<(String, PValue)>,
) -> std::result::Result<PValue, PErr> {
    super::function::from_cdf(
        has_pdf,
        |xs| cdf.call(vec![xs], kwargs),
        outcomes,
        denominator,
    )
}

/// A host mapping (a JS `Map`) as Python's `dict(items)`: keys are equal when Python's `==` says so (`1`, `1.0`
/// and `True` are one key: the first one's position, the last one's value), not by the host's own key equality.
/// Where Python takes a callable or a mapping (`map`, `reroll`, `group_by`, ...) it acts as `mapping.get(o, o)`,
/// exactly as the catalogue's `mapping`.
pub fn mapping(items: PValue) -> std::result::Result<PValue, PErr> {
    let items = super::ops::dict_items(items)?;
    Ok(PValue::Func(Arc::new(super::catalogue::Func::Mapping(
        items.into_iter().collect(),
    ))))
}

#[cfg(test)]
mod tests {
    //! A host multiset function must trace and evaluate exactly as the same body written as a program (`mf_def`),
    //! host evaluators inside it included (the JS facade's tests check the same against the oracle).
    use super::*;
    use crate::program::{call, eval, wire::result_of};
    use serde_json::json;

    type BodyFn = fn(Vec<PValue>, Vec<(String, PValue)>) -> std::result::Result<PValue, PErr>;

    struct Body(BodyFn);

    impl fmt::Debug for Body {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("Body")
        }
    }

    impl HostFunction for Body {
        fn positional(&self) -> (usize, Option<usize>) {
            (0, None)
        }
        fn call(
            &self,
            args: Vec<PValue>,
            kwargs: Vec<(String, PValue)>,
        ) -> std::result::Result<PValue, PErr> {
            (self.0)(args, kwargs)
        }
    }

    /// Counts the elements of outcomes over 2 (`state + count`), as a host evaluator.
    #[derive(Debug)]
    struct CountHigh;

    impl HostEvaluator for CountHigh {
        fn initial_state(&self, _: Order, _: &[Value], _: &[Value]) -> Result<Value> {
            Ok(Value::from(0))
        }
        fn next_state(
            &self,
            state: &Value,
            _: Order,
            o: &Value,
            counts: &[Value],
        ) -> Result<Option<Value>> {
            let int = |v: &Value| match v {
                Value::Int(i) => i64::try_from(i).expect("a small int"),
                _ => panic!("an int"),
            };
            let high = int(o) > 2;
            Ok(Some(Value::from(
                int(state) + if high { int(&counts[0]) } else { 0 },
            )))
        }
        fn extra_outcomes(&self, _: &[Value]) -> Result<Vec<Value>> {
            Ok(Vec::new())
        }
        fn final_outcome(
            &self,
            s: &Value,
            _: Order,
            _: &[Value],
            _: &[Value],
        ) -> Result<Final<PValue>> {
            Ok(Final::Outcome(PValue::Value(s.clone())))
        }
    }

    fn s(x: &str) -> PValue {
        PValue::Value(Value::str(x))
    }

    fn pools() -> PValue {
        PValue::List(vec![
            eval(&json!({"op": "die_pool", "die": {"op": "d", "sides": 4}, "rolls": 2})).unwrap(),
            eval(&json!({"op": "die_pool", "die": {"op": "d", "sides": 3}, "rolls": 1})).unwrap(),
        ])
    }

    fn mf_call(f: PValue, kwargs: Vec<(PValue, PValue)>) -> serde_json::Value {
        let kwargs = PValue::List(
            kwargs
                .into_iter()
                .map(|(k, v)| PValue::tuple(vec![k, v]))
                .collect(),
        );
        result_of(
            &call(
                "mf_call_kw",
                vec![("f", f), ("args", pools()), ("kwargs", kwargs)],
            )
            .unwrap_or_else(|e| panic!("{e:?}")),
        )
    }

    #[test]
    fn host_body_traces_as_a_program_body() {
        // `(a - b).size(), (a + b).sum()` as a joint result.
        let host = multiset_function(
            Arc::new(Body(|args, _| {
                let diff = call(
                    "mx_binary",
                    vec![
                        ("a", args[0].clone()),
                        ("o", s("-")),
                        ("b", args[1].clone()),
                    ],
                )?;
                let both = call(
                    "mx_binary",
                    vec![
                        ("a", args[0].clone()),
                        ("o", s("+")),
                        ("b", args[1].clone()),
                    ],
                )?;
                Ok(PValue::tuple(vec![
                    call("mx_evaluate", vec![("expr", diff), ("method", s("size"))])?,
                    call("mx_evaluate", vec![("expr", both), ("method", s("sum"))])?,
                ]))
            })),
            2,
            false,
            Vec::new(),
        );
        let var = |i| json!({"op": "mf_var", "i": i});
        let program = eval(&json!({"op": "mf_def", "params": 2, "var": {"bool": false}, "body": {"op": "tuple", "items": [
            {"op": "mx_evaluate", "method": {"str": "size"}, "expr": {"op": "mx_binary", "a": var(0), "o": {"str": "-"}, "b": var(1)}},
            {"op": "mx_evaluate", "method": {"str": "sum"}, "expr": {"op": "mx_binary", "a": var(0), "o": {"str": "+"}, "b": var(1)}},
        ]}}))
        .unwrap();
        let expected = mf_call(program, Vec::new());
        assert_eq!(expected["ok"], json!(true));
        assert_eq!(mf_call(host, Vec::new()), expected);
    }

    #[test]
    fn host_evaluator_in_a_body_is_recorded() {
        // `CountHigh().evaluate(a | b)` and a keyword-only argument returned as a constant.
        let host = multiset_function(
            Arc::new(Body(|args, kwargs| {
                let union = call(
                    "mx_binary",
                    vec![
                        ("a", args[0].clone()),
                        ("o", s("|")),
                        ("b", args[1].clone()),
                    ],
                )?;
                let evaluator: Arc<dyn HostEvaluator> = Arc::new(CountHigh);
                let recorded = evaluate(&evaluator, PValue::List(vec![union]))?;
                assert!(matches!(recorded, PValue::RawResult(_)));
                Ok(PValue::tuple(vec![recorded, kwargs[0].1.clone()]))
            })),
            2,
            false,
            vec!["k".into()],
        );
        // The same evaluation outside a function, with the constant beside it.
        let union = call(
            "mx_binary",
            vec![
                ("a", py_iter(pools()).unwrap()[0].clone()),
                ("o", s("|")),
                ("b", py_iter(pools()).unwrap()[1].clone()),
            ],
        )
        .unwrap();
        let evaluator: Arc<dyn HostEvaluator> = Arc::new(CountHigh);
        let PValue::Die(direct) = evaluate(&evaluator, PValue::List(vec![union])).unwrap() else {
            panic!("not a die")
        };
        let expected: Vec<_> = direct
            .items()
            .iter()
            .map(|(o, q)| json!([{"tuple": [result_of(&PValue::Value(o.clone()))["value"], 7]}, q.to_string().parse::<u64>().unwrap()]))
            .collect();
        let result = mf_call(host, vec![(s("k"), PValue::Value(Value::from(7)))]);
        assert_eq!(result["die"], json!(expected));
    }
}
