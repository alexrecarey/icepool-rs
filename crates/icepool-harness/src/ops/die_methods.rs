//! The die methods with nodes of their own (harness/grammar/die_methods.py, die_extra.py).

use crate::die_methods;
use crate::{Args, PValue};
use icepool_core::Value;

use super::Dispatch;

/// The die methods nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "zero" => args.with(["die"], |[die]| die_methods::zero(die)),
        "zero_outcome" => args.with(["die"], |[die]| die_methods::zero_outcome(die)),
        "sign" => args.with(["die"], |[die]| die_methods::sign(die)),
        "die_cmp" => args.with(["die", "other"], |[die, other]| {
            die_methods::cmp(die, other)
        }),
        "is_in" => args.with(["die", "outcomes"], |[die, outcomes]| {
            die_methods::is_in(die, outcomes)
        }),
        "die_count" => args.with(["die", "rolls", "outcomes"], |[die, rolls, outcomes]| {
            die_methods::count(die, rolls, outcomes)
        }),
        "if_else" => args.with(
            ["die", "if_true", "if_false", "count", "depth", "end"],
            |[die, if_true, if_false, count, depth, end]| {
                // `None` is the default for both again arguments.
                let some = |x: PValue| match x {
                    PValue::Value(Value::None) => None,
                    x => Some(x),
                };
                die_methods::if_else(die, if_true, if_false, some(count), some(depth), end)
            },
        ),
        "die_repr" => args.with(["die"], |[die]| die_methods::repr(die)),
        "simplify" => args.with(["die"], |[die]| die_methods::simplify(die)),
        "die_pop" => args.with(["die", "max"], |[die, max]| die_methods::pop(die, max)),
        "from_cumulative" => args.with(
            ["outcomes", "cumulative", "reverse"],
            |[outcomes, cumulative, reverse]| {
                die_methods::from_cumulative(outcomes, cumulative, reverse)
            },
        ),
        "die_method" => args.with(["die", "name", "arg"], |[die, name, arg]| {
            die_methods::die_method(die, name, arg)
        }),
        _ => return Err(args),
    })
}
