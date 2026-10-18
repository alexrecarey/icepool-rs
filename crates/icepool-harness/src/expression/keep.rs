//! Keeping and ordering: `MultisetKeep`, `sort_pair`, `max_pair_*`, `versus_*` and `force_order`.

use super::operators::{binary, int_arg};
use super::{as_tuple_expression, into_pvalue, the_expression, KeepIndexArg, MExpr, OpNode};
use crate::evaluate::implicit_convert;
use crate::ops::{py_iter, wrong_type};
use crate::{raise, PErr, PValue};
use icepool_core::evaluator::lexi::{compute_lexi_tuple, compute_lexi_tuple_with_zero_right_first};
use icepool_core::expression::{BinaryOperator, KeepCountsComparison, MultisetKeepIndex, Operator};
use icepool_core::Error;
use icepool_core::{Int, Value};
use std::sync::Arc;

/// `MultisetKeep(expr, index=index)` for a slice or a sequence of counts and `...` (`list_index`: given as a
/// list, which makes the expression unhashable).
pub fn multiset_keep(
    e: MExpr,
    index: MultisetKeepIndex,
    list_index: bool,
    given: KeepIndexArg,
) -> MExpr {
    MExpr::Op(Arc::new(OpNode {
        operator: Operator::Keep(index),
        children: vec![e],
        list_index,
        keep_index: Some(given),
    }))
}

/// A sequence index for `MultisetKeep`: its elements are only looked at when evaluated; `int`s and `...` here.
pub fn multiset_keep_seq(index: PValue) -> Result<MultisetKeepIndex, PErr> {
    // `_initial_state` only takes a `Sequence` (a list or tuple here; the port leaves out a str, whose characters
    // are never `...`, and a `Vector`); anything else is a TypeError there, when evaluated.
    match &index {
        PValue::List(_) | PValue::Tuple(_) | PValue::Value(Value::Tuple(_)) => {}
        PValue::Value(Value::Str(_) | Value::Vector(_) | Value::Host(_)) => {
            return Err(PErr::Unimplemented("this MultisetKeep index".into()))
        }
        PValue::Value(v) => return Ok(MultisetKeepIndex::Invalid(v.type_name().into())),
        PValue::Die(_) => return Ok(MultisetKeepIndex::Invalid("Die".into())),
        PValue::DieWithTruth(..) => return Ok(MultisetKeepIndex::Invalid("DieWithTruth".into())),
        _ => return Err(PErr::Unimplemented("this MultisetKeep index".into())),
    }
    let items = py_iter(index)?
        .into_iter()
        .map(|x| match x {
            PValue::Ellipsis => Ok(None),
            x => match int_arg(&x) {
                Some(n) => Ok(Some(n)),
                None => Err(PErr::Unimplemented(
                    "a MultisetKeep index element that is not an int".into(),
                )),
            },
        })
        .collect::<Result<_, _>>()?;
    Ok(MultisetKeepIndex::Seq(items))
}

/// A `str` argument of a pairing method; any other value can never match (ValueError when evaluated).
fn name(x: &PValue) -> Arc<str> {
    match x {
        PValue::Value(Value::Str(s)) => s.clone(),
        _ => Arc::from("\0not a str"),
    }
}

/// An `Order` argument (`Order(order)` on the Python side): -1, 0 or 1.
fn order_arg(x: &PValue) -> Result<i64, PErr> {
    match int_arg(x) {
        Some(o) if (Int::from(-1)..=Int::ONE).contains(&o) => Ok(o.to_i64().expect("-1, 0 or 1")),
        _ => Err(PErr::Unimplemented("an order that is not an Order".into())),
    }
}

/// `expr.sort_pair(comparison, other, order, extra)`, `sort_pair_keep_while` and `sort_pair_drop_while`.
/// Nothing is checked until evaluated.
pub fn sort_pair(
    e: PValue,
    method: &str,
    comparison: PValue,
    other: PValue,
    order: PValue,
    extra: PValue,
) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let other = implicit_convert(other)?;
    let mut sort_order = order_arg(&order)?;
    if sort_order == 0 {
        sort_order = -1;
    }
    let (comparison, extra) = (name(&comparison), name(&extra));
    let operator = match method {
        "sort_pair" => Operator::SortPair {
            comparison,
            sort_order,
            extra,
        },
        "sort_pair_keep_while" | "sort_pair_drop_while" => Operator::SortPairWhile {
            keep: method == "sort_pair_keep_while",
            comparison,
            sort_order,
            extra,
        },
        _ => return Err(PErr::Unimplemented(format!("pairing method {method}"))),
    };
    Ok(into_pvalue(MExpr::op(operator, vec![e, other])))
}

/// `expr.max_pair_keep(comparison, other, priority)` / `max_pair_drop`.
pub fn max_pair(
    e: PValue,
    keep: bool,
    comparison: PValue,
    other: PValue,
    priority: PValue,
) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let tuple_other = as_tuple_expression(&other).is_some();
    let other = implicit_convert(other)?;
    let comparison = name(&comparison);
    if &*comparison == "==" {
        if keep && tuple_other {
            // `+other`: a multiset tuple expression has no unary `+`.
            return wrong_type("bad operand type for unary +");
        }
        let result = if keep {
            // `+self & +other`
            let pos = |x| {
                MExpr::op(
                    Operator::KeepCounts(KeepCountsComparison::Ge, Int::ZERO),
                    vec![x],
                )
            };
            binary(BinaryOperator::Intersection, vec![pos(e), pos(other)])
        } else {
            // `self - other`
            binary(BinaryOperator::DifferenceDropNegative, vec![e, other])
        };
        return Ok(into_pvalue(result));
    }
    let (order, early) = match &priority {
        PValue::Value(Value::None) => {
            let (left_first, _, _) = compute_lexi_tuple(&comparison, 1)?;
            (if left_first != 0 { -1 } else { 1 }, false)
        }
        PValue::Value(Value::Str(p)) if &**p == "low" || &**p == "high" => {
            let order = if &**p == "low" { 1 } else { -1 };
            let (left_first, _, _) = compute_lexi_tuple(&comparison, order)?;
            (order, left_first != 0)
        }
        _ => {
            return raise(Error::InvalidArgument(
                "priority must be 'low' or 'high'.".into(),
            ))
        }
    };
    let (_, tie, _) = compute_lexi_tuple(&comparison, order)?;
    let pair_equal = tie != 0;
    let operator = if early {
        Operator::MaxPairEarly {
            order,
            pair_equal,
            keep,
        }
    } else {
        Operator::MaxPairLate {
            order,
            pair_equal,
            keep,
        }
    };
    Ok(into_pvalue(MExpr::op(operator, vec![e, other])))
}

/// `expr.versus_all(comparison, other)` / `versus_any`.
pub fn versus(e: PValue, all: bool, comparison: PValue, other: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let other = implicit_convert(other)?;
    let (mut lexi_tuple, mut order) = compute_lexi_tuple_with_zero_right_first(&name(&comparison))?;
    if !all {
        lexi_tuple = (lexi_tuple.2, lexi_tuple.1, lexi_tuple.0);
        order = -order;
    }
    Ok(into_pvalue(MExpr::op(
        Operator::Versus { lexi_tuple, order },
        vec![e, other],
    )))
}

/// `expr.force_order(order)`: `Order.Any` returns the expression itself.
pub fn force_order(e: PValue, order: PValue) -> Result<PValue, PErr> {
    let e = the_expression(e)?;
    let order = order_arg(&order)?;
    if order == 0 {
        return Ok(into_pvalue(e));
    }
    Ok(into_pvalue(MExpr::op(Operator::ForceOrder(order), vec![e])))
}
