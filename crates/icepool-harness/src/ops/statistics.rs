//! Quantities, probabilities, statistics, marginals and denominators (harness/grammar/statistics.py,
//! die_extra.py `standardized_moment`).

use crate::statistics;
use crate::Args;

use super::Dispatch;

/// The statistics nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "quantities" => args.with(["die", "comparison"], |[die, comparison]| {
            statistics::quantities(die, comparison)
        }),
        "probabilities" => args.with(
            ["die", "comparison", "percent"],
            |[die, comparison, percent]| statistics::probabilities(die, comparison, percent),
        ),
        "quantity" => args.with(["die", "args"], |[die, a]| statistics::quantity(die, a)),
        "probability" => args.with(["die", "args", "percent"], |[die, a, percent]| {
            statistics::probability(die, a, percent)
        }),
        "nearest" => args.with(
            ["die", "comparison", "outcome"],
            |[die, comparison, outcome]| statistics::nearest(die, comparison, outcome),
        ),
        "die_stat" => args.with(["die", "stat"], |[die, stat]| {
            statistics::die_stat(die, stat)
        }),
        "standardized_moment" => args.with(["die", "k"], |[die, k]| {
            statistics::standardized_moment(die, k)
        }),
        "entropy" => args.with(["die", "base"], |[die, base]| {
            statistics::entropy(die, base)
        }),
        "die_quantile" => args.with(["die", "method", "args"], |[die, method, a]| {
            statistics::quantile(die, method, a)
        }),
        "cdf_distance" => args.with(["a", "b", "stat"], |[a, b, stat]| {
            statistics::cdf_distance(a, b, stat)
        }),
        "marginal" => args.with(["die", "index"], |[die, index]| {
            statistics::marginal(die, index)
        }),
        "marginal_slice" => args.with(
            ["die", "start", "stop", "step"],
            |[die, start, stop, step]| statistics::marginal_slice(die, start, stop, step),
        ),
        "marginals" => args.with(["die"], |[die]| statistics::marginals(die)),
        "group_by" => args.with(["die", "key", "star"], |[die, key, star]| {
            statistics::group_by(die, key, star)
        }),
        "group_by_index" => args.with(["die", "index"], |[die, index]| {
            statistics::group_by_index(die, index)
        }),
        "marginal_attr" => args.with(["die", "name"], |[die, name]| {
            statistics::marginal_attr(die, name)
        }),
        "group_by_attr" => args.with(["die", "name"], |[die, name]| {
            statistics::group_by_attr(die, name)
        }),
        "joint_stat" => args.with(["die", "stat", "i", "j"], |[die, stat, i, j]| {
            statistics::joint_stat(die, stat, i, j)
        }),
        "quantities_op" => args.with(["die", "method", "x"], |[die, method, x]| {
            statistics::quantities_op(die, method, x)
        }),
        "pad_to_denominator" => args.with(
            ["die", "denominator", "outcome"],
            |[die, denominator, outcome]| statistics::pad_to_denominator(die, denominator, outcome),
        ),
        "multiply_to_denominator" => args.with(["die", "denominator"], |[die, denominator]| {
            statistics::multiply_to_denominator(die, denominator)
        }),
        "quantity_where" => args.with(["die", "which", "star"], |[die, which, star]| {
            statistics::quantity_where(die, which, star)
        }),
        "probability_where" => args.with(
            ["die", "which", "star", "percent"],
            |[die, which, star, percent]| statistics::probability_where(die, which, star, percent),
        ),
        _ => return Err(args),
    })
}
