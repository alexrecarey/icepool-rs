//! The free functions of function.py and wallenius.py (harness/grammar/function.py).

use crate::function;
use crate::Args;

use super::Dispatch;

/// The function nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "z" => args.with(["sides"], |[sides]| function::z(sides)),
        "dz_attr" => args.with(["name"], |[name]| function::dz_attr(name)),
        "coin" => args.with(["n", "d", "max_denominator"], |[n, d, m]| {
            function::coin(n, d, m)
        }),
        "stochastic_round" => args.with(["x", "max_denominator"], |[x, m]| {
            function::stochastic_round(x, m)
        }),
        "one_hot" => args.with(["sides"], |[sides]| function::one_hot(sides)),
        "pointwise" => args.with(["args", "max", "spread"], |[a, max, spread]| {
            function::pointwise(a, max, spread)
        }),
        "extreme_outcome" => args.with(["args", "max", "spread"], |[a, max, spread]| {
            function::extreme_outcome(a, max, spread)
        }),
        "harmonize_denominators" => args.with(["dice", "weights"], |[dice, weights]| {
            function::harmonize_denominators(dice, weights)
        }),
        "consecutive" => args.with(["args"], |[a]| function::consecutive(a)),
        "sorted_union" => args.with(["args"], |[a]| function::sorted_union(a)),
        "reduce" | "accumulate" => args.with(["f", "dice", "initial"], |[f, dice, initial]| {
            if op == "reduce" {
                function::reduce(f, dice, initial)
            } else {
                function::accumulate(f, dice, initial)
            }
        }),
        "from_rv" => args.with(
            ["rv", "params", "outcomes", "denominator"],
            |[rv, params, outcomes, den]| function::from_rv(rv, params, outcomes, den),
        ),
        "die_stochastic_round" => args.with(["die", "max_denominator"], |[die, m]| {
            function::die_stochastic_round(die, m)
        }),
        "wallenius_deal_expr" => args.with(
            ["data", "mapping", "hand_size"],
            |[data, mapping, hand_size]| function::wallenius_deal_expr(data, mapping, hand_size),
        ),
        "wallenius_deal" => args.with(
            ["data", "mapping", "hand_size", "method"],
            |[data, mapping, hand_size, method]| {
                function::wallenius_deal(data, mapping, hand_size, method)
            },
        ),
        _ => return Err(args),
    })
}
