//! The grammar nodes (harness/grammar/*.py), each unpacking its Python-style arguments and calling the core.

use std::borrow::Cow;

use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use super::again::{self, AgainOp};
use super::catalogue::Func;
use super::collection;
use super::deck;
use super::die_methods;
use super::evaluate;
use super::expression;
use super::format;
use super::function;
use super::map;
use super::map_tools;
use super::multi_deal;
use super::pool;
use super::population;
use super::statistics;
use super::to_pool;
use super::{raise, Args, PErr, PValue};
use crate::creation_args::DieArg;
use crate::error::Error;
use crate::value::{BinOp, Num, UnOp};
use crate::{Die, Quantity, Value};

pub fn call(op: &str, args: Args) -> Result<PValue, PErr> {
    match op {
        "d" => {
            let [sides] = args.exactly(["sides"])?;
            d(sides)
        }
        "die" | "die_typed" | "die_vectors" | "die_symbols" => {
            let [outcomes] = args.exactly(["outcomes"])?;
            die(outcomes, None)
        }
        "die_times" | "mixture" => {
            let [outcomes, times] = args.exactly(["outcomes", "times"])?;
            die(outcomes, Some(times))
        }
        "weights" => {
            let [items] = args.exactly(["items"])?;
            weights(items)
        }
        "fn" => {
            let [name, params] = args.exactly(["name", "params"])?;
            Ok(PValue::Func(Func::new(name, py_iter(params)?)?.into()))
        }
        "Reroll" => {
            let [] = args.exactly([])?;
            Ok(PValue::Reroll)
        }
        "Again" => {
            let [] = args.exactly([])?;
            // `icepool.Again = AgainExpression(is_additive=True)`.
            Ok(PValue::Again(std::sync::Arc::new(
                crate::again::Again::Again { is_additive: true },
            )))
        }
        "again" => {
            let [outcomes, depth, end] = args.exactly(["outcomes", "depth", "end"])?;
            // `again_depth=None` is the default (depth mode, depth 1).
            let depth = match depth {
                PValue::Value(Value::None) => None,
                d => Some(d),
            };
            die_with_agains(outcomes, None, None, depth, end)
        }
        "Restart" => {
            let [] = args.exactly([])?;
            Ok(PValue::Restart)
        }
        "again_count" => {
            let [outcomes, count, depth, end] =
                args.exactly(["outcomes", "count", "depth", "end"])?;
            // `None` is the default for both.
            let some = |x: PValue| match x {
                PValue::Value(Value::None) => None,
                x => Some(x),
            };
            die_with_agains(outcomes, None, some(count), some(depth), end)
        }
        "explode" => {
            let [die, which, star, depth, end] =
                args.exactly(["die", "which", "star", "depth", "end"])?;
            again::explode(die, which, star, depth, end)
        }
        "matmul" => {
            let [n, die] = args.exactly(["n", "die"])?;
            matmul(n, die)
        }
        "tuple" => {
            let [items] = args.exactly(["items"])?;
            Ok(PValue::tuple(py_iter(items)?))
        }
        "map" => {
            let [f, args, star] = args.exactly(["f", "args", "star"])?;
            map::map(f, args, star, vec![])
        }
        "map_function" => {
            let [f, args_, star, kwargs, decorator] =
                args.exactly(["f", "args", "star", "kwargs", "decorator"])?;
            map::map_function(f, args_, star, kwargs, decorator)
        }
        "map_kw" => {
            let [f, args, star, kwargs] = args.exactly(["f", "args", "star", "kwargs"])?;
            map::map(f, args, star, map::kwargs(kwargs)?)
        }
        "map_repeat" => {
            let [f, args, star, repeat] = args.exactly(["f", "args", "star", "repeat"])?;
            map_tools::map_repeat(f, args, star, repeat)
        }
        "map_and_time" => {
            let [f, initial, args, star, repeat] =
                args.exactly(["f", "initial", "args", "star", "repeat"])?;
            map_tools::map_and_time(f, initial, args, star, repeat)
        }
        "mean_time_to_absorb" => {
            let [f, initial, args, star] = args.exactly(["f", "initial", "args", "star"])?;
            map_tools::mean_time_to_absorb(f, initial, args, star)
        }
        "time_to_sum" => {
            let [die, target, max_time, dnf] =
                args.exactly(["die", "target", "max_time", "dnf"])?;
            map_tools::time_to_sum(die, target, max_time, dnf)
        }
        "reroll" | "filter" => {
            let [die, which, star, depth] = args.exactly(["die", "which", "star", "depth"])?;
            map::reroll(op == "filter", die, which, star, depth)
        }
        "map_to_pool" => {
            let [f, args, star] = args.exactly(["f", "args", "star"])?;
            to_pool::map_to_pool(f, args, star)
        }
        "die_map_to_pool" => {
            let [die, f, args, star] = args.exactly(["die", "f", "args", "star"])?;
            to_pool::die_map_to_pool(die, f, args, star)
        }
        "explode_to_pool" => {
            let [die, rolls, outcomes, star, depth] =
                args.exactly(["die", "rolls", "outcomes", "star", "depth"])?;
            to_pool::explode_to_pool(die, rolls, outcomes, star, depth)
        }
        "reroll_to_pool" => {
            let [die, rolls, outcomes, max_rerolls, star, depth, mode] = args.exactly([
                "die",
                "rolls",
                "outcomes",
                "max_rerolls",
                "star",
                "depth",
                "mode",
            ])?;
            to_pool::reroll_to_pool(die, rolls, outcomes, max_rerolls, star, depth, mode)
        }
        "split" => {
            let [die, which, star] = args.exactly(["die", "which", "star"])?;
            map::split(die, which, star)
        }
        "clip" | "truncate" => {
            let [die, min, max] = args.exactly(["die", "min", "max"])?;
            map::clip_or_truncate(op == "clip", die, min, max)
        }
        "zero" => {
            let [die] = args.exactly(["die"])?;
            die_methods::zero(die)
        }
        "zero_outcome" => {
            let [die] = args.exactly(["die"])?;
            die_methods::zero_outcome(die)
        }
        "sign" => {
            let [die] = args.exactly(["die"])?;
            die_methods::sign(die)
        }
        "die_cmp" => {
            let [die, other] = args.exactly(["die", "other"])?;
            die_methods::cmp(die, other)
        }
        "is_in" => {
            let [die, outcomes] = args.exactly(["die", "outcomes"])?;
            die_methods::is_in(die, outcomes)
        }
        "die_count" => {
            let [die, rolls, outcomes] = args.exactly(["die", "rolls", "outcomes"])?;
            die_methods::count(die, rolls, outcomes)
        }
        "if_else" => {
            let [die, if_true, if_false, count, depth, end] =
                args.exactly(["die", "if_true", "if_false", "count", "depth", "end"])?;
            // `None` is the default for both again arguments.
            let some = |x: PValue| match x {
                PValue::Value(Value::None) => None,
                x => Some(x),
            };
            die_methods::if_else(die, if_true, if_false, some(count), some(depth), end)
        }
        "die_repr" => {
            let [die] = args.exactly(["die"])?;
            die_methods::repr(die)
        }
        "simplify" => {
            let [die] = args.exactly(["die"])?;
            die_methods::simplify(die)
        }
        "die_pop" => {
            let [die, max] = args.exactly(["die", "max"])?;
            die_methods::pop(die, max)
        }
        "from_cumulative" => {
            let [outcomes, cumulative, reverse] =
                args.exactly(["outcomes", "cumulative", "reverse"])?;
            die_methods::from_cumulative(outcomes, cumulative, reverse)
        }
        "Ellipsis" => {
            let [] = args.exactly([])?;
            Ok(PValue::Ellipsis)
        }
        "pool" => {
            let [dice, times] = args.exactly(["dice", "times"])?;
            pool::pool(dice, times)
        }
        "pool_dict" => {
            let [items, times] = args.exactly(["items", "times"])?;
            pool::pool_dict(items, times)
        }
        "d_pool" | "z_pool" => {
            let [die_sizes, mapping] = args.exactly(["die_sizes", "mapping"])?;
            let mapping = matches!(mapping, PValue::Value(Value::Bool(true)));
            pool::standard_pool(op == "z_pool", die_sizes, mapping)
        }
        "die_pool" => {
            let [die, rolls] = args.exactly(["die", "rolls"])?;
            pool::die_pool(die, rolls)
        }
        "pool_keep" | "pool_index" => {
            let [p, index] = args.exactly(["pool", "index"])?;
            pool::pool_keep(p, index)
        }
        "pool_slice" => {
            let [p, start, stop, step] = args.exactly(["pool", "start", "stop", "step"])?;
            pool::pool_slice(p, start, stop, step)
        }
        "pool_highest" | "pool_lowest" => {
            let [p, keep, drop] = args.exactly(["pool", "keep", "drop"])?;
            pool::pool_highest_lowest(op == "pool_highest", p, keep, drop)
        }
        "pool_middle" => {
            let [p, keep, tie] = args.exactly(["pool", "keep", "tie"])?;
            pool::pool_middle(p, keep, tie)
        }
        "die_method" => {
            let [die, name, arg] = args.exactly(["die", "name", "arg"])?;
            die_methods::die_method(die, name, arg)
        }
        "die_middle" => {
            let [die, rolls, keep, tie] = args.exactly(["die", "rolls", "keep", "tie"])?;
            pool::die_middle(die, rolls, keep, tie)
        }
        "die_keep" => {
            let [die, rolls, index] = args.exactly(["die", "rolls", "index"])?;
            pool::die_keep(die, rolls, index)
        }
        "die_keep_slice" => {
            let [die, rolls, start, stop, step] =
                args.exactly(["die", "rolls", "start", "stop", "step"])?;
            pool::die_keep_slice(die, rolls, start, stop, step)
        }
        "middle" => {
            let [a, keep, tie, default] = args.exactly(["args", "keep", "tie", "default"])?;
            pool::middle(a, keep, tie, default)
        }
        "pool_mul" => {
            let [p, n] = args.exactly(["pool", "n"])?;
            pool::pool_mul(p, n)
        }
        "pool_neg" => {
            let [p] = args.exactly(["pool"])?;
            pool::pool_neg(p)
        }
        "pool_add" => {
            let [a, b] = args.exactly(["a", "b"])?;
            pool::pool_add(a, b)
        }
        "mx_binary" => {
            let [a, o, b] = args.exactly(["a", "o", "b"])?;
            expression::binary_op(&str_arg(&o)?, a, b)
        }
        "mx_method" => {
            let [name, a, k] = args.exactly(["name", "args", "keep_negative_counts"])?;
            expression::method(&str_arg(&name)?, a, k)
        }
        "mx_counts" => {
            let [e, o, comparison, n] = args.exactly(["expr", "o", "comparison", "n"])?;
            expression::counts(e, &str_arg(&o)?, comparison, n)
        }
        "mx_filter" => {
            let [e, invert, target] = args.exactly(["expr", "invert", "target"])?;
            expression::filter_outcomes(e, bool_arg(&invert)?, target)
        }
        "mx_map_counts" => {
            let [a, function] = args.exactly(["args", "function"])?;
            expression::map_counts(a, function)
        }
        "mx_sort_pair" => {
            let [e, method, comparison, other, order, extra] =
                args.exactly(["expr", "method", "comparison", "other", "order", "extra"])?;
            expression::sort_pair(e, &str_arg(&method)?, comparison, other, order, extra)
        }
        "mx_max_pair" => {
            let [e, keep, comparison, other, priority] =
                args.exactly(["expr", "keep", "comparison", "other", "priority"])?;
            expression::max_pair(e, bool_arg(&keep)?, comparison, other, priority)
        }
        "mx_versus" => {
            let [e, all, comparison, other] =
                args.exactly(["expr", "all", "comparison", "other"])?;
            expression::versus(e, bool_arg(&all)?, comparison, other)
        }
        "mx_force_order" => {
            let [e, order] = args.exactly(["expr", "order"])?;
            expression::force_order(e, order)
        }
        "weightless" => {
            let [g] = args.exactly(["generator"])?;
            expression::weightless(g)
        }
        "mx_mixture" => {
            let [inners] = args.exactly(["inners"])?;
            expression::mixture(inners)
        }
        "mx_tuple_index" => {
            let [e, index] = args.exactly(["expr", "index"])?;
            expression::tuple_index(e, index)
        }
        "mx_tuple_slice" => {
            let [e, start, stop, step] = args.exactly(["expr", "start", "stop", "step"])?;
            expression::tuple_slice(e, start, stop, step)
        }
        "mf_var" => {
            let [i] = args.exactly(["i"])?;
            super::multiset_function::mf_var(i)
        }
        "mf_call" => {
            let [f, a, star] = args.exactly(["f", "args", "star"])?;
            super::multiset_function::mf_call(f, a, star)
        }
        "mf_call_kw" => {
            let [f, a, kwargs] = args.exactly(["f", "args", "kwargs"])?;
            super::multiset_function::mf_call_kw(f, a, kwargs)
        }
        "mf_kwarg" => {
            let [name] = args.exactly(["name"])?;
            super::multiset_function::mf_kwarg(name)
        }
        "mx_evaluate" | "mx_tuple_evaluate" | "to_pool_evaluate" | "weightless_evaluate" => {
            let [e, method] = args.exactly(["expr", "method"])?;
            evaluate::method_evaluation(&str_arg(&method)?, e)
        }
        "deck" => {
            let [outcomes, times] = args.exactly(["outcomes", "times"])?;
            deck::deck(outcomes, times)
        }
        "deck_op" => {
            let [op, a, b] = args.exactly(["operator", "a", "b"])?;
            deck::deck_op(op, a, b)
        }
        "deck_mul" => {
            let [d, op, n] = args.exactly(["deck", "operator", "n"])?;
            deck::deck_mul(d, op, n)
        }
        "deck_map" => {
            let [d, repl, star] = args.exactly(["deck", "repl", "star"])?;
            deck::deck_map(d, repl, star)
        }
        "deck_sequence" => {
            let [d, deals] = args.exactly(["deck", "deals"])?;
            deck::deck_sequence(d, deals)
        }
        "deck_method" => {
            let [d, name, arg] = args.exactly(["deck", "name", "arg"])?;
            deck::deck_method(d, name, arg)
        }
        "deal_str" => {
            let [d] = args.exactly(["deal"])?;
            deck::deal_str(d)
        }
        "deck_info" => {
            let [d] = args.exactly(["deck"])?;
            deck::deck_info(d)
        }
        "deck_pop" => {
            let [d, max] = args.exactly(["deck", "max"])?;
            deck::deck_pop(d, max)
        }
        "deck_stat" => {
            let [d, stat] = args.exactly(["deck", "stat"])?;
            deck::deck_stat(d, stat)
        }
        "deck_pad_to_denominator" => {
            let [d, denominator, outcome] = args.exactly(["deck", "denominator", "outcome"])?;
            deck::deck_pad_to_denominator(d, denominator, outcome)
        }
        "deal" => {
            let [d, hand_size] = args.exactly(["deck", "hand_size"])?;
            deck::deal(d, hand_size)
        }
        "multi_deal" => {
            let [d, hand_sizes] = args.exactly(["deck", "hand_sizes"])?;
            deck::deal(d, hand_sizes)
        }
        "deal_groups" => {
            let [d, groups] = args.exactly(["deck", "hand_groups"])?;
            multi_deal::deal_groups(d, groups)
        }
        "multi_deal_info" => {
            let [d] = args.exactly(["multi_deal"])?;
            multi_deal::multi_deal_info(d)
        }
        "multi_deal_pop" => {
            let [d, order, outcome] = args.exactly(["multi_deal", "order", "outcome"])?;
            multi_deal::multi_deal_pop(d, order, outcome)
        }
        "deal_info" => {
            let [d] = args.exactly(["deal"])?;
            deck::deal_info(d)
        }
        "deal_pop" => {
            let [d, order, outcome] = args.exactly(["deal", "order", "outcome"])?;
            deck::deal_pop(d, order, outcome)
        }
        "pool_info" => {
            let [p] = args.exactly(["pool"])?;
            pool::pool_info(p)
        }
        "pool_pop" => {
            let [p, order, outcome] = args.exactly(["pool", "order", "outcome"])?;
            pool::pool_pop(p, order, outcome)
        }
        "die_highest" | "die_lowest" => {
            let [die, rolls, keep, drop] = args.exactly(["die", "rolls", "keep", "drop"])?;
            pool::die_highest_lowest(op == "die_highest", die, rolls, keep, drop)
        }
        "highest" | "lowest" => {
            let [a, keep, drop, default] = args.exactly(["args", "keep", "drop", "default"])?;
            pool::highest_lowest(op == "highest", a, keep, drop, default)
        }
        "pool_sum"
        | "pool_size"
        | "pool_empty"
        | "pool_any"
        | "pool_product_of_counts"
        | "pool_highest_outcome_and_count"
        | "pool_largest_count"
        | "pool_largest_count_and_outcome"
        | "pool_largest_straight"
        | "pool_all_straights" => {
            let [p] = args.exactly(["pool"])?;
            evaluate::evaluation(op, p)
        }
        "pool_all_counts" => {
            let [p, filter] = args.exactly(["pool", "filter"])?;
            evaluate::all_counts(p, filter)
        }
        "pool_expand" => {
            let [p, order] = args.exactly(["pool", "order"])?;
            evaluate::expand(p, order)
        }
        "pool_largest_straight_and_outcome" => {
            let [p, priority] = args.exactly(["pool", "priority"])?;
            evaluate::largest_straight_and_outcome(p, priority)
        }
        "pool_count_subset" => {
            let [p, divisor, empty_divisor] = args.exactly(["pool", "divisor", "empty_divisor"])?;
            evaluate::count_subset(p, divisor, empty_divisor)
        }
        "pool_issubset" | "pool_issuperset" | "pool_isdisjoint" => {
            let [p, other] = args.exactly(["pool", "other"])?;
            evaluate::compare(op, p, other)
        }
        "pool_compare" => {
            let [p, other, cmp] = args.exactly(["pool", "other", "cmp"])?;
            let PValue::Value(Value::Str(cmp)) = cmp else {
                return raise(Error::Key("pool_compare names a comparison".into()));
            };
            evaluate::compare(&cmp, p, other)
        }
        "pool_sum_map" => {
            let [p, map] = args.exactly(["pool", "map"])?;
            evaluate::sum_map(p, map)
        }
        "pool_largest_count_wild" => {
            let [p, wild, wild_low, wild_high] =
                args.exactly(["pool", "wild", "wild_low", "wild_high"])?;
            evaluate::largest_count_wild(p, wild, wild_low, wild_high)
        }
        "pool_all_straights_reduce_counts" => {
            let [p, reducer] = args.exactly(["pool", "reducer"])?;
            evaluate::all_straights_reduce_counts(p, reducer)
        }
        "argsort" => {
            let [a, order, limit, limit_pad, tie] =
                args.exactly(["args", "order", "limit", "limit_pad", "tie"])?;
            evaluate::argsort(a, order, limit, limit_pad, tie)
        }
        "argsort_grouped" => {
            let [a, order, limit] = args.exactly(["args", "order", "limit"])?;
            evaluate::argsort_grouped(a, order, limit)
        }
        "pool_leximin" | "pool_leximax" => {
            let [p, comparison, other, extra] =
                args.exactly(["pool", "comparison", "other", "extra"])?;
            evaluate::lexi(p, op == "pool_leximax", comparison, other, extra)
        }
        "mx_compare" => {
            let [a, o, b] = args.exactly(["a", "o", "b"])?;
            evaluate::compare_operator(&str_arg(&o)?, a, b)
        }
        "mx_rfloordiv" => {
            let [other, p] = args.exactly(["other", "pool"])?;
            evaluate::rfloordiv(other, p)
        }
        "keep_evaluate" => {
            let [p, index] = args.exactly(["pool", "index"])?;
            evaluate::keep_evaluate(p, index)
        }
        "evaluate" | "multi_deal_evaluate" | "tuple_op_evaluate" => {
            let [name, params, a] = args.exactly(["name", "params", "args"])?;
            evaluate::catalogue(name, params, a, vec![])
        }
        "evaluate_kw" => {
            let [name, params, a, kwargs] = args.exactly(["name", "params", "args", "kwargs"])?;
            evaluate::catalogue(name, params, a, map::kwargs(kwargs)?)
        }
        "quantities" => {
            let [die, comparison] = args.exactly(["die", "comparison"])?;
            statistics::quantities(die, comparison)
        }
        "probabilities" => {
            let [die, comparison, percent] = args.exactly(["die", "comparison", "percent"])?;
            statistics::probabilities(die, comparison, percent)
        }
        "quantity" => {
            let [die, a] = args.exactly(["die", "args"])?;
            statistics::quantity(die, a)
        }
        "probability" => {
            let [die, a, percent] = args.exactly(["die", "args", "percent"])?;
            statistics::probability(die, a, percent)
        }
        "nearest" => {
            let [die, comparison, outcome] = args.exactly(["die", "comparison", "outcome"])?;
            statistics::nearest(die, comparison, outcome)
        }
        "die_stat" => {
            let [die, stat] = args.exactly(["die", "stat"])?;
            statistics::die_stat(die, stat)
        }
        "standardized_moment" => {
            let [die, k] = args.exactly(["die", "k"])?;
            statistics::standardized_moment(die, k)
        }
        "entropy" => {
            let [die, base] = args.exactly(["die", "base"])?;
            statistics::entropy(die, base)
        }
        "die_quantile" => {
            let [die, method, a] = args.exactly(["die", "method", "args"])?;
            statistics::quantile(die, method, a)
        }
        "cdf_distance" => {
            let [a, b, stat] = args.exactly(["a", "b", "stat"])?;
            statistics::cdf_distance(a, b, stat)
        }
        "marginal" => {
            let [die, index] = args.exactly(["die", "index"])?;
            statistics::marginal(die, index)
        }
        "marginal_slice" => {
            let [die, start, stop, step] = args.exactly(["die", "start", "stop", "step"])?;
            statistics::marginal_slice(die, start, stop, step)
        }
        "marginals" => {
            let [die] = args.exactly(["die"])?;
            statistics::marginals(die)
        }
        "group_by" => {
            let [die, key, star] = args.exactly(["die", "key", "star"])?;
            statistics::group_by(die, key, star)
        }
        "group_by_index" => {
            let [die, index] = args.exactly(["die", "index"])?;
            statistics::group_by_index(die, index)
        }
        "joint_stat" => {
            let [die, stat, i, j] = args.exactly(["die", "stat", "i", "j"])?;
            statistics::joint_stat(die, stat, i, j)
        }
        "quantities_op" => {
            let [die, method, x] = args.exactly(["die", "method", "x"])?;
            statistics::quantities_op(die, method, x)
        }
        "pad_to_denominator" => {
            let [die, denominator, outcome] = args.exactly(["die", "denominator", "outcome"])?;
            statistics::pad_to_denominator(die, denominator, outcome)
        }
        "multiply_to_denominator" => {
            let [die, denominator] = args.exactly(["die", "denominator"])?;
            statistics::multiply_to_denominator(die, denominator)
        }
        "quantity_where" => {
            let [die, which, star] = args.exactly(["die", "which", "star"])?;
            statistics::quantity_where(die, which, star)
        }
        "probability_where" => {
            let [die, which, star, percent] = args.exactly(["die", "which", "star", "percent"])?;
            statistics::probability_where(die, which, star, percent)
        }
        "round" => {
            let [a, ndigits] = args.exactly(["a", "ndigits"])?;
            round(a, ndigits)
        }
        "format" => {
            let [die, spec] = args.exactly(["die", "spec"])?;
            format::format(die, spec)
        }
        "format_probability_inverse" => {
            let [probability, a] = args.exactly(["probability", "args"])?;
            format::format_probability_inverse(probability, a)
        }
        "common_outcome_length" => {
            let [die] = args.exactly(["die"])?;
            format::common_outcome_length(die)
        }
        "sample" => {
            let [die, seed, n] = args.exactly(["die", "seed", "n"])?;
            format::sample(die, seed, n)
        }
        "eq" | "ne" => {
            let [a, b] = args.exactly(["a", "b"])?;
            format::eq(a, b, op == "ne")
        }
        "truth" => {
            let [x] = args.exactly(["x"])?;
            format::truth(x)
        }
        "equals" => {
            let [a, b, simplify] = args.exactly(["a", "b", "simplify"])?;
            format::equals(a, b, simplify)
        }
        "hash_key_compare" => {
            let [a, b, cmp] = args.exactly(["a", "b", "cmp"])?;
            format::hash_key_compare(a, b, cmp)
        }
        "z" => {
            let [sides] = args.exactly(["sides"])?;
            function::z(sides)
        }
        "dz_attr" => {
            let [name] = args.exactly(["name"])?;
            function::dz_attr(name)
        }
        "coin" => {
            let [n, d, m] = args.exactly(["n", "d", "max_denominator"])?;
            function::coin(n, d, m)
        }
        "stochastic_round" => {
            let [x, m] = args.exactly(["x", "max_denominator"])?;
            function::stochastic_round(x, m)
        }
        "one_hot" => {
            let [sides] = args.exactly(["sides"])?;
            function::one_hot(sides)
        }
        "pointwise" => {
            let [a, max, spread] = args.exactly(["args", "max", "spread"])?;
            function::pointwise(a, max, spread)
        }
        "extreme_outcome" => {
            let [a, max, spread] = args.exactly(["args", "max", "spread"])?;
            function::extreme_outcome(a, max, spread)
        }
        "harmonize_denominators" => {
            let [dice, weights] = args.exactly(["dice", "weights"])?;
            function::harmonize_denominators(dice, weights)
        }
        "consecutive" => {
            let [a] = args.exactly(["args"])?;
            function::consecutive(a)
        }
        "sorted_union" => {
            let [a] = args.exactly(["args"])?;
            function::sorted_union(a)
        }
        "reduce" | "accumulate" => {
            let [f, dice, initial] = args.exactly(["f", "dice", "initial"])?;
            if op == "reduce" {
                function::reduce(f, dice, initial)
            } else {
                function::accumulate(f, dice, initial)
            }
        }
        "from_rv" => {
            let [rv, params, outcomes, den] =
                args.exactly(["rv", "params", "outcomes", "denominator"])?;
            function::from_rv(rv, params, outcomes, den)
        }
        "wallenius_deal" => {
            let [data, mapping, hand_size, method] =
                args.exactly(["data", "mapping", "hand_size", "method"])?;
            function::wallenius_deal(data, mapping, hand_size, method)
        }
        "cartesian_product" => {
            let [a, vector] = args.exactly(["args", "vector"])?;
            match vector {
                PValue::Value(Value::Bool(vector)) => collection::cartesian_product(a, !vector),
                _ => Err(PErr::Unimplemented("cartesian_product outcome type".into())),
            }
        }
        "iter_cartesian_product" => {
            let [a] = args.exactly(["args"])?;
            collection::iter_cartesian_product(a)
        }
        "pop_append" => {
            let [p, outcome, quantity] = args.exactly(["population", "outcome", "quantity"])?;
            population::append(p, outcome, quantity)
        }
        "pop_remove" => {
            let [p, outcome, quantity] = args.exactly(["population", "outcome", "quantity"])?;
            population::remove(p, outcome, quantity)
        }
        "to_one_hot" => {
            let [p, outcomes] = args.exactly(["population", "outcomes"])?;
            population::to_one_hot(p, outcomes)
        }
        "deck_zero" => {
            let [d] = args.exactly(["deck"])?;
            population::deck_zero(d)
        }
        "deck_split" => {
            let [d, which, star] = args.exactly(["deck", "which", "star"])?;
            population::deck_split(d, which, star)
        }
        "mean_time_to_sum" => {
            let [die, targets] = args.exactly(["die", "targets"])?;
            population::mean_time_to_sum(die, targets)
        }
        "vectorize" | "tupleize" => {
            let [a] = args.exactly(["args"])?;
            collection::cartesian_product(a, op == "tupleize")
        }
        "vector" => {
            let [items] = args.exactly(["items"])?;
            collection::vector(items)
        }
        "symbols" => {
            let [x] = args.exactly(["x"])?;
            collection::symbols(x)
        }
        "coll_binary" => {
            let [a, b, name] = args.exactly(["a", "b", "name"])?;
            collection::binary(a, b, name)
        }
        "coll_unary" => {
            let [a, name, ndigits] = args.exactly(["a", "name", "ndigits"])?;
            collection::unary(a, name, ndigits)
        }
        "coll_truth" => {
            let [x] = args.exactly(["x"])?;
            format::truth(x)
        }
        "vector_method" => {
            let [v, name, arg] = args.exactly(["v", "name", "arg"])?;
            collection::vector_method(v, name, arg)
        }
        "symbols_method" => {
            let [s, name, arg] = args.exactly(["s", "name", "arg"])?;
            collection::symbols_method(s, name, arg)
        }
        _ => {
            if let Some(bin) = binary_op(op) {
                let [a, b] = args.exactly(["a", "b"])?;
                binary(bin, a, b)
            } else if let Some(un) = unary_op(op) {
                let [a] = args.exactly(["a"])?;
                unary(un, a)
            } else {
                Err(PErr::Unimplemented(op.into()))
            }
        }
    }
}

/// The binary operator nodes (harness/grammar/basic.py, operators.py), named after Python's `operator` module.
pub(super) fn binary_op(op: &str) -> Option<BinOp> {
    use BinOp::*;
    Some(match op {
        "add" => Add,
        "sub" => Sub,
        "mul" => Mul,
        "truediv" => TrueDiv,
        "floordiv" => FloorDiv,
        "mod" => Mod,
        "pow" => Pow,
        "lshift" => LShift,
        "rshift" => RShift,
        "and" => And,
        "or" => Or,
        "xor" => Xor,
        "lt" => Lt,
        "le" => Le,
        "gt" => Gt,
        "ge" => Ge,
        _ => return None,
    })
}

pub(super) fn unary_op(op: &str) -> Option<UnOp> {
    use UnOp::*;
    Some(match op {
        "neg" => Neg,
        "pos" => Pos,
        "invert" => Invert,
        "abs" => Abs,
        "trunc" => Trunc,
        "floor" => Floor,
        "ceil" => Ceil,
        _ => return None,
    })
}

/// `implicit_convert_to_die`: a die stays as it is, anything else becomes `Die([x])` (which raises for `None`, and
/// for a list, which is unhashable; `Reroll` gives the empty die).
pub(super) fn to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d),
        x => Ok(Die::new(vec![die_arg(x)?], vec![Quantity::ONE])?),
    }
}

/// `a <op> b` in Python. With a die on the left, `Die.__op__` converts the right operand. With a die only on the
/// right, the left operand's own `__op__` returns `NotImplemented` (no outcome type knows about dice), so Python
/// calls the die's reflected method: `__radd__` and friends convert the left operand and keep the order
/// (`3 - die` is `Die([3]) - die`), while a comparison reflects (`3 < die` is `die > 3`).
pub(super) fn binary(op: BinOp, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let apply = |x: &Die<Value>, op: BinOp, y: &Die<Value>| {
        x.binary_operator(y, |p, q| p.binary(op, q))
            .map(PValue::Die)
            .map_err(PErr::from)
    };
    match (a, b) {
        // `AgainExpression` defines every operator (and dice return NotImplemented for it), so an expression on
        // either side builds a bigger expression. A comparison with the expression on the right reflects.
        (a @ PValue::Again(_), b) => again::apply(AgainOp::Binary(op), vec![a, b]),
        // Except that `str % x` is printf-style formatting, which runs before the expression's `__rmod__`. The
        // expression is neither a tuple nor a mapping, so it is the one argument: without a conversion specifier
        // it is left over. (With one, `%s` would format its repr, which holds an address.)
        (PValue::Value(Value::Str(s)), PValue::Again(_)) if op == BinOp::Mod => {
            if s.contains('%') {
                Err(PErr::Unimplemented(
                    "str % Again with conversion specifiers".into(),
                ))
            } else {
                type_error("not all arguments converted during string formatting")
            }
        }
        (a, b @ PValue::Again(_)) => match op.reflected_comparison() {
            Some(reflected) => again::apply(AgainOp::Binary(reflected), vec![b, a]),
            None => again::apply(AgainOp::Binary(op), vec![a, b]),
        },
        (PValue::Die(a), b) => apply(&a, op, &to_die(b)?),
        // The exception: `str % die` is printf-style formatting with the die as the argument, and a die is a
        // Mapping, which formatting accepts even when the format string never uses it.
        (PValue::Value(Value::Str(s)), PValue::Die(_)) if op == BinOp::Mod => {
            Ok(PValue::Value(crate::value::ops::str_format_mapping(&s)?))
        }
        // `Symbols.__floordiv__` takes any iterable as `count_subset`, and a die is one (a mapping: its quantities
        // are the counts).
        (PValue::Value(Value::Symbols(s)), PValue::Die(d) | PValue::DieWithTruth(d, _))
            if op == BinOp::FloorDiv =>
        {
            let items = d
                .items()
                .iter()
                .map(|(o, q)| (o.clone(), Value::Int(q.clone().into())))
                .collect();
            Ok(PValue::Value(s.count_subset_items(items, None)?))
        }
        (a, PValue::Die(b)) => match op.reflected_comparison() {
            Some(reflected) => apply(&b, reflected, &to_die(a)?),
            None => apply(&to_die(a)?, op, &b),
        },
        (PValue::Value(a), PValue::Value(b)) => Ok(PValue::Value(a.binary(op, &b)?)),
        _ => Err(PErr::Unimplemented(
            "operators on lists, Reroll, tuples of dice, functions".into(),
        )),
    }
}

/// `-a`, `+a`, `~a`, `abs(a)`, `math.trunc(a)`, `math.floor(a)`, `math.ceil(a)`: `Die.unary_operator` on a die.
pub(super) fn unary(op: UnOp, a: PValue) -> Result<PValue, PErr> {
    match a {
        // `AgainExpression` has `-`, `+`, `~` and `abs`, but no `__trunc__`, `__floor__`, `__ceil__`.
        a @ PValue::Again(_) if matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Invert | UnOp::Abs) => {
            again::apply(AgainOp::Unary(op), vec![a])
        }
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.unary(op))?)),
        PValue::Value(v) => Ok(PValue::Value(v.unary(op)?)),
        _ => type_error("bad operand type for a unary operator"),
    }
}

/// `Die._sum_all(k)` recurses once per roll down to the largest sum already in the die's `_sum_cache`, so a big
/// enough step raises RecursionError (Python's limit is 1000 frames; at top level `Die([n]) @ d1` fails from about
/// n = 985). Recursing this deep always works, whatever the caller's stack...
const SAFE_RECURSION: u64 = 800;
/// ...and this deep never does.
const SURE_RECURSION_ERROR: u64 = 1000;

/// Whether Python's `_sum_all` would hit the recursion limit computing these counts in order, given the cache it
/// fills as it goes. A step between the two limits depends on the caller's stack, which the port cannot know, so
/// it is unimplemented. (The port assumes the right die's `_sum_cache` starts empty, but it lives on the die
/// object, and `icepool.d` caches its dice, so a Python process that summed `d(6)` before has it warm. Only a
/// recursion past the limit can tell the difference.)
fn check_sum_all_recursion(counts: &[(i64, Quantity)]) -> Result<(), PErr> {
    // The largest positive roll count cached so far (1 is a base case, cached for free).
    let mut cached = 1;
    for (count, _) in counts {
        let k = count.unsigned_abs();
        if k > cached {
            let depth = k - cached + u64::from(*count < 0);
            if depth >= SURE_RECURSION_ERROR {
                return raise(Error::Value(
                    "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
                ));
            }
            if depth > SAFE_RECURSION {
                return Err(PErr::Unimplemented(
                    "n @ die close to Python's recursion limit".into(),
                ));
            }
            cached = k;
        }
    }
    Ok(())
}

/// `n @ die`: roll `n`, then roll `die` that many times and sum the outcomes. An int `n` has no `@`, so Python
/// calls `Die.__rmatmul__`, which converts it with `implicit_convert_to_die`: `Die([n]) @ die`. With an `Again`
/// expression on either side (dice return NotImplemented for it) the result is a bigger expression.
pub(super) fn matmul(n: PValue, die: PValue) -> Result<PValue, PErr> {
    let (n, die) = match (n, die) {
        (n @ PValue::Again(_), die) | (n, die @ PValue::Again(_)) => {
            return again::apply(AgainOp::MatMul, vec![n, die])
        }
        (PValue::Die(n), die) => (n, to_die(die)?),
        (n, PValue::Die(die)) => (to_die(n)?, die),
        _ => return type_error("unsupported operand type(s) for @"),
    };
    matmul_dice(&n, &die).map(PValue::Die)
}

/// `n.__matmul__(die)` once both operands are dice (`Die.__matmul__` after `implicit_convert_to_die`). Public for
/// the Python facade, whose `@` is this too: the counts' Python types (float counts, Fractions) matter as they do
/// here.
pub fn matmul_dice(n: &Die<Value>, die: &Die<Value>) -> Result<Die<Value>, PErr> {
    // `max(abs(self.min_outcome()), abs(self.max_outcome()))`, then `_sum_all(die_count)` for each outcome.
    let mut counts = Vec::with_capacity(n.len());
    let mut kinds = Vec::with_capacity(n.len());
    for (outcome, quantity) in n.items() {
        let (count, kind) = die_count(outcome)?;
        counts.push((count, quantity.clone()));
        kinds.push(kind);
    }
    check_sum_all_recursion(&counts)?;
    if kinds.iter().all(|k| *k == CountKind::Int) {
        return Ok(die.matmul(&counts)?);
    }
    // Some count is a float (with an integral value: others raised above). `_sum_all` treats it as the equal
    // int, but the factor `den**(max_abs_die_count - abs(die_count))` is then a float, and so is every quantity it
    // scales. (A Fraction exponent keeps the factor an int: `int ** Fraction` with a non-negative integral
    // exponent is `int ** int`.) `Die(data)` drops a non-int quantity that is zero and raises for any
    // other, so the result is the int terms alone, unless a non-int term adds anything non-zero to an outcome.
    // `max` returns the first of two equal values: abs(min_outcome) unless abs(max_outcome) is greater.
    let (first, last) = (counts[0].0, counts[counts.len() - 1].0);
    let max_float = if last.unsigned_abs() > first.unsigned_abs() {
        kinds[kinds.len() - 1]
    } else {
        kinds[0]
    } == CountKind::Float;
    let max_abs = first.unsigned_abs().max(last.unsigned_abs());
    let denominator = die.denominator();
    let mut data: Vec<(Value, Quantity, bool)> = Vec::new();
    let mut index: FxHashMap<Value, usize> = FxHashMap::default();
    for ((term, kind), (count, _)) in die
        .matmul_terms(&counts)?
        .into_iter()
        .zip(&kinds)
        .zip(&counts)
    {
        let float_factor = max_float || *kind == CountKind::Float;
        if float_factor {
            // `int ** float` converts the int first, and overflows rather than giving inf.
            let exponent = (max_abs - count.unsigned_abs()) as i32;
            let base = crate::value::int_to_f64(&IBig::from(denominator.clone()))?;
            if !base.powi(exponent).is_finite() {
                return raise(Error::Overflow("int ** float: result too large".into()));
            }
        }
        for (outcome, quantity) in term {
            match index.get(&outcome) {
                Some(&i) => {
                    data[i].1 += quantity;
                    data[i].2 |= float_factor;
                }
                None => {
                    index.insert(outcome.clone(), data.len());
                    data.push((outcome, quantity, float_factor));
                }
            }
        }
    }
    let mut items = Vec::with_capacity(data.len());
    for (outcome, quantity, float) in data {
        if float && !quantity.is_zero() {
            return type_error("'float' object cannot be interpreted as an integer");
        }
        items.push((outcome, quantity));
    }
    Ok(Die::from_dict(items)?)
}

/// Whether a count in `n @ die` is a float in Python, which makes its factor a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountKind {
    Int,
    Float,
}

/// One outcome of the left die of `@` as a count of rolls.
fn die_count(outcome: &Value) -> Result<(i64, CountKind), PErr> {
    let (int, kind) = match outcome {
        Value::Int(i) => (i.clone(), CountKind::Int),
        Value::Bool(b) => (IBig::from(*b as u8), CountKind::Int),
        // `_sum_all` subtracts 1 until it reaches 0 or 1, negating on the way below 0, so a count that is not
        // integral (or not finite) recurses until Python raises RecursionError.
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 => (
            IBig::from_str_radix(&format!("{f:.0}"), 10).expect("an integral float"),
            CountKind::Float,
        ),
        Value::Fraction(r) if r.denominator().is_one() => (r.numerator().clone(), CountKind::Int),
        Value::Float(_) | Value::Fraction(_) => {
            return raise(Error::Value(
                "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
            ))
        }
        // Only the Python facade has these (an `IntEnum` count, a numpy int): Python's answer depends on the type.
        Value::Host(_) => {
            return Err(PErr::Unimplemented(
                "a count of a host type in n @ die".into(),
            ))
        }
        v => return type_error(format!("bad operand type for abs(): '{}'", v.type_name())),
    };
    // Beyond i64, `_sum_all` recurses past Python's limit.
    let count = i64::try_from(int).map_err(|_| {
        PErr::Raise(Error::Value(
            "RecursionError: maximum recursion depth exceeded in _sum_all".into(),
        ))
    })?;
    Ok((count, kind))
}

/// `round(a, ndigits)`. For a die, `Die.round` rounds each outcome, so a bad `ndigits` only raises if there is an
/// outcome to round.
fn round(a: PValue, ndigits: PValue) -> Result<PValue, PErr> {
    // A die or a list as ndigits is not an int, so rounding any outcome with it raises; but `Die.round` on an
    // empty die rounds nothing and returns it. A str stands in: `Value::round` raises for any non-int ndigits.
    let nd = match &ndigits {
        PValue::Value(v) => v.clone(),
        _ => Value::str(""),
    };
    match a {
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.round(&nd))?)),
        PValue::Value(v) => Ok(PValue::Value(v.round(&nd)?)),
        _ => type_error("type doesn't define __round__ method"),
    }
}

/// A node's `str` argument naming an operation (`{"str": ...}` in the program).
fn str_arg(x: &PValue) -> Result<String, PErr> {
    match x {
        PValue::Value(Value::Str(s)) => Ok(s.to_string()),
        _ => raise(Error::Key("expected a str naming an operation".into())),
    }
}

/// A node's flag argument (`{"bool": ...}`).
fn bool_arg(x: &PValue) -> Result<bool, PErr> {
    match x {
        PValue::Value(Value::Bool(b)) => Ok(*b),
        _ => raise(Error::Type("expected a bool flag".into())),
    }
}

pub(super) fn type_error<T>(msg: impl Into<String>) -> Result<T, PErr> {
    raise(Error::Type(msg.into()))
}

fn negative_times<T>() -> Result<T, PErr> {
    raise(Error::Value("times cannot have negative values.".into()))
}

fn zero() -> Num<'static> {
    Num::Int(Cow::Owned(IBig::ZERO))
}

/// `icepool.d(sides)`: `sides` must be an `int` (a `bool` is one).
pub(super) fn d(sides: PValue) -> Result<PValue, PErr> {
    let sides = match sides {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        _ => return type_error("sides must be an int."),
    };
    // Beyond i64 Python would run out of memory long before finishing.
    let sides =
        i64::try_from(sides).map_err(|_| PErr::Raise(Error::Overflow("sides too large".into())))?;
    Ok(PValue::Die(crate::d(sides)?))
}

/// Python's `iter(x)` over a program value: a list, a tuple's elements, a str's characters, a die's outcomes.
pub(super) fn py_iter(x: PValue) -> Result<Vec<PValue>, PErr> {
    Ok(match x {
        PValue::DieWithTruthError(e, _) => return Err(e.into()),
        PValue::List(xs) | PValue::Tuple(xs) => xs,
        PValue::Value(Value::Tuple(t)) => t.iter().cloned().map(PValue::Value).collect(),
        PValue::Value(Value::Str(s)) => s
            .chars()
            .map(|c| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => {
            d.outcomes().cloned().map(PValue::Value).collect()
        }
        PValue::Value(Value::Vector(v)) => v.data()?.iter().cloned().map(PValue::Value).collect(),
        // A mapping: its keys, in the dict's order.
        PValue::Value(Value::Symbols(s)) => s
            .items()
            .iter()
            .map(|(c, _)| PValue::Value(Value::str(c.encode_utf8(&mut [0; 4]))))
            .collect(),
        PValue::Value(v) => {
            return type_error(format!("'{}' object is not iterable", v.type_name()))
        }
        PValue::Reroll | PValue::Restart => {
            return type_error("'RerollType' object is not iterable")
        }
        PValue::Break(_) => return type_error("'Break' object is not iterable"),
        PValue::Func(_) => return Err(PErr::Unimplemented("iterating a function".into())),
        PValue::Again(_) => return type_error("'AgainExpression' object is not iterable"),
        PValue::Ellipsis => return type_error("'ellipsis' object is not iterable"),
        // A deck is a mapping: iterating it gives its outcomes.
        PValue::Deck(d) => d.outcomes().cloned().map(PValue::Value).collect(),
        PValue::Pool(_) | PValue::Deal(_) | PValue::MultiDeal(_) | PValue::Expr(_) => {
            return Err(PErr::Unimplemented("iterating a multiset generator".into()))
        }
        PValue::MultisetFunction(_) | PValue::RawResult(_) => {
            return Err(PErr::Unimplemented(
                "iterating a multiset function result".into(),
            ))
        }
    })
}

/// `expand_arg`'s view of one outcome: a die is mixed in, a hashable value is an outcome, `Reroll` drops out, a
/// tuple holding dice is `tupleize`d, a list is unhashable.
pub(super) fn die_arg(x: PValue) -> Result<DieArg<Value>, PErr> {
    match x {
        PValue::Value(v) => Ok(DieArg::Outcome(v)),
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(DieArg::Die(d)),
        PValue::DieWithTruthError(e, _) => Err(e.into()),
        // Any population is a mapping to `expand_arg`.
        PValue::Deck(d) => Ok(DieArg::Die(d.population().clone())),
        // `arg in icepool.REROLL_TYPES`.
        PValue::Reroll | PValue::Restart => Ok(DieArg::Reroll),
        // `map` unwraps a `Break`; nothing else returns one.
        PValue::Break(_) => Err(PErr::Unimplemented("a Break as an outcome".into())),
        PValue::Tuple(items) => map::tupleize(items),
        PValue::List(_) => type_error("unhashable type: 'list'"),
        PValue::Func(_) => Err(PErr::Unimplemented("a function as an outcome".into())),
        // `Die(...)` replaces top-level expressions first (`die_with_agains`); one nested in a tuple is not handled.
        PValue::Again(_) => Err(PErr::Unimplemented(
            "an Again expression nested in an outcome".into(),
        )),
        PValue::Ellipsis
        | PValue::Pool(_)
        | PValue::Deal(_)
        | PValue::MultiDeal(_)
        | PValue::Expr(_)
        | PValue::MultisetFunction(_)
        | PValue::RawResult(_) => Err(PErr::Unimplemented(
            "Ellipsis, a pool or a multiset function as an outcome".into(),
        )),
    }
}

/// `icepool.Die(outcomes)` (nodes `die`, `die_typed`) and `icepool.Die(outcomes, times)` (node `die_times`).
fn die(outcomes: PValue, times: Option<PValue>) -> Result<PValue, PErr> {
    die_with_agains(outcomes, times, None, None, PValue::Value(Value::None))
}

/// `icepool.Die(outcomes, times, again_count=, again_depth=, again_end=)`: outcomes that are `Again` expressions
/// are rolled again (population/again.py; `None` arguments are `None` here); without any, the again arguments are
/// ignored.
pub(super) fn die_with_agains(
    outcomes: PValue,
    times: Option<PValue>,
    again_count: Option<PValue>,
    again_depth: Option<PValue>,
    again_end: PValue,
) -> Result<PValue, PErr> {
    let (objects, times) = itemize(outcomes, times)?;
    if objects.iter().any(|x| matches!(x, PValue::Again(_))) {
        if let Some(count) = again_count {
            if again_depth.is_some() {
                return raise(Error::Value(
                    "At most one of again_count and again_depth may be used.".into(),
                ));
            }
            return again::evaluate_agains_using_count(objects, times, count, again_end);
        }
        // The default is depth mode with depth 1.
        let depth = again_depth.unwrap_or(PValue::Value(Value::from(1)));
        return again::evaluate_agains_using_depth(objects, times, depth, again_end);
    }
    new_die(objects, &times)
}

/// The rest of `Die.__new__` once the times are checked to be non-negative numbers and any `Again` is replaced.
pub(super) fn new_die(objects: Vec<PValue>, times: &[Value]) -> Result<PValue, PErr> {
    // A single die with weight 1 is returned as-is (a `DieWithTruth` keeps its type). Python tests
    // `times[0] == 1`, so `1.0` and `True` count too; they must be caught before `weight` would reject a non-int.
    if let ([d @ (PValue::Die(_) | PValue::DieWithTruth(..))], [t]) = (objects.as_slice(), times) {
        if t.as_num()
            .is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ONE)))
        {
            return Ok(d.clone());
        }
    }
    let args: Vec<DieArg<Value>> = objects.into_iter().map(die_arg).collect::<Result<_, _>>()?;
    let quantities = args
        .iter()
        .zip(times)
        .map(|(a, t)| weight(a, t))
        .collect::<Result<_, _>>()?;
    Ok(PValue::Die(Die::new(args, quantities)?))
}

/// `creation_args.itemize(keys, times)`: a sequence of keys and one multiplier per key (a Python number, checked
/// non-negative, but not yet checked to be a valid quantity: see `weight`).
pub(super) fn itemize(
    keys: PValue,
    times: Option<PValue>,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    // A die is a Mapping: its outcomes become the keys and its quantities multiply the times. Each outcome then has
    // denominator 1, so zero quantities drop out. So is a deck.
    let keys = match keys {
        PValue::Deck(d) => PValue::Die(d.population().clone()),
        keys => keys,
    };
    let (args, base): (Vec<PValue>, Option<Vec<Value>>) = match keys {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => {
            let (a, q) = d
                .items()
                .iter()
                .map(|(o, q)| (PValue::Value(o.clone()), Value::Int(q.clone().into())))
                .unzip();
            (a, Some(q))
        }
        // Any other sequence: each element once. `len()` of an int, float, None... raises TypeError.
        x @ (PValue::List(_)
        | PValue::Tuple(_)
        | PValue::Value(Value::Str(_) | Value::Tuple(_))) => (py_iter(x)?, None),
        PValue::Func(_) => return Err(PErr::Unimplemented("Die(mapping)".into())),
        _ => return type_error("Argument appears not to be a mapping or sequence."),
    };
    itemize_times(args, base, times)
}

/// `itemize(mapping, times)` for a mapping given as its keys and values (the values multiply the times).
pub(super) fn itemize_mapping(
    keys: Vec<PValue>,
    values: Vec<Value>,
    times: PValue,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    itemize_times(keys, Some(values), Some(times))
}

/// The rest of `itemize` once the keys are known: `base` holds a mapping's values.
fn itemize_times(
    args: Vec<PValue>,
    base: Option<Vec<Value>>,
    times: Option<PValue>,
) -> Result<(Vec<PValue>, Vec<Value>), PErr> {
    let times: Vec<Value> = match times {
        None => vec![Value::from(1); args.len()],
        // `isinstance(times, int)`: one factor for every key.
        Some(PValue::Value(t @ (Value::Int(_) | Value::Bool(_)))) => vec![t; args.len()],
        // Otherwise any sequence of the same length (even a str or a die, whose outcomes are then the times).
        Some(t) => {
            let ts = py_iter(t)?;
            if ts.len() != args.len() {
                return raise(Error::Value(format!(
                    "The number of times ({}) must equal the number of keys ({}).",
                    ts.len(),
                    args.len()
                )));
            }
            ts.into_iter()
                .map(|t| match t {
                    PValue::Value(v) => Ok(v),
                    _ => type_error("'<' not supported between a quantity and 'int'"),
                })
                .collect::<Result<_, _>>()?
        }
    };
    let times = match base {
        None => times,
        Some(q) => q
            .iter()
            .zip(&times)
            .map(|(v, t)| match v {
                Value::Int(q) if *q >= IBig::ZERO => {
                    int_times(&dashu_base::UnsignedAbs::unsigned_abs(q.clone()), t)
                }
                // `v * x` for a mapping value that is not a quantity.
                v => v.binary(BinOp::Mul, t).map_err(PErr::Raise),
            })
            .collect::<Result<_, _>>()?,
    };
    // `any(x < 0 for x in times)`: raises for negatives, and for anything that is not a number.
    for t in &times {
        match t.as_num() {
            Some(n) if n < zero() => return negative_times(),
            Some(_) => {}
            None => {
                return type_error(format!(
                    "'<' not supported between instances of '{}' and 'int'",
                    t.type_name()
                ))
            }
        }
    }
    Ok((args, times))
}

/// Python's `q * t` for an int quantity `q`. Only numbers can make a valid quantity; anything else raises later
/// in Python too (at the latest at `x < 0`), so it raises here.
fn int_times(q: &Quantity, t: &Value) -> Result<Value, PErr> {
    let qi = IBig::from(q.clone());
    Ok(match t {
        Value::Int(i) => Value::Int(qi * i),
        Value::Bool(b) => Value::Int(qi * IBig::from(*b as u8)),
        Value::Fraction(r) => Value::Fraction(RBig::from(qi) * r),
        Value::Float(f) => {
            // float(q) rounds correctly and overflows rather than giving inf.
            let qf: f64 = q.to_string().parse().expect("an integer parses as f64");
            if qf.is_infinite() {
                return raise(Error::Overflow("int too large to convert to float".into()));
            }
            Value::Float(qf * f)
        }
        _ => {
            return type_error(format!(
                "can't multiply an int quantity by '{}'",
                t.type_name()
            ))
        }
    })
}

/// One non-negative number given as a quantity to `Die`, following what Python does with it in
/// `merge_weights_lcm`: an `int` (or `bool`) is the quantity; another number (`float`, `Fraction`) only survives if
/// it can never reach `math.gcd`: when it is zero, or when the argument is an empty die (denominator 0). Either way
/// it then contributes nothing, exactly like a 0.
fn weight(arg: &DieArg<Value>, t: &Value) -> Result<Quantity, PErr> {
    match t {
        Value::Int(i) => Ok(Quantity::try_from(i.clone()).expect("checked non-negative")),
        Value::Bool(b) => Ok(Quantity::from(*b as u8)),
        _ => {
            let empty_die = matches!(arg, DieArg::Die(d) if d.denominator().is_zero());
            if t.as_num().is_some_and(|n| n == zero()) || empty_die {
                Ok(Quantity::ZERO)
            } else {
                type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    t.type_name()
                ))
            }
        }
    }
}

/// A Python dict key: a value or a die (dice hash by their items).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Value(Value),
    Die(Die<Value>),
}

/// `{k: v for k, v in items}`: each item unpacks into exactly two; a repeated key (by ==, e.g. True and 1) keeps
/// its first spelling and its last value. Returned in insertion order.
pub(super) fn dict_items(items: PValue) -> Result<Vec<(Key, PValue)>, PErr> {
    let mut entries: Vec<(Key, PValue)> = Vec::new();
    let mut index: FxHashMap<Key, usize> = FxHashMap::default();
    for item in py_iter(items)? {
        let Ok([k, v]) = <[PValue; 2]>::try_from(py_iter(item)?) else {
            return raise(Error::Value(
                "not enough or too many values to unpack (expected 2)".into(),
            ));
        };
        let k = match k {
            PValue::Value(v) => Key::Value(v),
            PValue::Die(d) => Key::Die(d),
            PValue::List(_) => return type_error("unhashable type: 'list'"),
            _ => return Err(PErr::Unimplemented("this dict key".into())),
        };
        match index.get(&k) {
            Some(&i) => entries[i].1 = v,
            None => {
                index.insert(k.clone(), entries.len());
                entries.push((k, v));
            }
        }
    }
    Ok(entries)
}

/// `icepool.Die({outcome: quantity for outcome, quantity in items})`.
fn weights(items: PValue) -> Result<PValue, PErr> {
    let (keys, values): (Vec<Key>, Vec<PValue>) = dict_items(items)?.into_iter().unzip();
    let args: Vec<PValue> = keys
        .into_iter()
        .map(|k| match k {
            Key::Value(v) => PValue::Value(v),
            Key::Die(d) => PValue::Die(d),
        })
        .collect();
    weights_die(args, values)
}

/// `icepool.Die(mapping)` for a dict's distinct keys and its values.
pub(super) fn weights_die(args: Vec<PValue>, values: Vec<PValue>) -> Result<PValue, PErr> {
    // Die(mapping): itemize multiplies each quantity by 1, which leaves any number as it is, then checks `x < 0`.
    let mut times = Vec::with_capacity(values.len());
    for v in values {
        match v {
            PValue::Value(t) if t.as_num().is_some_and(|n| n < zero()) => return negative_times(),
            PValue::Value(t) if t.as_num().is_some() => times.push(t),
            _ => return type_error("'<' not supported between a quantity and 'int'"),
        }
    }
    new_die(args, &times)
}
