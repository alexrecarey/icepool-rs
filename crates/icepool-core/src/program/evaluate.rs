//! Evaluation nodes (harness/grammar/evaluate.py): `pool.sum()`, `pool.size()`, ... through the built-in
//! evaluators of `crate::evaluator`, and the shared code that turns evaluator arguments into sources and the
//! final outcomes into a die.

use super::catalogue_evaluators::CatalogueEvaluator;
use super::ops::{die_arg, py_iter, type_error};
use super::pool::{pool, the_pool};
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
use crate::evaluator::{evaluate_with_depth_check, MultisetEvaluator};
use crate::pool::MultisetSource;
use crate::{Die, Pool, Value};

/// `implicit_convert_to_expression`: a pool stays itself; a mapping or sequence (a list, tuple or str; a die is a
/// mapping, which `Pool` refuses) becomes `Pool(arg)`; anything else raises `ImplicitConversionError`.
pub fn implicit_convert(arg: PValue) -> Result<Pool<Value>, PErr> {
    match arg {
        PValue::Pool(p) => Ok(p),
        arg @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Die(_)
        | PValue::Value(Value::Tuple(_) | Value::Str(_))) => {
            the_pool(pool(arg, PValue::Value(Value::from(1)))?)
        }
        PValue::Value(v) => type_error(format!(
            "Argument of type {} cannot be implicitly converted to a Pool.",
            v.type_name()
        )),
        _ => Err(PErr::Unimplemented("this evaluator argument".into())),
    }
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

/// `evaluator.evaluate(*pools)`: one source per pool, and `Die(final_outcomes, final_weights)` (the outer
/// `Die({result: 1})` of `MultisetEvaluatorBase.evaluate` is the result itself).
pub fn run<E>(evaluator: &E, pools: &[Pool<Value>]) -> Result<Die<Value>, PErr>
where
    E: MultisetEvaluator<Value, Final = Value>,
{
    let sources: Vec<MultisetSource<Value>> = pools
        .iter()
        .map(|p| MultisetSource::Pool(p.source()))
        .collect();
    let finals = evaluate_with_depth_check(evaluator, &sources, check_recursion)?;
    let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
    let outcomes = outcomes
        .into_iter()
        .map(|o| die_arg(PValue::Value(o)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Die::new(outcomes, weights)?)
}

fn die(
    evaluator: &impl MultisetEvaluator<Value, Final = Value>,
    pools: &[Pool<Value>],
) -> Result<PValue, PErr> {
    run(evaluator, pools).map(PValue::Die)
}

/// The one-pool evaluation methods of `MultisetExpression` (`pool.sum()` is node `pool_sum`, and so on).
pub fn evaluation(op: &str, p: PValue) -> Result<PValue, PErr> {
    let pools = [the_pool(p)?];
    match op {
        "pool_sum" => die(&SumEvaluator, &pools),
        "pool_size" => die(&SizeEvaluator, &pools),
        "pool_empty" => die(&EmptyEvaluator { any: false }, &pools),
        "pool_any" => die(&EmptyEvaluator { any: true }, &pools),
        "pool_product_of_counts" => die(&ProductOfCountsEvaluator, &pools),
        "pool_highest_outcome_and_count" => die(&HighestOutcomeAndCountEvaluator, &pools),
        "pool_largest_count" => die(&LargestCountEvaluator, &pools),
        "pool_largest_count_and_outcome" => die(&LargestCountAndOutcomeEvaluator, &pools),
        "pool_largest_straight" => die(&LargestStraightEvaluator, &pools),
        "pool_all_straights" => die(&AllStraightsEvaluator, &pools),
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
    let pools = [the_pool(p)?];
    die(
        &AllCountsEvaluator {
            filter: value(filter)?,
        },
        &pools,
    )
}

/// `pool.expand(order=order)`.
pub fn expand(p: PValue, order: PValue) -> Result<PValue, PErr> {
    let pools = [the_pool(p)?];
    // `order != Order.Ascending` (an `IntEnum`, so `1` and `True` are ascending too).
    let ascending = value(order)? == Value::from(1);
    die(&ExpandEvaluator { ascending }, &pools)
}

/// `pool.largest_straight_and_outcome(priority)`.
pub fn largest_straight_and_outcome(p: PValue, priority: PValue) -> Result<PValue, PErr> {
    let pools = [the_pool(p)?];
    let prioritize_highest = match value(priority)? {
        v if v == Value::str("high") => true,
        v if v == Value::str("low") => false,
        _ => return raise(Error::Value("priority must be 'low' or 'high'.".into())),
    };
    die(
        &LargestStraightAndOutcomeEvaluator { prioritize_highest },
        &pools,
    )
}

/// `keep_evaluator.evaluate(pool, index=index)`.
pub fn keep_evaluate(p: PValue, index: PValue) -> Result<PValue, PErr> {
    let pools = [the_pool(p)?];
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
    die(&KeepEvaluator { index }, &pools)
}

/// `pool.count_subset(divisor, empty_divisor=empty_divisor)`.
pub fn count_subset(p: PValue, divisor: PValue, empty_divisor: PValue) -> Result<PValue, PErr> {
    let left = the_pool(p)?;
    let right = implicit_convert(divisor)?;
    die(
        &CountSubsetEvaluator {
            empty_divisor: value(empty_divisor)?,
        },
        &[left, right],
    )
}

/// `pool.issubset(other)`, `pool.issuperset(other)`, `pool.isdisjoint(other)`, and node `pool_compare`'s
/// `cmp` names for the other comparison evaluators.
pub fn compare(op: &str, p: PValue, other: PValue) -> Result<PValue, PErr> {
    let left = the_pool(p)?;
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
    die(&ComparisonEvaluator(comparison), &[left, right])
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
    let pools = py_iter(args)?
        .into_iter()
        .map(implicit_convert)
        .collect::<Result<Vec<_>, _>>()?;
    super::catalogue_evaluators::run(&evaluator, &pools)
}
