//! Pools and keeping: construction, keep/index/slice, highest/lowest/middle, pool operators
//! (harness/grammar/pool.py).

use crate::pool;
use crate::{Args, PValue};
use icepool_core::Value;

use super::Dispatch;

/// The pool nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "Ellipsis" => args.with([], |[]| Ok(PValue::Ellipsis)),
        "pool" => args.with(["dice", "times"], |[dice, times]| pool::pool(dice, times)),
        "pool_dict" => args.with(["items", "times"], |[items, times]| {
            pool::pool_dict(items, times)
        }),
        "d_pool" | "z_pool" => args.with(["die_sizes", "mapping"], |[die_sizes, mapping]| {
            let mapping = matches!(mapping, PValue::Value(Value::Bool(true)));
            pool::standard_pool(op == "z_pool", die_sizes, mapping)
        }),
        "die_pool" => args.with(["die", "rolls"], |[die, rolls]| pool::die_pool(die, rolls)),
        "pool_keep" | "pool_index" => {
            args.with(["pool", "index"], |[p, index]| pool::pool_keep(p, index))
        }
        "pool_slice" => args.with(
            ["pool", "start", "stop", "step"],
            |[p, start, stop, step]| pool::pool_slice(p, start, stop, step),
        ),
        "pool_highest" | "pool_lowest" => args.with(["pool", "keep", "drop"], |[p, keep, drop]| {
            pool::pool_highest_lowest(op == "pool_highest", p, keep, drop)
        }),
        "pool_middle" => args.with(["pool", "keep", "tie"], |[p, keep, tie]| {
            pool::pool_middle(p, keep, tie)
        }),
        "die_middle" => args.with(["die", "rolls", "keep", "tie"], |[die, rolls, keep, tie]| {
            pool::die_middle(die, rolls, keep, tie)
        }),
        "die_keep" => args.with(["die", "rolls", "index"], |[die, rolls, index]| {
            pool::die_keep(die, rolls, index)
        }),
        "die_keep_slice" => args.with(
            ["die", "rolls", "start", "stop", "step"],
            |[die, rolls, start, stop, step]| pool::die_keep_slice(die, rolls, start, stop, step),
        ),
        "middle" => args.with(
            ["args", "keep", "tie", "default"],
            |[a, keep, tie, default]| pool::middle(a, keep, tie, default),
        ),
        "pool_mul" => args.with(["pool", "n"], |[p, n]| pool::pool_mul(p, n)),
        "pool_neg" => args.with(["pool"], |[p]| pool::pool_neg(p)),
        "pool_add" => args.with(["a", "b"], |[a, b]| pool::pool_add(a, b)),
        "pool_info" => args.with(["pool"], |[p]| pool::pool_info(p)),
        "pool_pop" => args.with(["pool", "order", "outcome"], |[p, order, outcome]| {
            pool::pool_pop(p, order, outcome)
        }),
        "die_highest" | "die_lowest" => args.with(
            ["die", "rolls", "keep", "drop"],
            |[die, rolls, keep, drop]| {
                pool::die_highest_lowest(op == "die_highest", die, rolls, keep, drop)
            },
        ),
        "highest" | "lowest" => args.with(
            ["args", "keep", "drop", "default"],
            |[a, keep, drop, default]| {
                pool::highest_lowest(op == "highest", a, keep, drop, default)
            },
        ),
        _ => return Err(args),
    })
}
