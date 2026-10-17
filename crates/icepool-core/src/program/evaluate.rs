//! Evaluation nodes (harness/grammar/evaluate.py): `pool.sum()`, `pool.size()`, ... through the built-in
//! evaluators of `crate::evaluator`, and the shared code that turns evaluator arguments into sources and the
//! final outcomes into a die.

use rustc_hash::FxHashSet;

use super::catalogue::{truthy, Func};
use super::catalogue_evaluators::CatalogueEvaluator;
use super::expression::{
    as_expression, as_tuple_expression, prepare, same_expression, the_expression, MExpr,
};
use super::map::the_die;
use super::multiset_function::{in_function, raw_result};
use super::ops::{die_arg, new_die, py_iter, type_error, Key};
use super::pool::pool;
use super::{raise, PErr, PValue};
use crate::error::Error;
use crate::evaluator::argsort::{ArgsortEvaluator, ArgsortGroupedEvaluator, Tie};
use crate::evaluator::basic::{
    EmptyEvaluator, ExpandEvaluator, MappedSumEvaluator, ProductOfCountsEvaluator, SizeEvaluator,
    SumEvaluator,
};
use crate::evaluator::comparison::{Comparison, ComparisonEvaluator};
use crate::evaluator::keep::KeepEvaluator;
use crate::evaluator::lexi::LexiComparisonEvaluator;
use crate::evaluator::poker::{
    AllCountsEvaluator, AllStraightsEvaluator, AllStraightsReduceCountsEvaluator,
    CountSubsetEvaluator, HighestOutcomeAndCountEvaluator, LargestCountAndOutcomeEvaluator,
    LargestCountEvaluator, LargestCountWithWildEvaluator, LargestStraightAndOutcomeEvaluator,
    LargestStraightEvaluator,
};
use crate::evaluator::{evaluate_expressions, MultisetEvaluator};
use crate::lexi::compute_lexi_tuple_with_extra;
use crate::order::Order;
use crate::{Die, Quantity, Value};

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
        // An `AgainExpression` is neither a mapping nor a sequence.
        PValue::Again(_) => {
            type_error("Argument of type AgainExpression cannot be implicitly converted to a Pool.")
        }
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
        return Err(Error::Recursion(
                "maximum recursion depth exceeded in evaluate_backward".into(),
        ));
    }
    if outcomes > SAFE_RECURSION {
        return Err(Error::Unsupported(
            "a multiset evaluation close to Python's recursion limit".into(),
        ));
    }
    Ok(())
}

/// `evaluator.evaluate(*expressions)`: each prepared evaluation (`_prepare`; several with a mixture) gives
/// `Die(final_outcomes, final_weights)`, and those are mixed by weight ([`mix`]). Inside a `@multiset_function`
/// body, the raw result recording the evaluation instead.
pub fn run<E>(evaluator: E, exprs: &[MExpr]) -> Result<PValue, PErr>
where
    E: MultisetEvaluator<Value, Final = Value> + Send + Sync + 'static,
    E::State: 'static,
{
    if in_function(exprs) {
        return Ok(raw_result(evaluator, exprs));
    }
    let mut results = Vec::new();
    for (flats, sources, weight) in prepare(exprs)? {
        let finals = evaluate_expressions(&evaluator, &flats, &sources, check_recursion)?;
        let (outcomes, weights): (Vec<_>, Vec<_>) = finals.into_iter().unzip();
        let outcomes = outcomes
            .into_iter()
            .map(|o| die_arg(PValue::Value(o)))
            .collect::<Result<Vec<_>, _>>()?;
        results.push((PValue::Die(Die::new(outcomes, weights)?), weight));
    }
    mix(results)
}

/// The end of `MultisetEvaluatorBase.evaluate`: `final_data[result] += weight` for each prepared evaluation's die
/// (equal dice merge, the first kept), then `Die(final_data)` (one die with weight 1 is itself).
pub(super) fn mix(results: Vec<(PValue, Quantity)>) -> Result<PValue, PErr> {
    let mut data: Vec<(PValue, Quantity)> = Vec::new();
    for (result, weight) in results {
        let key = the_die(result.clone())?;
        match data
            .iter_mut()
            .find(|(d, _)| matches!(the_die(d.clone()), Ok(d) if d == key))
        {
            Some((_, w)) => *w += weight,
            None => data.push((result, weight)),
        }
    }
    let (dice, weights): (Vec<_>, Vec<_>) = data.into_iter().unzip();
    let weights: Vec<Value> = weights.into_iter().map(|w| Value::Int(w.into())).collect();
    new_die(dice, &weights)
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
    die(
        AllCountsEvaluator {
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
    let evaluator = straight_and_outcome_evaluator(priority)?;
    die(evaluator, &pools)
}

/// `LargestStraightAndOutcomeEvaluator(priority)`: its constructor checks the priority, as the method does.
fn straight_and_outcome_evaluator(
    priority: PValue,
) -> Result<LargestStraightAndOutcomeEvaluator, PErr> {
    let prioritize_highest = match value(priority)? {
        v if v == Value::str("high") => true,
        v if v == Value::str("low") => false,
        _ => return raise(Error::Value("priority must be 'low' or 'high'.".into())),
    };
    Ok(LargestStraightAndOutcomeEvaluator { prioritize_highest })
}

/// `icepool.evaluator.LargestStraightAndOutcomeEvaluator(priority).evaluate(arg)`: the evaluator is built (and
/// the priority checked) before its argument is converted.
pub fn straight_and_outcome_evaluate(priority: PValue, arg: PValue) -> Result<PValue, PErr> {
    let evaluator = straight_and_outcome_evaluator(priority)?;
    die(evaluator, &[implicit_convert(arg)?])
}

/// `keep_evaluator.evaluate(pool, index=index)`.
pub fn keep_evaluate(p: PValue, index: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    let index = match value(index)? {
        Value::None => None,
        Value::Int(i) => Some(i.into()),
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
    die(
        CountSubsetEvaluator {
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

// --- K6c: evaluations with callables, argsort, lexi, the comparison operators ----------------------------------

/// A program-layer error inside a core evaluator: an unimplemented feature stays unsupported.
fn core_error(e: PErr) -> Error {
    match e {
        PErr::Raise(e) => e,
        PErr::Unimplemented(what) => Error::Unsupported(what),
    }
}

/// The result of a catalogue callable inside an evaluator, as a value: a die or `Reroll` as an evaluation state
/// is beyond the port's `Value` states.
fn plain(x: PValue) -> crate::error::Result<Value> {
    match x {
        PValue::Value(v) => Ok(v),
        _ => Err(Error::Unsupported(
            "a callable returning a die or Reroll inside an evaluation".into(),
        )),
    }
}

/// Calls a catalogue function with values. A mapping (a dict) is not callable.
fn call_values(f: &Func, args: &[Value]) -> crate::error::Result<Value> {
    if matches!(f, Func::Mapping(_)) {
        return Err(Error::Type("'dict' object is not callable".into()));
    }
    plain(
        f.call(args.iter().cloned().map(PValue::Value).collect())
            .map_err(core_error)?,
    )
}

/// `pool.sum(map=map)`: `None` is the plain sum; a callable or mapping maps each outcome just before summing (for a
/// mapping, unmapped outcomes are left as-is: `Func::call` is `mapping.get(o, o)`).
pub fn sum_map(p: PValue, map: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    match map {
        PValue::Value(Value::None) => die(SumEvaluator, &pools),
        PValue::Func(f) => die(
            MappedSumEvaluator {
                map: move |o: &Value| {
                    plain(f.call(vec![PValue::Value(o.clone())]).map_err(core_error)?)
                },
            },
            &pools,
        ),
        // `{k: v for k, v in map.items()}`
        PValue::Value(v) => type_error(format!(
            "AttributeError: '{}' object has no attribute 'items'",
            v.type_name()
        )),
        _ => Err(PErr::Unimplemented("this sum map".into())),
    }
}

type Select = Box<dyn Fn(&Value) -> crate::error::Result<Value> + Send + Sync>;

/// `make_select_function(which)`: `None` selects nothing (`False`), a callable is called with the outcome (its
/// result as is: the evaluator takes its truth value where Python does; a result that is not a plain value, a die,
/// gives its truth value now), any other collection becomes a `frozenset` tested with `in`.
fn make_select_function(which: PValue) -> Result<Select, PErr> {
    let set: FxHashSet<Key> = match which {
        PValue::Value(Value::None) => return Ok(Box::new(|_| Ok(Value::Bool(false)))),
        PValue::Func(f) => match &*f {
            // A dict is not callable: `frozenset(dict)` is its keys.
            Func::Mapping(m) => m.keys().cloned().collect(),
            _ => {
                return Ok(Box::new(move |o: &Value| {
                    match f.call(vec![PValue::Value(o.clone())]).map_err(core_error)? {
                        PValue::Value(v) => Ok(v),
                        p => truthy(&p).map(Value::Bool).map_err(core_error),
                    }
                }))
            }
        },
        PValue::List(items) => items
            .into_iter()
            .map(|x| match x {
                PValue::Value(v) => Ok(Key::Value(v)),
                PValue::Die(d) => Ok(Key::Die(d)),
                PValue::List(_) => type_error("unhashable type: 'list'"),
                _ => Err(PErr::Unimplemented("this wild collection element".into())),
            })
            .collect::<Result<_, _>>()?,
        PValue::Value(Value::Tuple(t)) => t.iter().cloned().map(Key::Value).collect(),
        PValue::Value(Value::Str(s)) => s
            .chars()
            .map(|c| Key::Value(Value::str(&c.to_string())))
            .collect(),
        PValue::Die(d) => d.outcomes().cloned().map(Key::Value).collect(),
        PValue::Value(
            v @ (Value::Int(_) | Value::Bool(_) | Value::Float(_) | Value::Fraction(_)),
        ) => {
            return type_error(format!("'{}' object is not iterable", v.type_name()));
        }
        _ => return Err(PErr::Unimplemented("this wild selector".into())),
    };
    Ok(Box::new(move |o: &Value| {
        Ok(Value::Bool(set.contains(&Key::Value(o.clone()))))
    }))
}

/// `pool.largest_count(wild=wild, wild_low=wild_low, wild_high=wild_high)`: without any wild, the plain largest
/// count.
pub fn largest_count_wild(
    p: PValue,
    wild: PValue,
    wild_low: PValue,
    wild_high: PValue,
) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    let none = |x: &PValue| matches!(x, PValue::Value(Value::None));
    if none(&wild) && none(&wild_low) && none(&wild_high) {
        return die(LargestCountEvaluator, &pools);
    }
    die(
        LargestCountWithWildEvaluator {
            wild: make_select_function(wild)?,
            wild_low: make_select_function(wild_low)?,
            wild_high: make_select_function(wild_high)?,
        },
        &pools,
    )
}

/// `pool.all_straights_reduce_counts(reducer)`.
pub fn all_straights_reduce_counts(p: PValue, reducer: PValue) -> Result<PValue, PErr> {
    let pools = [method_self(p)?];
    let PValue::Func(f) = reducer else {
        // Anything else is not callable, which only matters once a straight continues.
        return Err(PErr::Unimplemented(
            "a reducer that is not a catalogue function".into(),
        ));
    };
    die(
        AllStraightsReduceCountsEvaluator {
            reducer: move |a: &Value, b: &Value| call_values(&f, &[a.clone(), b.clone()]),
        },
        &pools,
    )
}

/// An `int | None` evaluator parameter (`limit`).
fn optional_int(x: PValue) -> Result<Option<i64>, PErr> {
    match value(x)? {
        Value::None => Ok(None),
        Value::Int(i) => {
            Ok(Some(i64::try_from(&i).map_err(|_| {
                PErr::Unimplemented("a limit beyond 64 bits".into())
            })?))
        }
        _ => Err(PErr::Unimplemented("a limit that is not an int".into())),
    }
}

/// `MultisetExpression.argsort(*args, order=, limit=, limit_pad=, tie=)` (called on the class, so the first
/// argument may be a sequence too). All arguments are converted before `tie` is looked at.
pub fn argsort(
    args: PValue,
    order: PValue,
    limit: PValue,
    limit_pad: PValue,
    tie: PValue,
) -> Result<PValue, PErr> {
    let exprs = argsort_args(args)?;
    let tie = match value(tie)? {
        v if v == Value::str("drop") => Tie::Drop,
        v if v == Value::str("left") => Tie::Left,
        v if v == Value::str("right") => Tie::Right,
        _ => {
            return raise(Error::Value(
                "tie must be \"drop\", \"left\", or \"right\".".into(),
            ))
        }
    };
    let limit_pad = match value(limit_pad)? {
        Value::None => None,
        v => Some(v),
    };
    let evaluator = ArgsortEvaluator {
        tie,
        order: value(order)?,
        limit: optional_int(limit)?,
        limit_pad,
    };
    die(evaluator, &exprs)
}

/// `MultisetExpression.argsort_grouped(*args, order=, limit=)`.
pub fn argsort_grouped(args: PValue, order: PValue, limit: PValue) -> Result<PValue, PErr> {
    let exprs = argsort_args(args)?;
    let evaluator = ArgsortGroupedEvaluator {
        order: value(order)?,
        limit: optional_int(limit)?,
    };
    die(evaluator, &exprs)
}

/// `self, *args` of the argsort methods, each `implicit_convert_to_expression`ed.
fn argsort_args(args: PValue) -> Result<Vec<MExpr>, PErr> {
    let args = py_iter(args)?;
    if args.is_empty() {
        return type_error("argsort() missing 1 required positional argument: 'self'");
    }
    args.into_iter().map(implicit_convert).collect()
}

/// `pool.leximin(comparison, other, extra)` / `pool.leximax(...)`: the lexi tuple is computed (and checked)
/// before `other` is converted.
pub fn lexi(
    p: PValue,
    max: bool,
    comparison: PValue,
    other: PValue,
    extra: PValue,
) -> Result<PValue, PErr> {
    let left = method_self(p)?;
    let sort_order = if max {
        Order::Descending
    } else {
        Order::Ascending
    };
    // A non-str matches no `case` either.
    let name = |x: PValue| match x {
        PValue::Value(Value::Str(s)) => s.to_string(),
        _ => String::new(),
    };
    let lexi_tuple =
        compute_lexi_tuple_with_extra(&name(comparison), sort_order.value(), &name(extra))?;
    let right = implicit_convert(other)?;
    die(
        LexiComparisonEvaluator {
            sort_order,
            lexi_tuple,
        },
        &[left, right],
    )
}

/// Whether Python's `except TypeError` catches this error.
fn is_type_error(e: &PErr) -> bool {
    matches!(e, PErr::Raise(Error::Type(_)))
}

/// The comparison evaluator of `<`, `<=`, `>`, `>=`, and the reflected operator (`a < b` falls back on
/// `b > a`).
fn order_comparison(op: &str) -> Option<(Comparison, &'static str)> {
    Some(match op {
        "<" => (Comparison::IsProperSubset, ">"),
        "<=" => (Comparison::IsSubset, ">="),
        ">" => (Comparison::IsProperSuperset, "<"),
        ">=" => (Comparison::IsSuperset, "<="),
        _ => return None,
    })
}

/// `a op b` for `op` in `< <= > >= == !=` with a multiset expression on at least one side (node `mx_compare`).
///
/// `MultisetExpression.__lt__` etc. return `NotImplemented` on a `TypeError` (from converting the other operand or
/// from the evaluation), and Python then tries the reflected operator before raising `TypeError` itself. `==` and
/// `!=` give a `DieWithTruth` whose data is evaluated lazily (an evaluation error is only raised when the data is
/// used) and whose truth value is `self.equals(other)` (hash keys); when the other operand cannot be converted,
/// Python falls back on identity: `False` for `==`, `True` for `!=`.
pub fn compare_operator(op: &str, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let is_die = |x: &PValue| matches!(x, PValue::Die(_) | PValue::DieWithTruth(..));
    let plain_operand = |x: &PValue| {
        matches!(x, PValue::List(_))
            || matches!(x, PValue::Value(v) if !matches!(v, Value::Vector(_) | Value::Symbols(_) | Value::Host(_)))
    };
    match (as_expression(&a), as_expression(&b)) {
        // `Die.__op__` converts the expression into a one-outcome die, whose `Counts` checks that its outcome is
        // sortable with `bool(pool < pool)`: a die has no truth value (or the comparison raises), so TypeError.
        (None, Some(_)) if is_die(&a) => type_error("Items do not appear to be sortable."),
        (Some(left), _) => match (op, order_comparison(op)) {
            (_, Some((comparison, reflected))) => {
                // `self._compare(other, ...)`, NotImplemented on TypeError.
                let attempt = |left: MExpr, other: PValue, comparison: Comparison| {
                    implicit_convert(other)
                        .and_then(|right| die(ComparisonEvaluator(comparison), &[left, right]))
                };
                match attempt(left, b.clone(), comparison) {
                    Err(e) if is_type_error(&e) => {
                        // The reflected operator of an expression on the right (its own NotImplemented ends it).
                        if let Some(right) = as_expression(&b) {
                            let reflected =
                                order_comparison(reflected).expect("reflected comparison").0;
                            match attempt(right, a, reflected) {
                                Err(e) if is_type_error(&e) => {}
                                result => return result,
                            }
                        }
                        type_error(format!("'{op}' not supported between these instances"))
                    }
                    result => result,
                }
            }
            ("==" | "!=", None) => {
                let ne = op == "!=";
                let right = match implicit_convert(b.clone()) {
                    Ok(right) => right,
                    Err(e) if is_type_error(&e) => {
                        // NotImplemented: the other operand's own `__eq__`. A die converts the expression (see
                        // above); for the others it is NotImplemented too, then identity.
                        if is_die(&b) {
                            return type_error("Items do not appear to be sortable.");
                        }
                        return Ok(PValue::Value(Value::Bool(ne)));
                    }
                    Err(e) => return Err(e),
                };
                // `self.equals(other)`: only another expression has a hash key.
                let equals = match as_expression(&b) {
                    Some(other) => same_expression(&left, &other)?,
                    None => false,
                };
                let truth = equals != ne;
                let comparison = if ne {
                    Comparison::IsNotEqualSet
                } else {
                    Comparison::IsEqualSet
                };
                let exprs = [left, right];
                if in_function(&exprs) {
                    // The data callback refuses a raw result.
                    return Ok(PValue::DieWithTruthError(
                        Error::Type("Did not resolve to a die.".into()),
                        truth,
                    ));
                }
                match die(ComparisonEvaluator(comparison), &exprs) {
                    Ok(PValue::Die(d)) => Ok(PValue::DieWithTruth(d, truth)),
                    Ok(_) => Err(PErr::Unimplemented(
                        "a comparison result that is not a die".into(),
                    )),
                    Err(PErr::Raise(e)) => Ok(PValue::DieWithTruthError(e, truth)),
                    Err(e) => Err(e),
                }
            }
            _ => Err(PErr::Unimplemented(format!("multiset comparison {op}"))),
        },
        // The left operand's own operator returns NotImplemented (no plain type knows multisets), so Python calls
        // the expression's reflected one: `a < b` is `b > a`, `a == b` is `b == a`.
        (None, Some(_)) if plain_operand(&a) => match order_comparison(op) {
            Some((_, reflected)) => compare_operator(reflected, b, a),
            None => compare_operator(op, b, a),
        },
        _ => Err(PErr::Unimplemented(
            "a multiset comparison with these operands".into(),
        )),
    }
}

/// `other // pool` (node `mx_rfloordiv`): with an expression on the left, its `__floordiv__` is `count_subset`
/// (the right side is not an int); otherwise the left operand's own `//` returns NotImplemented and
/// `MultisetExpression.__rfloordiv__` is `implicit_convert_to_expression(other).count_subset(self)`, whose
/// conversion errors are not caught.
pub fn rfloordiv(other: PValue, p: PValue) -> Result<PValue, PErr> {
    let right = method_self(p)?;
    let left = match other {
        // `Die.__floordiv__` converts the pool into a one-outcome die (see `compare_operator`).
        PValue::Die(_) | PValue::DieWithTruth(..) => {
            return type_error("Items do not appear to be sortable.")
        }
        PValue::Value(Value::Vector(_) | Value::Symbols(_) | Value::Host(_)) => {
            return Err(PErr::Unimplemented(
                "this left operand of // a multiset".into(),
            ))
        }
        other => implicit_convert(other)?,
    };
    die(
        CountSubsetEvaluator {
            empty_divisor: Value::None,
        },
        &[left, right],
    )
}

// --- S5d: the built-in evaluator classes, over tuple counts too -------------------------------------------------

/// `getattr(icepool.evaluator, evaluator)(*params, **ctor_kwargs).evaluate(*args, **kwargs)` (node
/// `builtin_evaluate`): a built-in evaluator class with the parameters its generator gives, on any multisets (its
/// inputs are mostly MultiDeals, or operators over them: tuple counts). The evaluator is built first, as in
/// Python, then the arguments converted.
pub fn builtin_evaluate(
    evaluator: PValue,
    params: PValue,
    ctor_kwargs: PValue,
    kwargs: PValue,
    args: PValue,
) -> Result<PValue, PErr> {
    let PValue::Value(Value::Str(name)) = evaluator else {
        return raise(Error::Key("builtin_evaluate names an evaluator class".into()));
    };
    let mut params = py_iter(params)?.into_iter();
    let ctor_kwargs = super::map::kwargs(ctor_kwargs)?;
    let kwargs = super::map::kwargs(kwargs)?;
    let unsupported = || {
        Err(PErr::Unimplemented(format!(
            "builtin_evaluate of {name} with these parameters"
        )))
    };
    // The one keyword argument `key` of `kwargs` (constructor or `evaluate`), if no other is given.
    let only = |kwargs: &[(String, PValue)], key: &str| -> Result<Option<PValue>, PErr> {
        match kwargs {
            [] => Ok(None),
            [(k, v)] if k == key => Ok(Some(v.clone())),
            _ => Err(PErr::Unimplemented(format!(
                "builtin_evaluate of {name} with these keyword arguments"
            ))),
        }
    };
    let positional = params.len();
    let exprs = || -> Result<Vec<MExpr>, PErr> {
        py_iter(args.clone())?
            .into_iter()
            .map(implicit_convert)
            .collect()
    };
    let simple = |kwargs: &[(String, PValue)]| ctor_kwargs.is_empty() && kwargs.is_empty() && positional == 0;
    let comparison = match &*name {
        "IsProperSubsetEvaluator" => Some(Comparison::IsProperSubset),
        "IsSubsetEvaluator" => Some(Comparison::IsSubset),
        "IsProperSupersetEvaluator" => Some(Comparison::IsProperSuperset),
        "IsSupersetEvaluator" => Some(Comparison::IsSuperset),
        "IsEqualSetEvaluator" => Some(Comparison::IsEqualSet),
        "IsNotEqualSetEvaluator" => Some(Comparison::IsNotEqualSet),
        "IsDisjointSetEvaluator" => Some(Comparison::IsDisjointSet),
        _ => None,
    };
    if let Some(comparison) = comparison {
        if !simple(&kwargs) {
            return unsupported();
        }
        return die(ComparisonEvaluator(comparison), &exprs()?);
    }
    match &*name {
        "SumEvaluator" if simple(&kwargs) => die(SumEvaluator, &exprs()?),
        "SizeEvaluator" if simple(&kwargs) => die(SizeEvaluator, &exprs()?),
        "ProductOfCountsEvaluator" if simple(&kwargs) => die(ProductOfCountsEvaluator, &exprs()?),
        "HighestOutcomeAndCountEvaluator" if simple(&kwargs) => {
            die(HighestOutcomeAndCountEvaluator, &exprs()?)
        }
        "LargestCountEvaluator" if simple(&kwargs) => die(LargestCountEvaluator, &exprs()?),
        "LargestCountAndOutcomeEvaluator" if simple(&kwargs) => {
            die(LargestCountAndOutcomeEvaluator, &exprs()?)
        }
        "LargestStraightEvaluator" if simple(&kwargs) => die(LargestStraightEvaluator, &exprs()?),
        "AllStraightsEvaluator" if simple(&kwargs) => die(AllStraightsEvaluator, &exprs()?),
        "AllStraightsReduceCountsEvaluator" if simple(&kwargs) => die(
            // The default reducer, `operator.mul`.
            AllStraightsReduceCountsEvaluator {
                reducer: |a: &Value, b: &Value| a.binary(crate::value::BinOp::Mul, b),
            },
            &exprs()?,
        ),
        "EmptyEvaluator" if positional == 0 && ctor_kwargs.is_empty() => {
            // `any` is a keyword argument of `initial_state` / `final_outcome`; only its truth value is read.
            let any = match only(&kwargs, "any")? {
                None => false,
                Some(PValue::Value(Value::Bool(b))) => b,
                Some(_) => return unsupported(),
            };
            die(EmptyEvaluator { any }, &exprs()?)
        }
        "ExpandEvaluator" if positional == 0 && ctor_kwargs.is_empty() => {
            // `reverse = order != Order.Ascending` in `final_outcome`.
            let ascending = match only(&kwargs, "order")? {
                None => true,
                Some(order) => value(order)? == Value::from(1),
            };
            die(ExpandEvaluator { ascending }, &exprs()?)
        }
        "KeepEvaluator" if positional == 0 && ctor_kwargs.is_empty() => {
            let Some(index) = only(&kwargs, "index")? else {
                return unsupported();
            };
            let index = match value(index)? {
                Value::None => None,
                Value::Int(i) => Some(i.into()),
                _ => return unsupported(),
            };
            die(KeepEvaluator { index }, &exprs()?)
        }
        "AllCountsEvaluator" if positional == 0 && kwargs.is_empty() => {
            let filter = match only(&ctor_kwargs, "filter")? {
                None => Value::from(1),
                Some(filter) => value(filter)?,
            };
            die(AllCountsEvaluator { filter }, &exprs()?)
        }
        "CountSubsetEvaluator" if positional == 0 && kwargs.is_empty() => {
            let empty_divisor = match only(&ctor_kwargs, "empty_divisor")? {
                None => Value::None,
                Some(d) => value(d)?,
            };
            die(CountSubsetEvaluator { empty_divisor }, &exprs()?)
        }
        "LargestStraightAndOutcomeEvaluator"
            if positional == 1 && ctor_kwargs.is_empty() && kwargs.is_empty() =>
        {
            let evaluator = straight_and_outcome_evaluator(params.next().expect("one parameter"))?;
            die(evaluator, &exprs()?)
        }
        "LargestCountWithWildEvaluator"
            if positional == 3 && ctor_kwargs.is_empty() && kwargs.is_empty() =>
        {
            let mut select = || make_select_function(params.next().expect("three parameters"));
            let evaluator = LargestCountWithWildEvaluator {
                wild: select()?,
                wild_low: select()?,
                wild_high: select()?,
            };
            die(evaluator, &exprs()?)
        }
        "ArgsortEvaluatorDrop" | "ArgsortEvaluatorLeft" | "ArgsortEvaluatorRight" | "ArgsortGroupedEvaluator"
            if positional == 0 && kwargs.is_empty() =>
        {
            // Keyword-only constructor arguments, defaults `order=Order.Descending, limit=None, limit_pad=None`.
            let grouped = &*name == "ArgsortGroupedEvaluator";
            let (mut order, mut limit, mut limit_pad) = (Value::from(Order::Descending.value()), None, None);
            for (key, v) in ctor_kwargs.iter().cloned() {
                match &*key {
                    "order" => order = value(v)?,
                    "limit" => limit = optional_int(v)?,
                    "limit_pad" if !grouped => {
                        limit_pad = match value(v)? {
                            Value::None => None,
                            v => Some(v),
                        }
                    }
                    _ => return unsupported(),
                }
            }
            if grouped {
                return die(ArgsortGroupedEvaluator { order, limit }, &exprs()?);
            }
            let tie = match &*name {
                "ArgsortEvaluatorDrop" => Tie::Drop,
                "ArgsortEvaluatorLeft" => Tie::Left,
                _ => Tie::Right,
            };
            die(
                ArgsortEvaluator {
                    tie,
                    order,
                    limit,
                    limit_pad,
                },
                &exprs()?,
            )
        }
        "LexiComparisonEvaluator" if simple(&[]) && kwargs.len() == 2 => {
            // `leximin` / `leximax` pass `sort_order` and `lexi_tuple` to `evaluate`.
            let get = |key: &str| kwargs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let (Some(sort_order), Some(lexi_tuple)) = (get("sort_order"), get("lexi_tuple")) else {
                return unsupported();
            };
            let Value::Int(sort_order) = value(sort_order)? else {
                return unsupported();
            };
            let Some(sort_order) = i64::try_from(&sort_order).ok().and_then(Order::from_int) else {
                return unsupported();
            };
            let lexi_tuple = py_iter(lexi_tuple)?
                .into_iter()
                .map(|x| match value(x)? {
                    Value::Int(i) => Ok(i64::try_from(&i).ok()),
                    _ => Ok(None),
                })
                .collect::<Result<Option<Vec<i64>>, PErr>>()?
                .unwrap_or_default();
            let [a, b, c, d, e] = lexi_tuple[..] else {
                return unsupported();
            };
            die(
                LexiComparisonEvaluator {
                    sort_order,
                    lexi_tuple: (a, b, c, d, e),
                },
                &exprs()?,
            )
        }
        _ => unsupported(),
    }
}
