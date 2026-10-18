//! The grammar nodes (harness/grammar/*.py), each unpacking its Python-style arguments and calling the core.
//! One file per domain, after the grammar files; [`call`] asks each file's table in turn.

mod collection;
mod deck;
mod dice;
mod die_methods;
mod evaluate;
mod format;
mod function;
mod maps;
mod multiset;
mod operators;
mod pool;
mod population;
mod statistics;

use crate::{raise, Args, PErr, PValue};
use dashu_int::IBig;
use icepool_core::value::Num;
use icepool_core::Error;
use icepool_core::Value;
use std::borrow::Cow;

pub(crate) use dice::{
    d, dict_items, die_arg, die_with_agains, itemize, itemize_mapping, new_die, py_iter,
    weights_die, Key,
};
pub(crate) use operators::{binary, binary_op, matmul, to_die, unary, unary_op};

/// What a domain's table gives back: the node's result, or the arguments if `op` is not one of its nodes.
type Dispatch<'a> = Result<Result<PValue, PErr>, Args<'a>>;

/// Every domain's table, in the order [`call`] asks them. Node names are unique, so the order only matters for
/// `operators`, whose fallback (any name of `operator`'s functions) comes last, as it did in the single match.
const TABLES: [for<'a> fn(&str, Args<'a>) -> Dispatch<'a>; 13] = [
    dice::call,
    maps::call,
    die_methods::call,
    pool::call,
    multiset::call,
    evaluate::call,
    deck::call,
    statistics::call,
    format::call,
    function::call,
    population::call,
    collection::call,
    operators::call,
];

/// Runs the node `op` on its evaluated arguments.
pub fn call(op: &str, mut args: Args) -> Result<PValue, PErr> {
    for table in TABLES {
        match table(op, args) {
            Ok(result) => return result,
            Err(unused) => args = unused,
        }
    }
    Err(PErr::Unimplemented(op.into()))
}

/// A node's `str` argument naming an operation (`{"str": ...}` in the program).
fn str_arg(x: &PValue) -> Result<String, PErr> {
    match x {
        PValue::Value(Value::Str(s)) => Ok(s.to_string()),
        _ => raise(Error::MissingKey(
            "expected a str naming an operation".into(),
        )),
    }
}

/// A node's flag argument (`{"bool": ...}`).
fn bool_arg(x: &PValue) -> Result<bool, PErr> {
    match x {
        PValue::Value(Value::Bool(b)) => Ok(*b),
        _ => raise(Error::WrongType("expected a bool flag".into())),
    }
}

pub(crate) fn wrong_type<T>(msg: impl Into<String>) -> Result<T, PErr> {
    raise(Error::WrongType(msg.into()))
}

/// Python's `AttributeError`: the object lacks the method or attribute a program calls.
pub(crate) fn missing_attribute<T>(msg: impl Into<String>) -> Result<T, PErr> {
    raise(Error::MissingAttribute(msg.into()))
}

/// A list used where Python hashes (a dict key, a set member, an outcome).
pub(crate) fn unhashable_list<T>() -> Result<T, PErr> {
    raise(Error::Unhashable { type_name: "list" })
}

fn negative_times<T>() -> Result<T, PErr> {
    raise(Error::InvalidArgument(
        "times cannot have negative values.".into(),
    ))
}

fn zero() -> Num<'static> {
    Num::Int(Cow::Owned(IBig::ZERO))
}
