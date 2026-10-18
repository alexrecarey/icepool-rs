//! Multiset expressions, multiset functions and weightless sources (harness/grammar/multiset.py,
//! multiset_function.py, weightless.py).

use super::{bool_arg, str_arg};
use crate::evaluate;
use crate::expression;
use crate::Args;

use super::Dispatch;

/// The multiset nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "mx_binary" => args.with(["a", "o", "b"], |[a, o, b]| {
            expression::binary_op(&str_arg(&o)?, a, b)
        }),
        "mx_method" => args.with(["name", "args", "keep_negative_counts"], |[name, a, k]| {
            expression::method(&str_arg(&name)?, a, k)
        }),
        "mx_counts" => args.with(["expr", "o", "comparison", "n"], |[e, o, comparison, n]| {
            expression::counts(e, &str_arg(&o)?, comparison, n)
        }),
        "mx_filter" => args.with(["expr", "invert", "target"], |[e, invert, target]| {
            expression::filter_outcomes(e, bool_arg(&invert)?, target)
        }),
        "mx_map_counts" => args.with(["args", "function"], |[a, function]| {
            expression::map_counts(a, function)
        }),
        "mx_sort_pair" => args.with(
            ["expr", "method", "comparison", "other", "order", "extra"],
            |[e, method, comparison, other, order, extra]| {
                expression::sort_pair(e, &str_arg(&method)?, comparison, other, order, extra)
            },
        ),
        "mx_max_pair" => args.with(
            ["expr", "keep", "comparison", "other", "priority"],
            |[e, keep, comparison, other, priority]| {
                expression::max_pair(e, bool_arg(&keep)?, comparison, other, priority)
            },
        ),
        "mx_versus" => args.with(
            ["expr", "all", "comparison", "other"],
            |[e, all, comparison, other]| expression::versus(e, bool_arg(&all)?, comparison, other),
        ),
        "mx_force_order" => args.with(["expr", "order"], |[e, order]| {
            expression::force_order(e, order)
        }),
        "weightless" => args.with(["generator"], |[g]| expression::weightless(g)),
        "mx_str" => args.with(["expr"], |[e]| expression::mx_str(e)),
        "mx_mixture" => args.with(["inners"], |[inners]| expression::mixture(inners)),
        "mx_tuple_index" => args.with(["expr", "index"], |[e, index]| {
            expression::tuple_index(e, index)
        }),
        "mx_tuple_slice" => args.with(
            ["expr", "start", "stop", "step"],
            |[e, start, stop, step]| expression::tuple_slice(e, start, stop, step),
        ),
        "mf_var" => args.with(["i"], |[i]| crate::multiset_function::mf_var(i)),
        "mf_call" => args.with(["f", "args", "star"], |[f, a, star]| {
            crate::multiset_function::mf_call(f, a, star)
        }),
        "mf_call_kw" => args.with(["f", "args", "kwargs"], |[f, a, kwargs]| {
            crate::multiset_function::mf_call_kw(f, a, kwargs)
        }),
        "mf_kwarg" => args.with(["name"], |[name]| crate::multiset_function::mf_kwarg(name)),
        "mx_evaluate" | "mx_tuple_evaluate" | "to_pool_evaluate" | "weightless_evaluate" => args
            .with(["expr", "method"], |[e, method]| {
                evaluate::method_evaluation(&str_arg(&method)?, e)
            }),
        "builtin_evaluate" => args.with(
            ["evaluator", "params", "ctor_kwargs", "kwargs", "args"],
            |[evaluator, params, ctor_kwargs, kwargs, a]| {
                evaluate::builtin_evaluate(evaluator, params, ctor_kwargs, kwargs, a)
            },
        ),
        _ => return Err(args),
    })
}
