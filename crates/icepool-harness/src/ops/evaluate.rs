//! The built-in multiset evaluations and `evaluate` with a catalogue evaluator
//! (harness/grammar/evaluate.py).

use super::str_arg;
use crate::evaluate;
use crate::map;
use crate::{raise, Args, PValue};
use icepool_core::Error;
use icepool_core::Value;

use super::Dispatch;

/// The evaluate nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "pool_sum"
        | "pool_size"
        | "pool_empty"
        | "pool_any"
        | "pool_product_of_counts"
        | "pool_highest_outcome_and_count"
        | "pool_largest_count"
        | "pool_largest_count_and_outcome"
        | "pool_largest_straight"
        | "pool_all_straights" => args.with(["pool"], |[p]| evaluate::evaluation(op, p)),
        "pool_all_counts" => args.with(["pool", "filter"], |[p, filter]| {
            evaluate::all_counts(p, filter)
        }),
        "pool_expand" => args.with(["pool", "order"], |[p, order]| evaluate::expand(p, order)),
        "pool_largest_straight_and_outcome" => args.with(["pool", "priority"], |[p, priority]| {
            evaluate::largest_straight_and_outcome(p, priority)
        }),
        "straight_and_outcome_evaluate" => args.with(["priority", "arg"], |[priority, arg]| {
            evaluate::straight_and_outcome_evaluate(priority, arg)
        }),
        "pool_count_subset" => args.with(
            ["pool", "divisor", "empty_divisor"],
            |[p, divisor, empty_divisor]| evaluate::count_subset(p, divisor, empty_divisor),
        ),
        "pool_issubset" | "pool_issuperset" | "pool_isdisjoint" => args
            .with(["pool", "other"], |[p, other]| {
                evaluate::compare(op, p, other)
            }),
        "pool_compare" => args.with(["pool", "other", "cmp"], |[p, other, cmp]| {
            let PValue::Value(Value::Str(cmp)) = cmp else {
                return raise(Error::MissingKey("pool_compare names a comparison".into()));
            };
            evaluate::compare(&cmp, p, other)
        }),
        "pool_sum_map" => args.with(["pool", "map"], |[p, map]| evaluate::sum_map(p, map)),
        "pool_largest_count_wild" => args.with(
            ["pool", "wild", "wild_low", "wild_high"],
            |[p, wild, wild_low, wild_high]| {
                evaluate::largest_count_wild(p, wild, wild_low, wild_high)
            },
        ),
        "pool_all_straights_reduce_counts" => args.with(["pool", "reducer"], |[p, reducer]| {
            evaluate::all_straights_reduce_counts(p, reducer)
        }),
        "argsort" => args.with(
            ["args", "order", "limit", "limit_pad", "tie"],
            |[a, order, limit, limit_pad, tie]| evaluate::argsort(a, order, limit, limit_pad, tie),
        ),
        "argsort_grouped" => args.with(["args", "order", "limit"], |[a, order, limit]| {
            evaluate::argsort_grouped(a, order, limit)
        }),
        "pool_leximin" | "pool_leximax" => args.with(
            ["pool", "comparison", "other", "extra"],
            |[p, comparison, other, extra]| {
                evaluate::lexi(p, op == "pool_leximax", comparison, other, extra)
            },
        ),
        "mx_compare" => args.with(["a", "o", "b"], |[a, o, b]| {
            evaluate::compare_operator(&str_arg(&o)?, a, b)
        }),
        "mx_rfloordiv" => args.with(["other", "pool"], |[other, p]| {
            evaluate::rfloordiv(other, p)
        }),
        "keep_evaluate" => args.with(["pool", "index"], |[p, index]| {
            evaluate::keep_evaluate(p, index)
        }),
        "evaluate" | "multi_deal_evaluate" | "tuple_op_evaluate" => args
            .with(["name", "params", "args"], |[name, params, a]| {
                evaluate::catalogue(name, params, a, vec![])
            }),
        "evaluate_kw" => args.with(
            ["name", "params", "args", "kwargs"],
            |[name, params, a, kwargs]| evaluate::catalogue(name, params, a, map::kwargs(kwargs)?),
        ),
        _ => return Err(args),
    })
}
