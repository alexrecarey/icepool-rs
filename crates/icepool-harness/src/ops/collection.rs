//! Vectors, symbols, tuples as outcomes and complex numbers (harness/grammar/collection.py,
//! complex_num.py).

use crate::collection;
use crate::format;
use crate::Args;

use super::Dispatch;

/// The collection nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "vectorize" | "tupleize" => args.with(["args"], |[a]| {
            collection::cartesian_product(a, op == "tupleize")
        }),
        "vector" => args.with(["items"], |[items]| collection::vector(items)),
        "symbols" => args.with(["x"], |[x]| collection::symbols(x)),
        "complex" => args.with(["re", "im"], |[re, im]| collection::complex(re, im)),
        // Operators on complex values: the same generic `operator.<name>` as for collections (complex_num.py).
        "cx_binary" => args.with(["a", "b", "name"], |[a, b, name]| {
            collection::binary(a, b, name)
        }),
        "cx_unary" => args.with(["a", "name", "ndigits"], |[a, name, ndigits]| {
            collection::unary(a, name, ndigits)
        }),
        "coll_binary" => args.with(["a", "b", "name"], |[a, b, name]| {
            collection::binary(a, b, name)
        }),
        "coll_unary" => args.with(["a", "name", "ndigits"], |[a, name, ndigits]| {
            collection::unary(a, name, ndigits)
        }),
        "coll_truth" => args.with(["x"], |[x]| format::truth(x)),
        "vector_method" => args.with(["v", "name", "arg"], |[v, name, arg]| {
            collection::vector_method(v, name, arg)
        }),
        "symbols_method" => args.with(["s", "name", "arg"], |[s, name, arg]| {
            collection::symbols_method(s, name, arg)
        }),
        _ => return Err(args),
    })
}
