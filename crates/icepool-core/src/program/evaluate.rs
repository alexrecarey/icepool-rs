//! Evaluation nodes (harness/grammar/evaluate.py): `pool.sum()`, `pool.size()`, ... through the built-in
//! evaluators of `crate::evaluator`, and the shared code that turns evaluator arguments into sources and the
//! final outcomes into a die.

use super::catalogue_evaluators::CatalogueEvaluator;
use super::multiset_function::{in_function, raw_result};
use super::expression::{as_expression, as_tuple_expression, prepare, the_expression, MExpr};
use super::ops::{die_arg, py_iter, type_error};
use super::pool::pool;
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, ProductOfCountsEvaluator, SizeEvaluator, SumEvaluator,
};
use crate::evaluator::comparison::{Comparison, ComparisonEvaluator};
use crate::evaluator::keep::KeepEvaluator;
use crate::evaluator::poker::{
    AllCountsEvaluator, AllStraightsEvaluator, CountSubsetEvaluator,
    HighestOutcomeAndCountEvaluator, LargestCountAndOutcomeEvaluator, LargestCountEvaluator,
    LargestStraightAndOutcomeEvaluator, LargestStraightEvaluator,
};
use crate::evaluator::{evaluate_expressions, MultisetEvaluator};
use crate::{Die, Value};

/// `implicit_convert_to_expression`: an expression stays itself; a mapping or sequence (a list, tuple or str; a
/// die is a mapping, which `Pool` refuses) becomes `Pool(arg)`; anything else raises `ImplicitConversionError`.
pub fn implicit_convert(arg: PValue) -> Result<MExpr, PErr> {
    if let Some(e) = as_expression(&arg).or_else(|| as_tuple_expression(&arg)) {
        // A multiset tuple expression is an expression too (of another kind).
        return Ok(e);
    }
    match arg {
        // A deck is a mapping, but `Pool(deck)` refuses it.
        PValue::Deck(_) => raise(Error::Value(
            "A Pool cannot be constructed with a Deck argument.".into(),
        )),
        arg @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Die(_)
        | PValue::Value(Value::Tuple(_) | Value::Str(_))) => {
            the_expression(pool(arg, PValue::Value(Value::from(1)))?)
        }
        PValue::Value(v) => type_error(format!(
            "Argument of type {} cannot be implicitly converted to a Pool.",
            v.type_name()
        )),
        _ => Err(PErr::Unimplemented("this evaluator argument".into())),
    }
}

/// The `self` of an evaluation method (`pool.sum()`, ...): a multiset tuple expression (a `MultiDeal`, a tuple
/// parameter) has none of them.
fn method_self(p: PValue) -> Result<MExpr, PErr> {
    if as_tuple_expression(&p).is_some() {
        return type_error("AttributeError: a multiset tuple expression has no evaluation methods");
    }
    the_expression(p)
}

/// Python's evaluation recurses once per outcome (`evaluate_backward` / `evaluate_forward`) and raises
/// `RecursionError` near 1000 outcomes. Where exactly depends on the caller's stack (the program's nesting, the
/// harness's own frames) and on the process's history: popping a die computes `_popped_max` (a `cached_property`
/// several frames deep) unless an earlier evaluation already did. Measured: a top-level `pool.sum()` of 981 outcomes
/// succeeds with warm dice and fails at 982; with cold dice six levels down 970 still succeeds. The port raises from
/// `SURE_RECURSION_ERROR` and says unimplemented in between (the answer is not knowable from the program).
pub(super) fn check_recursion(outcomes: usize) -> Result<(), Error> {
    const SAFE_RECURSION: usize = 930;
    const SURE_RECURSION_ERROR: usize = 990;
    if outcomes >= SURE_RECURSION_ERROR {
        return Err(Error::Value(
            "RecursionError: maximum recursion depth exceeded in evaluate_backward".into(),
        ));
    }
    if outcomes > SAFE_RECURSION {
        return Err(Error::Unsupported(
            "a multiset evaluation close to Python's recursion limit".into(),
        ));
    }
    Ok(())
}

/// `evaluator.evaluate(*expressions)`: each prepared (`_prepare`), and `Die(final_outcomes, final_weights)` (the
/// outer `Die({result: 1})` of `MultisetEvaluatorBase.evaluate` is the result itself). Inside a
/// `@multiset_function` body, the raw result recording the evaluation instead.
pub fn run<E>(evaluator: E, exprs: &[MExpr]) -> Result<PValue, PErr>
where
    E: MultisetEvaluator<Value, Final = Value> + Send + Sync + 'static,
    E::State: 'static,
{
    if in_function(exprs) {
        return Ok(raw_result(evaluator, exprs));
    }
    let (flats, sources) = prepare(exprs)?;
    let finals = evaluate_expressions(&evaluator, &flats, &sources, check_recursion)?;
    let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
    let outcomes = outcomes
        .into_iter()
        .map(|o| die_arg(PValue::Value(o)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PValue::Die(Die::new(outcomes, weights)?))
}

fn die<E>(evaluator: E, pools: &[MExpr]) -> Result<PValue, PErr>
where
    E: MultisetEvaluator<Value, Final = Value> + Send + Sync + 'static,
    E::State: 'static,
{
    run(evaluator, pools)
}

/// The one-pool evaluation methods of `MultisetExpression` (`pool.sum()` is node `pool_sum`, and so on).
pub fn evaluation(op: &str, p: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    match op {
        "pool_sum" => die(SumEvaluator, &pools),
        "pool_size" => die(SizeEvaluator, &pools),
        "pool_empty" => die(EmptyEvaluator { any: false }, &pools),
        "pool_any" => die(EmptyEvaluator { any: true }, &pools),
        "pool_product_of_counts" => die(ProductOfCountsEvaluator, &pools),
        "pool_highest_outcome_and_count" => die(HighestOutcomeAndCountEvaluator, &pools),
        "pool_largest_count" => die(LargestCountEvaluator, &pools),
        "pool_largest_count_and_outcome" => die(LargestCountAndOutcomeEvaluator, &pools),
        "pool_largest_straight" => die(LargestStraightEvaluator, &pools),
        "pool_all_straights" => die(AllStraightsEvaluator, &pools),
        _ => Err(PErr::Unimplemented(op.into())),
    }
}

fn value(x: PValue) -> Result<Value, PErr> {
    match x {
        PValue::Value(v) => Ok(v),
        _ => Err(PErr::Unimplemented(
            "a non-value evaluator parameter".into(),
        )),
    }
}

/// `pool.all_counts(filter=filter)`.
pub fn all_counts(p: PValue, filter: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    die(AllCountsEvaluator {
            filter: value(filter)?,
        },
        &pools,
    )
}

/// `pool.expand(order=order)`.
pub fn expand(p: PValue, order: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    // `order != Order.Ascending` (an `IntEnum`, so `1` and `True` are ascending too).
    let ascending = value(order)? == Value::from(1);
    die(ExpandEvaluator { ascending }, &pools)
}

/// `pool.largest_straight_and_outcome(priority)`.
pub fn largest_straight_and_outcome(p: PValue, priority: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    let prioritize_highest = match value(priority)? {
        v if v == Value::str("high") => true,
        v if v == Value::str("low") => false,
        _ => return raise(Error::Value("priority must be 'low' or 'high'.".into())),
    };
    die(LargestStraightAndOutcomeEvaluator { prioritize_highest },
        &pools,
    )
}

/// `keep_evaluator.evaluate(pool, index=index)`.
pub fn keep_evaluate(p: PValue, index: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    let index = match value(index)? {
        Value::None => None,
        Value::Int(i) => Some(
            i64::try_from(&i)
                .map_err(|_| PErr::Unimplemented("a keep index beyond 64 bits".into()))?,
        ),
        _ => {
            return Err(PErr::Unimplemented(
                "a keep index that is not an int".into(),
            ))
        }
    };
    die(KeepEvaluator { index }, &pools)
}

/// `pool.count_subset(divisor, empty_divisor=empty_divisor)`.
pub fn count_subset(p: PValue, divisor: PValue, empty_divisor: PValue) -> Result<PValue, PErr> {
    let left = method_self(p)?;
    let right = implicit_convert(divisor)?;
    die(CountSubsetEvaluator {
            empty_divisor: value(empty_divisor)?,
        },
        &[left, right],
    )
}

/// `pool.issubset(other)`, `pool.issuperset(other)`, `pool.isdisjoint(other)`, and node `pool_compare`'s
/// `cmp` names for the other comparison evaluators.
pub fn compare(op: &str, p: PValue, other: PValue) -> Result<PValue, PErr> {
    let left = method_self(p)?;
    let right = implicit_convert(other)?;
    let comparison = match op {
        "pool_issubset" => Comparison::IsSubset,
        "pool_issuperset" => Comparison::IsSuperset,
        "pool_isdisjoint" => Comparison::IsDisjointSet,
        "proper_subset" => Comparison::IsProperSubset,
        "proper_superset" => Comparison::IsProperSuperset,
        "equal_set" => Comparison::IsEqualSet,
        "not_equal_set" => Comparison::IsNotEqualSet,
        _ => return Err(PErr::Unimplemented(op.into())),
    };
    die(ComparisonEvaluator(comparison), &[left, right])
}

/// `EVALUATORS[name](*params).evaluate(*args, **kwargs)` (harness/catalogue/evaluators.py).
pub fn catalogue(
    name: PValue,
    params: PValue,
    args: PValue,
    kwargs: Vec<(String, PValue)>,
) -> Result<PValue, PErr> {
    let PValue::Value(Value::Str(name)) = name else {
        return raise(Error::Key("catalogue names are strs".into()));
    };
    let evaluator = CatalogueEvaluator::new(&name, py_iter(params)?)?.with_kwargs(kwargs)?;
    let exprs = py_iter(args)?
        .into_iter()
        .map(implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    if in_function(&exprs) {
        return Ok(raw_result(evaluator, &exprs));
    }
    super::catalogue_evaluators::run(&evaluator, &exprs)
}

/// `getattr(expr, method)()` for the evaluation methods of `MultisetExpression` that take no argument (node
/// `mx_evaluate`), with their defaults: `expand(order=Order.Ascending)`, `all_counts(filter=1)`.
pub fn method_evaluation(method: &str, e: PValue) -> Result<PValue, PErr> {
    match method {
        "expand" => expand(e, PValue::Value(Value::from(1))),
        "all_counts" => all_counts(e, PValue::Value(Value::from(1))),
        "sum"
        | "size"
        | "empty"
        | "any"
        | "product_of_counts"
        | "highest_outcome_and_count"
        | "largest_count"
        | "largest_count_and_outcome"
        | "largest_straight"
        | "all_straights" => evaluation(&format!("pool_{method}"), e),
        _ => Err(PErr::Unimplemented(format!("evaluation method {method}"))),
    }
}
