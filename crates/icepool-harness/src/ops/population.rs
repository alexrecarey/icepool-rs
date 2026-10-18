//! The population methods of population/base.py and the other population helpers
//! (harness/grammar/population.py).

use crate::collection;
use crate::population;
use crate::{Args, PErr, PValue};
use icepool_core::Value;

use super::Dispatch;

/// The population nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "die_sequence" => args.with(["die", "rolls"], |[die, rolls]| {
            collection::die_sequence(die, rolls)
        }),
        "cartesian_product" => args.with(["args", "vector"], |[a, vector]| match vector {
            PValue::Value(Value::Bool(vector)) => collection::cartesian_product(a, !vector),
            _ => Err(PErr::Unimplemented("cartesian_product outcome type".into())),
        }),
        "iter_cartesian_product" => {
            args.with(["args"], |[a]| collection::iter_cartesian_product(a))
        }
        "pop_append" => args.with(
            ["population", "outcome", "quantity"],
            |[p, outcome, quantity]| population::append(p, outcome, quantity),
        ),
        "pop_remove" => args.with(
            ["population", "outcome", "quantity"],
            |[p, outcome, quantity]| population::remove(p, outcome, quantity),
        ),
        "to_one_hot" => args.with(["population", "outcomes"], |[p, outcomes]| {
            population::to_one_hot(p, outcomes)
        }),
        "deck_zero" => args.with(["deck"], |[d]| population::deck_zero(d)),
        "deck_split" => args.with(["deck", "which", "star"], |[d, which, star]| {
            population::deck_split(d, which, star)
        }),
        "mean_time_to_sum" => args.with(["die", "targets"], |[die, targets]| {
            population::mean_time_to_sum(die, targets)
        }),
        _ => return Err(args),
    })
}
