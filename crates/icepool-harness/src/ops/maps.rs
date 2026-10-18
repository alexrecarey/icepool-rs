//! The `map` family: map, reroll/filter, split, clip, Markov chains and the to-pool maps
//! (harness/grammar/maps.py, markov.py, to_pool.py).

use super::dice::py_iter;
use crate::catalogue::Func;
use crate::map;
use crate::map_tools;
use crate::to_pool;
use crate::{Args, PValue};

use super::Dispatch;

/// The maps nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "fn" => args.with(["name", "params"], |[name, params]| {
            Ok(PValue::Func(Func::new(name, py_iter(params)?)?.into()))
        }),
        "Reroll" => args.with([], |[]| Ok(PValue::Reroll)),
        "tuple" => args.with(["items"], |[items]| Ok(PValue::tuple(py_iter(items)?))),
        "map" => args.with(["f", "args", "star"], |[f, args, star]| {
            map::map(f, args, star, vec![])
        }),
        "map_function" => args.with(
            ["f", "args", "star", "kwargs", "decorator"],
            |[f, args_, star, kwargs, decorator]| {
                map::map_function(f, args_, star, kwargs, decorator)
            },
        ),
        "map_kw" => args.with(["f", "args", "star", "kwargs"], |[f, args, star, kwargs]| {
            map::map(f, args, star, map::kwargs(kwargs)?)
        }),
        "map_repeat" => args.with(["f", "args", "star", "repeat"], |[f, args, star, repeat]| {
            map_tools::map_repeat(f, args, star, repeat)
        }),
        "map_and_time" => args.with(
            ["f", "initial", "args", "star", "repeat"],
            |[f, initial, args, star, repeat]| {
                map_tools::map_and_time(f, initial, args, star, repeat)
            },
        ),
        "mean_time_to_absorb" => args.with(
            ["f", "initial", "args", "star"],
            |[f, initial, args, star]| map_tools::mean_time_to_absorb(f, initial, args, star),
        ),
        "die_mean_time_to_absorb" => args
            .with(["die", "f", "args", "star"], |[die, f, args, star]| {
                map_tools::die_mean_time_to_absorb(die, f, args, star)
            }),
        "time_to_sum" => args.with(
            ["die", "target", "max_time", "dnf"],
            |[die, target, max_time, dnf]| map_tools::time_to_sum(die, target, max_time, dnf),
        ),
        "reroll" | "filter" => args.with(
            ["die", "which", "star", "depth"],
            |[die, which, star, depth]| map::reroll(op == "filter", die, which, star, depth),
        ),
        "map_to_pool" => args.with(["f", "args", "star"], |[f, args, star]| {
            to_pool::map_to_pool(f, args, star)
        }),
        "die_map_to_pool" => args.with(["die", "f", "args", "star"], |[die, f, args, star]| {
            to_pool::die_map_to_pool(die, f, args, star)
        }),
        "explode_to_pool" => args.with(
            ["die", "rolls", "outcomes", "star", "depth"],
            |[die, rolls, outcomes, star, depth]| {
                to_pool::explode_to_pool(die, rolls, outcomes, star, depth)
            },
        ),
        "reroll_to_pool" => args.with(
            [
                "die",
                "rolls",
                "outcomes",
                "max_rerolls",
                "star",
                "depth",
                "mode",
            ],
            |[die, rolls, outcomes, max_rerolls, star, depth, mode]| {
                to_pool::reroll_to_pool(die, rolls, outcomes, max_rerolls, star, depth, mode)
            },
        ),
        "split" => args.with(["die", "which", "star"], |[die, which, star]| {
            map::split(die, which, star)
        }),
        "clip" | "truncate" => args.with(["die", "min", "max"], |[die, min, max]| {
            map::clip_or_truncate(op == "clip", die, min, max)
        }),
        "break_str" => args.with(["outcome"], |[outcome]| map_tools::break_str(outcome)),
        _ => return Err(args),
    })
}
