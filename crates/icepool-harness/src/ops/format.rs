//! Formatting, sampling, `==` and truth (harness/grammar/format.py).

use crate::format;
use crate::Args;

use super::Dispatch;

/// The format nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "format" => args.with(["die", "spec"], |[die, spec]| format::format(die, spec)),
        "format_probability_inverse" => args.with(["probability", "args"], |[probability, a]| {
            format::format_probability_inverse(probability, a)
        }),
        "common_outcome_length" => args.with(["die"], |[die]| format::common_outcome_length(die)),
        "sample" => args.with(["die", "seed", "n"], |[die, seed, n]| {
            format::sample(die, seed, n)
        }),
        "eq" | "ne" => args.with(["a", "b"], |[a, b]| format::eq(a, b, op == "ne")),
        // `again_truth` is `bool(expr)` for an `Again` expression (its own node only for its generator).
        "truth" | "again_truth" => args.with(["x"], |[x]| format::truth(x)),
        "equals" => args.with(["a", "b", "simplify"], |[a, b, simplify]| {
            format::equals(a, b, simplify)
        }),
        "equals_self" => args.with(["x"], |[x]| format::equals_self(x)),
        "hash_key_compare" => args.with(["a", "b", "cmp"], |[a, b, cmp]| {
            format::hash_key_compare(a, b, cmp)
        }),
        _ => return Err(args),
    })
}
